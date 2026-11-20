use super::{origin::Origin, session};
use crate::{
    compositor::Compositor,
    search::{Target as SearchTarget, catalogue::FileIdentity},
};
use enzui_shell::i18n::tr;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Clone)]
pub enum Target {
    Desktop {
        path: std::path::PathBuf,
        identity: FileIdentity,
    },
    Prepared {
        id: String,
        identity: FileIdentity,
    },
}

/// Captures the sender's desktop entry or exact prepared executable; no match leaves focus-only activation.
pub fn target(root: &Path, origin: &Origin) -> Result<Option<Target>, String> {
    if let Some(path) = &origin.desktop {
        return Ok(Some(Target::Desktop {
            path: path.clone(),
            identity: FileIdentity::capture(path)?,
        }));
    }
    for app in crate::catalog::APPS
        .iter()
        .filter(|app| crate::catalog::desktop_id(app.id).is_some())
    {
        let executable = crate::catalog::executable(root, app.id);
        if let Ok(identity) = FileIdentity::capture(&executable)
            && (identity.device, identity.inode) == (origin.device, origin.inode)
        {
            return Ok(Some(Target::Prepared {
                id: app.id.into(),
                identity,
            }));
        }
    }
    Ok(None)
}

fn admitted(live: &AtomicBool, authority: &AtomicBool) -> Result<(), String> {
    if live.load(Ordering::Acquire) && authority.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err(tr("shell-notification-activation-was-cancelled").into())
    }
}

/// Focuses the verified sender or launches its reviewed app; cancellation prevents new dispatch, not undoing a sent request.
pub async fn activate(
    compositor: &Compositor,
    root: &Path,
    origin: Origin,
    target: Option<Target>,
    token: Option<String>,
    live: Arc<AtomicBool>,
    authority: Arc<AtomicBool>,
) -> Result<(), String> {
    admitted(&live, &authority)?;
    let session = session::Target::from_environment()?;
    let connection = session.connect(Arc::new(AtomicU64::new(0))).await?;
    admitted(&live, &authority)?;
    origin.verify(&connection, session.unit()).await?;
    let (pid, start) = origin.process();
    let windows = compositor.search_windows().await?;
    admitted(&live, &authority)?;
    if let Some(window) = windows
        .items
        .into_iter()
        .filter_map(|item| item.target)
        .find_map(|target| match target {
            SearchTarget::Window(window) if window.pid == pid && window.start == start => {
                Some(window)
            }
            _ => None,
        })
    {
        origin.verify(&connection, session.unit()).await?;
        admitted(&live, &authority)?;
        return compositor.focus_search_result(window).await;
    }
    let Some(target) = target else {
        return Ok(());
    };
    origin.verify(&connection, session.unit()).await?;
    admitted(&live, &authority)?;
    launch(root, target, token, &connection, &live, &authority).await
}

async fn launch(
    root: &Path,
    target: Target,
    token: Option<String>,
    connection: &zbus::Connection,
    live: &AtomicBool,
    authority: &AtomicBool,
) -> Result<(), String> {
    let (app, program, args) = match target {
        Target::Desktop { path, identity } => {
            identity.verify(&path)?;
            if !crate::parental::entry_allowed(&path) {
                return Err(tr("shell-this-app-is-not-allowed-on-this-account").into());
            }
            if let Some(name) = crate::search::catalogue::dbus_activatable(&path) {
                admitted(live, authority)?;
                return crate::search::compositor::activate_application_on(
                    connection,
                    &name,
                    token.as_deref(),
                )
                .await;
            }
            let app = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| tr("shell-invalid-application-identity"))?
                .to_owned();
            (
                app,
                std::path::PathBuf::from("/usr/bin/gio"),
                vec![
                    std::ffi::OsString::from("launch"),
                    "--".into(),
                    path.into_os_string(),
                ],
            )
        }
        Target::Prepared { id, identity } => {
            if crate::catalog::desktop_id(&id).is_none() || !root.is_absolute() {
                return Err(tr("shell-application-is-no-longer-installed").into());
            }
            let executable = crate::catalog::executable(root, &id);
            identity.verify(&executable)?;
            if !crate::parental::program_allowed(&executable) {
                return Err(tr("shell-this-app-is-not-allowed-on-this-account").into());
            }
            let app = crate::catalog::desktop_id(&id).unwrap_or(&id).to_owned();
            (
                app,
                root.join("scripts/launch-desktop-app"),
                vec![id.into()],
            )
        }
    };
    let prefix = crate::app_scope::prefix(&app);
    let mut command = match prefix.split_first() {
        Some((run, rest)) => {
            let mut command = tokio::process::Command::new(run);
            command.args(rest).arg(program);
            command
        }
        None => tokio::process::Command::new(program),
    };
    command.args(args);
    activation_environment(&mut command, token.as_deref());
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(false);
    admitted(live, authority)?;
    let mut child = command
        .spawn()
        .map_err(|_| tr("shell-application-could-not-be-started"))?;
    let mut exited = tokio::spawn(async move { child.wait().await });
    match tokio::time::timeout(std::time::Duration::from_millis(300), &mut exited).await {
        Ok(Ok(Ok(status))) if status.success() => Ok(()),
        Ok(_) => Err(tr("shell-application-exited-before-opening-check-its-installation").into()),
        Err(_) => Ok(()),
    }
}

fn activation_environment(command: &mut tokio::process::Command, token: Option<&str>) {
    command
        .env_remove("XDG_ACTIVATION_TOKEN")
        .env_remove("DESKTOP_STARTUP_ID");
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct App(std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>);
    #[zbus::interface(name = "org.freedesktop.Application")]
    impl App {
        #[allow(
            clippy::unused_async,
            reason = "This method implements an asynchronous D-Bus interface."
        )]
        async fn activate(
            &self,
            platform: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        ) {
            assert!(platform.keys().all(|key| key == "activation-token"));
            self.0.lock().unwrap().push(
                platform
                    .get("activation-token")
                    .and_then(|value| <&str>::try_from(value).ok())
                    .map(str::to_owned),
            );
        }
    }
    #[tokio::test]
    async fn activation_async_behaviour() {
        // Dbus launch uses the original connection and fresh token.
        {
            let bus = crate::test_bus::Bus::start();
            let received = Arc::new(std::sync::Mutex::new(Vec::new()));
            let _owner = bus
                .serve(
                    "dev.enzui.ActivationQA",
                    "/dev/enzui/ActivationQA",
                    App(std::sync::Arc::clone(&received)),
                )
                .await;
            let connection = bus.client().await;
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("dev.enzui.ActivationQA.desktop");
            std::fs::write(&path, "[Desktop Entry]\nType=Application\nName=QA\nExec=/usr/bin/true\nDBusActivatable=true\n").unwrap();
            let target = || Target::Desktop {
                path: path.clone(),
                identity: FileIdentity::capture(&path).unwrap(),
            };
            let live = AtomicBool::new(true);
            let authority = AtomicBool::new(true);
            launch(
                directory.path(),
                target(),
                Some("fresh synthetic".into()),
                &connection,
                &live,
                &authority,
            )
            .await
            .unwrap();
            launch(
                directory.path(),
                target(),
                None,
                &connection,
                &live,
                &authority,
            )
            .await
            .unwrap();
            assert_eq!(
                *received.lock().unwrap(),
                vec![Some("fresh synthetic".into()), None]
            );
            live.store(false, Ordering::Release);
            assert!(
                launch(
                    directory.path(),
                    target(),
                    Some("late synthetic".into()),
                    &connection,
                    &live,
                    &authority
                )
                .await
                .is_err()
            );
            assert_eq!(received.lock().unwrap().len(), 2);
        }
        // Reviewed desktop replacement and revoked service do not launch.
        {
            let bus = crate::test_bus::Bus::start();
            let received = Arc::new(std::sync::Mutex::new(Vec::new()));
            let _owner = bus
                .serve(
                    "dev.enzui.ActivationQA",
                    "/dev/enzui/ActivationQA",
                    App(std::sync::Arc::clone(&received)),
                )
                .await;
            let connection = bus.client().await;
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("dev.enzui.ActivationQA.desktop");
            let text = "[Desktop Entry]\nType=Application\nName=QA\nExec=/usr/bin/true\nDBusActivatable=true\n";
            std::fs::write(&path, text).unwrap();
            let identity = FileIdentity::capture(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            std::fs::write(&path, format!("{text}Comment=changed\n")).unwrap();
            assert!(
                launch(
                    directory.path(),
                    Target::Desktop {
                        path: path.clone(),
                        identity
                    },
                    None,
                    &connection,
                    &AtomicBool::new(true),
                    &AtomicBool::new(true)
                )
                .await
                .is_err()
            );
            let identity = FileIdentity::capture(&path).unwrap();
            assert!(
                launch(
                    directory.path(),
                    Target::Desktop { path, identity },
                    None,
                    &connection,
                    &AtomicBool::new(true),
                    &AtomicBool::new(false)
                )
                .await
                .is_err()
            );
            assert!(received.lock().unwrap().is_empty());
        }
    }
}
