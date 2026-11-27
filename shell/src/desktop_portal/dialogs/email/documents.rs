use super::Options;
use crate::desktop_portal::bytes_path;
use rustix::fs::{Mode, OFlags, open};
use std::ffi::OsString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use zbus::zvariant::OwnedFd;

const DOCUMENTS: &str = "org.freedesktop.portal.Documents";

fn files(paths: Vec<PathBuf>) -> Option<(Vec<OwnedFd>, Vec<OsString>)> {
    let mut descriptors = Vec::with_capacity(paths.len());
    let mut names = Vec::with_capacity(paths.len());
    for path in paths {
        let file = File::from(open(&path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty()).ok()?);
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()?;
        names.push(path.file_name()?.to_owned());
        descriptors.push(std::os::fd::OwnedFd::from(file).into());
    }
    Some((descriptors, names))
}

pub(super) async fn export(paths: Vec<PathBuf>, app_id: &str) -> Option<Vec<PathBuf>> {
    if paths.len() > 128 {
        return None;
    }
    let (descriptors, names) = tokio::task::spawn_blocking(move || files(paths))
        .await
        .ok()??;
    let connection = zbus::Connection::session().await.ok()?;
    let portal = zbus::Proxy::new(
        &connection,
        DOCUMENTS,
        "/org/freedesktop/portal/documents",
        DOCUMENTS,
    )
    .await
    .ok()?;
    let (ids, options): (Vec<String>, Options) = portal
        .call("AddFull", &(descriptors, 0u32, app_id, vec!["read"]))
        .await
        .ok()?;
    let mount = bytes_path(options.get("mountpoint")?)?;
    if ids.len() != names.len() {
        return None;
    }
    ids.into_iter()
        .zip(names)
        .map(|(id, name)| {
            (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
                .then(|| mount.join(id).join(name))
        })
        .collect()
}
