use super::{Options, PortalDialogAnswer, PortalDialogKind};
use crate::desktop_portal::{BUS_NAME, Event, PATH, owned, serve_on};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;
use zbus::zvariant::{ObjectPath, Value};

struct DocumentsFixture {
    mount: PathBuf,
}

#[zbus::interface(name = "org.freedesktop.portal.Documents")]
impl DocumentsFixture {
    fn add_full(
        &self,
        fds: Vec<zbus::zvariant::OwnedFd>,
        flags: u32,
        app_id: &str,
        permissions: Vec<String>,
    ) -> (Vec<String>, Options) {
        use std::os::fd::AsRawFd;
        assert_eq!(fds.len(), 1);
        assert_eq!(flags, 0);
        assert_eq!(app_id, "org.example.Mail");
        assert_eq!(permissions, ["read"]);
        let attachment = std::fs::read_to_string(format!("/proc/self/fd/{}", fds[0].as_raw_fd()))
            .expect("attachment descriptor");
        assert_eq!(attachment, "Notes");
        let mut mount = self.mount.as_os_str().as_bytes().to_vec();
        mount.push(0);
        (
            vec!["a1".into()],
            Options::from([("mountpoint".into(), owned(Value::from(mount)))]),
        )
    }
}

#[tokio::test]
#[ignore = "run inside tools/test-session/run with gio installed"]
async fn backends_answer_on_a_private_bus() {
    tokio::time::timeout(Duration::from_secs(20), exercise_backends())
        .await
        .expect("portal requests settled");
}

async fn exercise_backends() {
    const CHOOSER: &str = "org.freedesktop.impl.portal.AppChooser";
    const LAUNCHER: &str = "org.freedesktop.impl.portal.DynamicLauncher";
    const ACCOUNT: &str = "org.freedesktop.impl.portal.Account";
    const EMAIL: &str = "org.freedesktop.impl.portal.Email";
    const REQUEST: &str = "org.freedesktop.impl.portal.Request";
    const PARENT: &str = "wayland:fixture";

    let root = PathBuf::from(std::env::var_os("TS_ROOT").expect("use tools/test-session/run"));
    let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").expect("private data home"));
    let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("private config home"));
    assert!(data.starts_with(&root) && config.starts_with(&root));
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).expect("fixture desktop entry directory");
    let mail_log = root.join("mail-uri");
    let mail_script = root.join("mail-handler");
    std::fs::write(
        &mail_script,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", mail_log.display()),
    )
    .expect("fixture mail handler");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&mail_script, std::fs::Permissions::from_mode(0o700))
        .expect("executable mail handler");
    for (id, name, mime, exec) in [
        (
            "org.example.Text",
            "Text Editor",
            "text/plain",
            "/usr/bin/true".to_owned(),
        ),
        (
            "org.example.Image",
            "Image Viewer",
            "image/png",
            "/usr/bin/true".to_owned(),
        ),
        (
            "org.example.Other",
            "Other Editor",
            "text/plain",
            "/usr/bin/true".to_owned(),
        ),
        (
            "org.example.Mail",
            "Mail Fixture",
            "x-scheme-handler/mailto",
            mail_script.display().to_string(),
        ),
    ] {
        std::fs::write(
            apps.join(format!("{id}.desktop")),
            format!(
                "[Desktop Entry]\nType=Application\nName={name}\nMimeType={mime};\nExec={exec} %u\n"
            ),
        )
        .expect("fixture desktop entry");
    }
    std::fs::write(
        config.join("mimeapps.list"),
        "[Default Applications]\nx-scheme-handler/mailto=org.example.Mail.desktop;\n",
    )
    .expect("fixture default app");

    let mail_desktop = apps.join("org.example.Mail.desktop");
    use std::io::Write as _;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(mail_desktop)
            .expect("mail desktop entry"),
        "X-Flatpak=org.example.Mail"
    )
    .expect("fixture sandbox metadata");
    let attachment = root.join("report & notes.txt");
    std::fs::write(&attachment, "Notes").expect("fixture attachment");
    let mount = root.join("doc");
    let _documents = zbus::connection::Builder::session()
        .expect("private documents bus")
        .serve_at(
            "/org/freedesktop/portal/documents",
            DocumentsFixture {
                mount: mount.clone(),
            },
        )
        .expect("serve fixture documents path")
        .name("org.freedesktop.portal.Documents")
        .expect("fixture documents name")
        .build()
        .await
        .expect("serve fixture documents");

    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let _portal = serve_on(
        zbus::connection::Builder::session().expect("private bus"),
        tx,
    )
    .await
    .expect("serve portal interfaces");
    let client = zbus::Connection::session().await.expect("portal caller");
    let other = zbus::Connection::session().await.expect("other caller");
    let handle = |path| ObjectPath::try_from(path).expect("request path");
    let proxy = |interface| zbus::Proxy::new(&client, BUS_NAME, PATH, interface);

    let app_request = "/org/freedesktop/portal/desktop/request/test/chooser";
    let chooser = proxy(CHOOSER).await.expect("app chooser proxy");
    let choose = {
        let chooser = chooser.clone();
        tokio::spawn(async move {
            chooser
                .call::<_, _, (u32, Options)>(
                    "ChooseApplication",
                    &(
                        handle(app_request),
                        "org.example.Test",
                        PARENT,
                        vec!["org.example.Image", "org.example.Text"],
                        Options::from([
                            (
                                "last_choice".into(),
                                owned(Value::from("org.example.Image")),
                            ),
                            ("content_type".into(), owned(Value::from("text/plain"))),
                        ]),
                    ),
                )
                .await
                .expect("ChooseApplication response")
        })
    };
    let Some(Event::Dialog(dialog, choose_reply)) = events.recv().await else {
        panic!("expected a chooser prompt");
    };
    let PortalDialogKind::OpenWith {
        choices,
        last_choice,
        ..
    } = dialog.kind
    else {
        panic!("expected application choices");
    };
    assert_eq!(dialog.parent, PARENT);
    assert_eq!(choices[0].name, "Text Editor");
    assert_eq!(choices[1].id, "org.example.Image");
    assert_eq!(last_choice.as_deref(), Some("org.example.Image"));
    let foreign_chooser = zbus::Proxy::new(&other, BUS_NAME, PATH, CHOOSER)
        .await
        .expect("other chooser proxy");
    let update = (handle(app_request), vec!["org.example.Other"]);
    assert!(
        foreign_chooser
            .call::<_, _, ()>("UpdateChoices", &update)
            .await
            .is_err()
    );
    chooser
        .call::<_, _, ()>("UpdateChoices", &update)
        .await
        .expect("update choices");
    let Some(Event::AppChoicesChanged(id, choices)) = events.recv().await else {
        panic!("expected updated application choices");
    };
    assert_eq!(id, dialog.id);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].name, "Other Editor");
    choose_reply
        .send(PortalDialogAnswer::Application {
            id: choices[0].id.clone(),
            activation_token: Some("fresh-token".into()),
        })
        .expect("choose updated application");
    let (response, results) = choose.await.expect("settled selection");
    assert_eq!(response, 0);
    assert_eq!(
        <&str>::try_from(&results["choice"]).expect("desktop ID"),
        "org.example.Other"
    );
    assert_eq!(
        <&str>::try_from(&results["activation_token"]).expect("activation token"),
        "fresh-token"
    );

    let launcher = proxy(LAUNCHER).await.expect("launcher proxy");
    assert_eq!(
        launcher
            .get_property::<u32>("SupportedLauncherTypes")
            .await
            .expect("launcher types"),
        3
    );
    assert_ne!(
        launcher
            .call::<_, _, u32>("RequestInstallToken", &("", Options::new()))
            .await
            .expect("token response"),
        0
    );
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><circle cx="32" cy="32" r="24"/></svg>"#;
    let icon = || owned(Value::from(("bytes", Value::from(svg.to_vec()))));
    let install = tokio::spawn(async move {
        launcher
            .call::<_, _, (u32, Options)>(
                "PrepareInstall",
                &(
                    handle("/org/freedesktop/portal/desktop/request/test/launcher"),
                    "org.example.Test",
                    PARENT,
                    "Original name",
                    icon(),
                    Options::from([
                        ("launcher_type".into(), owned(Value::from(2u32))),
                        ("target".into(), owned(Value::from("https://example.org"))),
                    ]),
                ),
            )
            .await
            .expect("PrepareInstall response")
    });
    let Some(Event::Dialog(dialog, install_reply)) = events.recv().await else {
        panic!("expected launcher consent");
    };
    assert_eq!(dialog.parent, PARENT);
    assert!(matches!(dialog.kind, PortalDialogKind::Install {
        editable: true, target: Some(target), ..
    } if target == "https://example.org"));
    install_reply
        .send(PortalDialogAnswer::Install("Edited name".into()))
        .expect("accept launcher");
    let (response, results) = install.await.expect("settled launcher");
    assert_eq!(response, 0);
    assert_eq!(
        <&str>::try_from(&results["name"]).expect("launcher name"),
        "Edited name"
    );
    assert_eq!(
        results["icon"],
        owned(Value::Value(Box::new(icon().into())))
    );

    let account_request = "/org/freedesktop/portal/desktop/request/test/account";
    let account = proxy(ACCOUNT).await.expect("account proxy");
    let account_call = tokio::spawn(async move {
        account
            .call::<_, _, (u32, Options)>(
                "GetUserInformation",
                &(
                    handle(account_request),
                    "org.example.Test",
                    PARENT,
                    Options::from([(
                        "reason".into(),
                        owned(Value::from("Show your name on your profile.")),
                    )]),
                ),
            )
            .await
            .expect("GetUserInformation response")
    });
    let Some(Event::Dialog(dialog, account_reply)) = events.recv().await else {
        panic!("expected an account consent prompt");
    };
    assert_eq!(dialog.parent, PARENT);
    assert!(
        matches!(dialog.kind, PortalDialogKind::ShareAccount { reason: Some(reason), .. }
        if reason == "Show your name on your profile.")
    );
    let foreign_request = zbus::Proxy::new(&other, BUS_NAME, account_request, REQUEST)
        .await
        .expect("other request proxy");
    assert!(
        foreign_request
            .call::<_, _, ()>("Close", &())
            .await
            .is_err()
    );
    let request = zbus::Proxy::new(&client, BUS_NAME, account_request, REQUEST)
        .await
        .expect("request proxy");
    request
        .call::<_, _, ()>("Close", &())
        .await
        .expect("cancel consent");
    assert!(matches!(events.recv().await, Some(Event::Close(id)) if id == dialog.id));
    assert_eq!(
        account_call.await.expect("settled account request"),
        (1, Options::new())
    );
    assert!(
        account_reply
            .send(PortalDialogAnswer::ShareAccount)
            .is_err()
    );

    let email = proxy(EMAIL).await.expect("email proxy");
    let response: (u32, Options) = email
        .call(
            "ComposeEmail",
            &(
                handle("/org/freedesktop/portal/desktop/request/test/email"),
                "org.example.Test",
                "",
                Options::from([
                    (
                        "addresses".into(),
                        owned(Value::from(vec!["to@example.org"])),
                    ),
                    (
                        "attachments".into(),
                        owned(Value::from(vec![
                            attachment.to_str().expect("fixture attachment path"),
                        ])),
                    ),
                ]),
            ),
        )
        .await
        .expect("ComposeEmail response");
    assert_eq!(response, (0, Options::new()));
    loop {
        match std::fs::read_to_string(&mail_log) {
            Ok(uri) if !uri.is_empty() => {
                let mount = mount
                    .to_str()
                    .expect("fixture mountpoint")
                    .replace('/', "%2F");
                assert_eq!(
                    uri,
                    format!(
                        "mailto:to@example.org?attachment=file%3A%2F%2F{mount}%2Fa1%2Freport%2520%26%2520notes.txt"
                    )
                );
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
}
