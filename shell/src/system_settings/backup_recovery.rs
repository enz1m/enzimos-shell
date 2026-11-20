//! Protected recovery-password delivery; confirmation always requires a separate user action.

use super::{SystemSettings, enzimos};
use enzui_core::native_child::{NativeCommand, Stdio};
use enzui_shell::i18n::tr;
use iced::futures::StreamExt;
use std::{
    collections::HashMap,
    os::fd::{AsFd as _, OwnedFd},
    path::{Path, PathBuf},
    time::Duration,
};
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Clone, Copy, Debug)]
pub enum Delivery {
    Save,
    Print,
}

#[derive(Clone, Debug)]
pub struct Delivered {
    pub ticket: String,
    pub printed: bool,
}

impl SystemSettings {
    pub async fn deliver_backup_recovery(&self, delivery: Delivery) -> Result<Delivered, String> {
        if !self.writes_allowed || self.demo {
            return Err(tr("shell-recovery-password-export-is-unavailable-in-this-session").into());
        }
        let destination = if matches!(delivery, Delivery::Save) {
            Some(choose_destination().await?)
        } else {
            None
        };
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        let (metadata, password) = backend.recovery_export().await.map_err(|r| r.message())?;
        let ticket = metadata
            .get("ticket")
            .and_then(serde_json::Value::as_str)
            .filter(|s| s.len() == 64)
            .ok_or_else(|| tr("shell-invalid-recovery-export"))?
            .to_owned();
        let uuid = metadata
            .get("destination_uuid")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| tr("shell-invalid-backup-identity"))?;
        if let Some((folder, name)) = destination {
            protected_helper(
                Delivery::Save,
                &password,
                None,
                &[
                    "--directory",
                    folder
                        .to_str()
                        .ok_or_else(|| tr("shell-this-folder-name-cannot-be-used"))?,
                    "--name",
                    &name,
                    "--backup-filesystem-uuid",
                    uuid,
                ],
            )
            .await?;
            drop(password);
        } else {
            let document = rustix::fs::memfd_create(
                "enzimos-recovery-document",
                rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
            )
            .map_err(|_| tr("shell-could-not-prepare-a-private-print-document"))?;
            rustix::fs::fchmod(&document, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
                .map_err(|_| tr("shell-could-not-protect-the-print-document"))?;
            protected_helper(Delivery::Print, &password, Some(&document), &[]).await?;
            drop(password);
            let seals = rustix::fs::SealFlags::SEAL
                | rustix::fs::SealFlags::GROW
                | rustix::fs::SealFlags::SHRINK
                | rustix::fs::SealFlags::WRITE;
            if !rustix::fs::fcntl_get_seals(&document).is_ok_and(|actual| actual.contains(seals)) {
                return Err(tr("shell-the-recovery-print-document-could-not-be-protected").into());
            }
            print_document(&document).await?;
        }
        Ok(Delivered {
            ticket,
            printed: matches!(delivery, Delivery::Print),
        })
    }

    pub async fn confirm_backup_recovery(&self, ticket: String) -> Result<(), String> {
        if !self.writes_allowed || self.demo {
            return Err(
                tr("shell-recovery-copy-confirmation-is-unavailable-in-this-session").into(),
            );
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        backend
            .recovery_confirm(ticket)
            .await
            .map_err(|r| r.message())
    }
}

fn protected_command(
    program: &Path,
    delivery: Delivery,
    password: &OwnedFd,
    document: Option<&OwnedFd>,
    args: &[&str],
) -> Result<NativeCommand, String> {
    if matches!(delivery, Delivery::Print) != document.is_some() {
        return Err(tr("shell-invalid-recovery-delivery-descriptors").into());
    }
    let failed = |_| String::from(tr("shell-could-not-pass-the-protected-recovery-document"));
    // The native ABI requires FDs above 2; they are inherited by this child
    // only, so concurrent application launches never see them.
    let mut command = NativeCommand::new(program)
        .args([
            "backup-recovery",
            if matches!(delivery, Delivery::Print) {
                "print"
            } else {
                "save"
            },
        ])
        .args(args)
        .args(["--descriptor", "3"])
        .env_clear_and_set([("PATH", "/usr/bin"), ("LC_ALL", "C.UTF-8")])
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Null)
        .descriptor(password.as_fd(), 3)
        .map_err(failed)?;
    if let Some(document) = document {
        command = command
            .args(["--document-fd", "4"])
            .descriptor(document.as_fd(), 4)
            .map_err(failed)?;
    }
    Ok(command)
}

struct KillOnDrop(OwnedFd);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // A reaped helper's pidfd refuses the signal, so this never reaches a reused PID.
        let _ = rustix::process::pidfd_send_signal(&self.0, rustix::process::Signal::KILL);
    }
}

async fn run_protected(
    command: NativeCommand,
    deadline: Duration,
) -> Result<(bool, Vec<u8>), String> {
    let mut child = command
        .spawn()
        .map_err(|_| tr("shell-could-not-start-protected-recovery-delivery"))?;
    let pid = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw);
    let _guard = KillOnDrop(
        pid.and_then(|pid| {
            rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok()
        })
        .ok_or_else(|| tr("shell-could-not-follow-protected-recovery-delivery"))?,
    );
    let output = child
        .stdout
        .take()
        .ok_or_else(|| tr("shell-could-not-start-protected-recovery-delivery"))?;
    let task = tokio::task::spawn_blocking(move || {
        let mut status = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(output, 4097), &mut status);
        child.wait().map(|exit| (exit.success(), status))
    });
    match tokio::time::timeout(deadline, task).await {
        Ok(Ok(Ok(result))) => Ok(result),
        Ok(_) => Err(tr("shell-could-not-start-protected-recovery-delivery").into()),
        Err(_) => Err(tr(
            "shell-recovery-delivery-timed-out-check-the-destination-before-trying-again",
        )
        .into()),
    }
}

async fn protected_helper(
    delivery: Delivery,
    password: &OwnedFd,
    document: Option<&OwnedFd>,
    args: &[&str],
) -> Result<(), String> {
    let command = protected_command(
        Path::new("/usr/lib/enzimos-native/enzimos-policy"),
        delivery,
        password,
        document,
        args,
    )?;
    // Document bytes use their inherited descriptor; stdout carries only public status.
    let (success, status) = run_protected(command, Duration::from_secs(30)).await?;
    let expected = if document.is_some() {
        "recovery-print-document-ready"
    } else {
        "recovery-file-saved"
    };
    let metadata = if status.len() <= 4096 {
        serde_json::from_slice::<serde_json::Value>(&status).ok()
    } else {
        None
    };
    if !success
        || !metadata.as_ref().is_some_and(|value| {
            value["status"] == expected && value["confirmation_recorded"] == false
        })
    {
        if document.is_none()
            && metadata.as_ref().is_some_and(|value| {
                value["reason"]
                    == enzui_shell::i18n::tr("shell-a-file-with-that-name-already-exists")
            })
        {
            return Err(tr("shell-a-file-with-that-name-already-exists-choose-a-new-filename-the-original-will").into());
        }
        return Err(if document.is_some() {
            tr("shell-this-password-cannot-be-printed-safely-save-it-to-a-separate-drive-instead")
        } else {
            tr("shell-choose-a-writable-drive-separate-from-this-computer-and-the-backup-drive-an")
        }
        .into());
    }
    Ok(())
}

async fn request(
    interface: &str,
    member: &str,
    document: Option<&OwnedFd>,
) -> Result<HashMap<String, OwnedValue>, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-the-desktop-dialog-service-is-unavailable"))?;
    let token = format!("recovery_{}", enzimos::request_id());
    let sender = connection
        .unique_name()
        .ok_or_else(|| tr("shell-the-desktop-session-has-no-identity"))?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let expected = format!("{PATH}/request/{sender}/{token}");
    let request = zbus::Proxy::new(
        &connection,
        PORTAL,
        expected.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|_| tr("shell-could-not-prepare-the-desktop-dialog"))?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(|_| tr("shell-could-not-follow-the-desktop-dialog"))?;
    let portal = zbus::Proxy::new(&connection, PORTAL, PATH, interface)
        .await
        .map_err(|_| tr("shell-the-desktop-dialog-service-is-unavailable"))?;
    let mut options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let opened: OwnedObjectPath = if let Some(document) = document {
        portal
            .call(
                member,
                &(
                    "",
                    enzui_shell::i18n::tr("shell-enzimos-recovery-password"),
                    Fd::from(document),
                    options,
                ),
            )
            .await
    } else {
        options.insert("current_name", Value::from("enzimOS-recovery-password.txt"));
        options.insert("accept_label", Value::from(tr("shell-save-a-new-file")));
        options.insert("modal", Value::from(true));
        portal
            .call(
                member,
                &(
                    "",
                    enzui_shell::i18n::tr(
                        "shell-choose-a-drive-separate-from-your-computer-and-backup-drive",
                    ),
                    options,
                ),
            )
            .await
    }
    .map_err(|_| tr("shell-could-not-open-the-desktop-dialog"))?;
    if opened.as_str() != expected {
        return Err(tr("shell-the-desktop-dialog-returned-an-unexpected-request").into());
    }
    let response = tokio::time::timeout(Duration::from_secs(180), responses.next()).await;
    let Ok(Some(signal)) = response else {
        let _: Result<(), _> = request.call("Close", &()).await;
        return Err(tr(
            "shell-the-recovery-dialog-expired-no-recovery-copy-confirmation-was-recorded",
        )
        .into());
    };
    let (code, values): (u32, HashMap<String, OwnedValue>) = signal
        .body()
        .deserialize()
        .map_err(|_| tr("shell-invalid-desktop-dialog-response"))?;
    if code != 0 {
        return Err(tr(
            "shell-recovery-delivery-was-cancelled-no-recovery-copy-confirmation-was-recorded",
        )
        .into());
    }
    Ok(values)
}

async fn choose_destination() -> Result<(PathBuf, String), String> {
    let mut response = request("org.freedesktop.portal.FileChooser", "SaveFile", None).await?;
    let uris: Vec<String> = response
        .remove("uris")
        .ok_or_else(|| tr("shell-no-file-selected"))?
        .try_into()
        .map_err(|_| tr("shell-invalid-file-selection"))?;
    if uris.len() != 1 {
        return Err(tr("shell-choose-one-local-file").into());
    }
    local_destination(&uris[0])
}

fn local_destination(uri: &str) -> Result<(PathBuf, String), String> {
    if uri.contains(['?', '#']) {
        return Err(tr("shell-invalid-file-address").into());
    }
    let encoded = uri
        .strip_prefix("file:///")
        .ok_or_else(|| tr("shell-choose-a-local-folder-on-a-separate-drive"))?;
    let mut bytes = vec![b'/'];
    let mut remaining = encoded.as_bytes();
    while let Some((&first, tail)) = remaining.split_first() {
        if first == b'%' {
            if tail.len() < 2 {
                return Err(tr("shell-invalid-folder-address").into());
            }
            let hex =
                std::str::from_utf8(&tail[..2]).map_err(|_| tr("shell-invalid-folder-address"))?;
            bytes
                .push(u8::from_str_radix(hex, 16).map_err(|_| tr("shell-invalid-folder-address"))?);
            remaining = &tail[2..];
        } else {
            bytes.push(first);
            remaining = tail;
        }
    }
    if bytes.contains(&0) {
        return Err(tr("shell-invalid-folder-address").into());
    }
    let path = PathBuf::from(
        String::from_utf8(bytes).map_err(|_| tr("shell-this-folder-name-cannot-be-used"))?,
    );
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| tr("shell-choose-a-filename"))?
        .to_owned();
    let parent = path
        .parent()
        .ok_or_else(|| tr("shell-choose-an-existing-local-folder"))?;
    let canonical = parent
        .canonicalize()
        .map_err(|_| tr("shell-the-selected-folder-is-no-longer-available"))?;
    if canonical != parent || !canonical.is_dir() {
        return Err(tr("shell-choose-an-existing-local-folder-without-symbolic-links").into());
    }
    Ok((canonical, name))
}

async fn print_document(document: &OwnedFd) -> Result<(), String> {
    request("org.freedesktop.portal.Print", "Print", Some(document))
        .await
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn backup_recovery_async_behaviour() {
        // Native delivery preserves private fds and keeps stdio public.
        {
            let dir = tempfile::tempdir().unwrap();
            let helper = dir.path().join("native-fixture");
            std::fs::write(&helper, r#"#!/usr/bin/python3
import os,sys,json,fcntl
assert sys.argv[1]=='backup-recovery'
action=sys.argv[2];args=dict(zip(sys.argv[3::2],sys.argv[4::2]))
fd=int(args['--descriptor']);assert fd>=3 and fcntl.fcntl(fd,fcntl.F_GETFD)==0
assert os.fstat(fd).st_mode&0o7777==0o600 and os.fstat(fd).st_uid==os.geteuid()
assert fcntl.fcntl(fd,fcntl.F_GET_SEALS)&(fcntl.F_SEAL_SEAL|fcntl.F_SEAL_SHRINK|fcntl.F_SEAL_GROW|fcntl.F_SEAL_WRITE)==(fcntl.F_SEAL_SEAL|fcntl.F_SEAL_SHRINK|fcntl.F_SEAL_GROW|fcntl.F_SEAL_WRITE)
assert sys.stdin.read()==''
assert os.read(fd,4096)==b'PUBLIC-RECOVERY-FIXTURE'
if action=='save':assert args['--directory']=='A space $(touch must-not-exist)'
if action=='print':
 out=int(args['--document-fd']);assert out>=3 and out!=fd and fcntl.fcntl(out,fcntl.F_GETFD)==0
 os.write(out,b'%PDF-PUBLIC-FIXTURE')
 fcntl.fcntl(out,fcntl.F_ADD_SEALS,fcntl.F_SEAL_SEAL|fcntl.F_SEAL_SHRINK|fcntl.F_SEAL_GROW|fcntl.F_SEAL_WRITE)
print(json.dumps({'status':'recovery-print-document-ready' if action=='print' else 'recovery-file-saved','confirmation_recorded':False}))
"#).unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            for delivery in [Delivery::Save, Delivery::Print] {
                let fd = rustix::fs::memfd_create(
                    "public-recovery-fixture",
                    rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
                )
                .unwrap();
                let mut input = std::fs::File::from(fd);
                input.write_all(b"PUBLIC-RECOVERY-FIXTURE").unwrap();
                input.rewind().unwrap();
                let input: OwnedFd = input.into();
                rustix::fs::fchmod(&input, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
                    .unwrap();
                rustix::fs::fcntl_add_seals(
                    &input,
                    rustix::fs::SealFlags::SEAL
                        | rustix::fs::SealFlags::SHRINK
                        | rustix::fs::SealFlags::GROW
                        | rustix::fs::SealFlags::WRITE,
                )
                .unwrap();
                let document = matches!(delivery, Delivery::Print).then(|| {
                    rustix::fs::memfd_create(
                        "public-document-fixture",
                        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
                    )
                    .unwrap()
                });
                if let Some(document) = &document {
                    rustix::fs::fchmod(document, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
                        .unwrap();
                }
                let args = if matches!(delivery, Delivery::Save) {
                    vec!["--directory", "A space $(touch must-not-exist)"]
                } else {
                    Vec::new()
                };
                let command =
                    protected_command(&helper, delivery, &input, document.as_ref(), &args).unwrap();
                let (success, status_bytes) = run_protected(command, Duration::from_secs(30))
                    .await
                    .unwrap();
                assert!(success);
                assert!(!status_bytes.windows(7).any(|window| window == b"FIXTURE"));
                let status: serde_json::Value = serde_json::from_slice(&status_bytes).unwrap();
                assert_eq!(status["confirmation_recorded"], false);
                assert_eq!(
                    rustix::io::fcntl_getfd(&input).unwrap(),
                    rustix::io::FdFlags::CLOEXEC
                );
                if let Some(document) = document {
                    assert_eq!(
                        rustix::io::fcntl_getfd(&document).unwrap(),
                        rustix::io::FdFlags::CLOEXEC
                    );
                    assert!(
                        rustix::fs::fcntl_get_seals(&document)
                            .unwrap()
                            .contains(rustix::fs::SealFlags::WRITE)
                    );
                    let mut file = std::fs::File::from(document);
                    file.seek(SeekFrom::Start(0)).unwrap();
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes).unwrap();
                    assert_eq!(bytes, b"%PDF-PUBLIC-FIXTURE");
                }
            }
        }
        // Native delivery deadline kills the helper.
        async {
            let dir = tempfile::tempdir().unwrap();
            let helper = dir.path().join("stalled-fixture");
            let pid_file = dir.path().join("pid");
            std::fs::write(
                &helper,
                format!(
                    "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                    pid_file.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            let input: OwnedFd = rustix::fs::memfd_create(
                "public-recovery-fixture",
                rustix::fs::MemfdFlags::CLOEXEC,
            )
            .unwrap();
            let command = protected_command(&helper, Delivery::Save, &input, None, &[]).unwrap();
            assert!(
                run_protected(command, Duration::from_millis(500))
                    .await
                    .unwrap_err()
                    .contains("timed out")
            );
            let pid = std::fs::read_to_string(&pid_file).unwrap();
            let process = std::path::PathBuf::from(format!("/proc/{}", pid.trim()));
            for _ in 0..40 {
                if !process.exists() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("the stalled helper outlived its deadline");
        }
        .await;
    }
}
