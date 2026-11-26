//! Local screenshot and screen recording completion notices.

use super::payload::{Action, Draft, Image, Urgency};
use crate::{capture::Saved, recording::Ending, search::catalogue::FileIdentity};
use enzui_shell::i18n::tr;
use std::{os::unix::fs::MetadataExt, path::PathBuf, process::Stdio, time::Duration};

/// A completion notice sent by the shell after a screen capture.
#[derive(Clone)]
pub struct CaptureNotice {
    summary: &'static str,
    body: String,
    icon: &'static str,
    transient: bool,
    image: Option<Image>,
    file: Option<SavedFile>,
}

impl std::fmt::Debug for CaptureNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CaptureNotification")
    }
}

impl CaptureNotice {
    /// Builds a notice with a bounded preview of the screenshot when requested.
    pub fn screenshot(saved: Saved, picture: &raster::DynamicImage, preview: bool) -> Self {
        let image = if preview {
            let pixels = picture.thumbnail(256, 256).into_rgba8();
            Image::from_rgba(pixels.width(), pixels.height(), pixels.into_raw())
                .ok()
                .map(|image| image.with_preview(true))
        } else {
            None
        };
        let retained = saved.path.is_some();
        let file = saved.path.zip(saved.identity).map(|(path, identity)| {
            SavedFile::new(path, identity, tr("shell-screenshot-could-not-be-opened"))
        });
        Self {
            summary: tr(if retained {
                "shell-screenshot-saved"
            } else {
                "shell-screenshot-copied"
            }),
            body: match (retained, saved.copied) {
                (true, true) => tr("shell-screenshot-also-on-your-clipboard"),
                (false, true) => tr("shell-screenshot-paste-it-with-ctrl-v"),
                _ => "",
            }
            .into(),
            icon: "camera",
            transient: !retained,
            image,
            file,
        }
    }

    /// Builds a notice for a finished recording. Failed recordings keep the error detail.
    pub fn recording(ending: Ending) -> Self {
        let (summary, body, file) = match ending {
            Ending::Saved(file) => ("shell-recording-saved", String::new(), Some(file)),
            Ending::Incomplete(file) => (
                "shell-recording-may-be-incomplete",
                String::new(),
                Some(file),
            ),
            Ending::Failed(error) => ("shell-recording-failed", error, None),
        };
        Self {
            summary: tr(summary),
            body,
            icon: "video",
            transient: file.is_none(),
            image: None,
            file,
        }
    }

    /// Text to use when the notification service is not ready.
    pub fn fallback(&self) -> &str {
        if self.body.is_empty() || self.file.is_some() {
            self.summary
        } else {
            &self.body
        }
    }

    pub(super) fn icon(&self) -> &'static str {
        self.icon
    }

    pub(super) fn into_draft(self) -> Draft {
        let actions = self.file.as_ref().map_or_else(Vec::new, |_| {
            ["default", "open"]
                .map(|key| Action {
                    key: key.into(),
                    label: tr("shell-open").into(),
                })
                .into()
        });
        Draft {
            app_name: "enzimOS".into(),
            desktop_entry: None,
            replaces: 0,
            summary: self.summary.into(),
            body: self.body,
            actions,
            reply: None,
            image: self.image,
            urgency: Urgency::Normal,
            resident: false,
            transient: self.transient,
            timeout_ms: -1,
            value: None,
            saved_file: self.file,
        }
    }
}

/// A file retained by a capture worker and verified again before a user opens it.
#[derive(Clone, PartialEq, Eq)]
pub struct SavedFile {
    path: PathBuf,
    identity: FileIdentity,
    open_error: &'static str,
}

impl std::fmt::Debug for SavedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SavedCaptureFile")
    }
}

impl SavedFile {
    pub(crate) fn new(path: PathBuf, identity: FileIdentity, open_error: &'static str) -> Self {
        Self {
            path,
            identity,
            open_error,
        }
    }

    fn verify(&self) -> Result<(), String> {
        let changed = || tr("shell-file-changed-or-is-no-longer-available").to_owned();
        if !self.path.is_absolute() {
            return Err(changed());
        }
        let file = rustix::fs::openat2(
            rustix::fs::CWD,
            &self.path,
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
        .map(std::fs::File::from)
        .map_err(|_| changed())?;
        let metadata = file.metadata().map_err(|_| changed())?;
        if !metadata.is_file()
            || metadata.mode() & 0o111 != 0
            || FileIdentity::from_metadata(&metadata) != self.identity
        {
            return Err(changed());
        }
        Ok(())
    }

    pub(super) async fn open(&self, token: Option<&str>) -> Result<(), String> {
        let file = self.clone();
        let failed = || self.open_error.to_owned();
        tokio::task::spawn_blocking(move || file.verify())
            .await
            .map_err(|_| failed())??;
        let mut command = tokio::process::Command::new("/usr/bin/gio");
        command
            .args(["open", "--"])
            .arg(&self.path)
            .env_remove("XDG_ACTIVATION_TOKEN")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(token) = token.filter(|token| !token.is_empty() && token.len() <= 4096) {
            command.env("XDG_ACTIVATION_TOKEN", token);
        }
        let mut child = command.spawn().map_err(|_| failed())?;
        let mut exited = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(15), child.wait()).await
        });
        match tokio::time::timeout(Duration::from_millis(300), &mut exited).await {
            Ok(Ok(Ok(Ok(status)))) if status.success() => Ok(()),
            Err(_) => Ok(()),
            _ => Err(failed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        capture::{Preferences, keep},
        notifications::{
            archive::Storage,
            model::{Action as Invoke, Model, Policy},
            origin::Origin,
            server::Snapshot,
        },
    };
    use std::{os::unix::fs::PermissionsExt, time::Instant};

    #[test]
    fn saved_screenshot_keeps_its_action_only_while_active() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let picture = raster::DynamicImage::new_rgba8(512, 128);
        let saved = keep(
            &picture,
            &Preferences {
                screenshot_folder: temp.path().to_str().unwrap().into(),
                copy_to_clipboard: false,
                ..Preferences::default()
            },
            None,
        )
        .unwrap();
        let path = saved.path.clone().unwrap();
        let draft = CaptureNotice::screenshot(saved, &picture, true).into_draft();
        let preview = draft.image.as_ref().unwrap();
        assert!(preview.preview());
        let (_, width, height) = preview.pixels();
        assert_eq!((width, height), (256, 64));
        assert!(draft.body.is_empty());
        assert!(draft.permits_action("default"));
        assert!(draft.permits_action("open"));
        let action = draft.saved_file.clone().unwrap();
        action.verify().unwrap();

        let policy = Policy {
            dnd: true,
            history: true,
            ..Policy::default()
        };
        let mut model = Model::new(1, policy.clone()).unwrap();
        let posted = model
            .post(
                Origin::internal("camera").unwrap(),
                draft,
                Instant::now(),
                1,
                false,
            )
            .unwrap();
        assert!(!posted.toast);
        let key = posted.notification.unwrap().key;
        assert!(
            model
                .close_from_app(&Origin::fixture("Other", 42), key.id, 1)
                .is_err()
        );
        assert!(
            model
                .authorize(key, Invoke::Invoke("default".into()))
                .is_ok()
        );
        model.dismiss(key, 2);
        assert!(
            model
                .authorize(key, Invoke::Invoke("default".into()))
                .is_err()
        );

        let mut snapshot = Snapshot::empty(1, policy.clone());
        snapshot.history.extend(model.history().iter().cloned());
        let store = Storage::for_settings(&temp.path().join("settings.json")).unwrap();
        store.save(&policy, &snapshot, 3).unwrap();
        let text =
            std::fs::read_to_string(temp.path().join("settings.json.notifications")).unwrap();
        assert!(!text.contains(path.to_str().unwrap()));
        let history = store.load(&policy, 3).unwrap();
        assert!(history[0].image.as_ref().unwrap().preview());

        let moved = temp.path().join("moved.png");
        std::fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        assert!(action.verify().is_err());
    }

    #[test]
    fn clipboard_only_capture_is_transient_and_has_no_open_action() {
        let draft = CaptureNotice::screenshot(
            Saved {
                path: None,
                copied: true,
                identity: None,
            },
            &raster::DynamicImage::new_rgba8(10, 10),
            false,
        )
        .into_draft();
        assert!(draft.transient);
        assert!(draft.actions.is_empty());
        assert!(draft.image.is_none());
    }

    #[test]
    fn recording_notices_open_retained_files_and_keep_failures_transient() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("recording.mp4");
        std::fs::write(&path, b"fixture").unwrap();
        let identity = FileIdentity::from_metadata(&std::fs::metadata(&path).unwrap());
        let file = SavedFile::new(
            path.clone(),
            identity,
            tr("shell-recording-could-not-be-opened"),
        );

        for ending in [
            Ending::Saved(file.clone()),
            Ending::Incomplete(file.clone()),
        ] {
            let draft = CaptureNotice::recording(ending).into_draft();
            assert!(!draft.transient);
            assert!(draft.permits_action("open"));
            assert_eq!(draft.saved_file, Some(file.clone()));
        }

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(file.verify().is_err());

        let failed = CaptureNotice::recording(Ending::Failed("Encoder error".into()));
        assert_eq!(failed.fallback(), "Encoder error");
        let failed = failed.into_draft();
        assert_eq!(failed.body, "Encoder error");
        assert!(failed.transient);
        assert!(failed.actions.is_empty());
    }
}
