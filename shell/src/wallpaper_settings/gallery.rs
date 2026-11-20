use enzui_shell::i18n::tr;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct GalleryEntry {
    pub(crate) id: String,
    pub(crate) source: String,
    pub(crate) thumbnail: String,
    pub(crate) digest: String,
    pub(crate) title: String,
}
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct Gallery {
    pub(crate) entries: Vec<GalleryEntry>,
    pub(crate) truncated: bool,
    pub(crate) skipped: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct LiveScene {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) animated: bool,
    #[serde(default)]
    pub(crate) parallax: bool,
    pub(crate) preview: Option<String>,
}

pub(crate) fn parse_live_scenes(output: &[u8], scenes: &Path) -> Result<Vec<LiveScene>, String> {
    let list: Vec<LiveScene> =
        serde_json::from_slice(output).map_err(|_| tr("shell-invalid-live-wallpaper-list"))?;
    let mut ids = std::collections::BTreeSet::new();
    if list.len() > 64
        || list.iter().any(|scene| {
            !enzui_shell::desktop_preferences::valid_scene_id(&scene.id)
                || !ids.insert(scene.id.clone())
                || scene.name.len() > 64
                || scene.description.len() > 400
                || scene
                    .name
                    .chars()
                    .chain(scene.description.chars())
                    .any(char::is_control)
                || scene.preview.as_ref().is_some_and(|preview| {
                    Path::new(preview) != scenes.join(&scene.id).join("preview.png")
                })
        })
    {
        return Err(tr("shell-invalid-live-wallpaper-entries").into());
    }
    Ok(list)
}

pub(crate) fn valid_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
pub(crate) fn parse_gallery(output: &[u8], cache: &Path) -> Result<Gallery, String> {
    let gallery: Gallery =
        serde_json::from_slice(output).map_err(|_| tr("shell-invalid-wallpaper-gallery-result"))?;
    let mut ids = std::collections::BTreeSet::new();
    if gallery.entries.len() > 48
        || gallery.entries.iter().any(|entry| {
            !valid_digest(&entry.id)
                || !valid_digest(&entry.digest)
                || !ids.insert(entry.id.clone())
                || !Path::new(&entry.source).is_absolute()
                || entry.source.len() > 4096
                || entry.title.len() > 800
                || entry.title.chars().any(char::is_control)
                || Path::new(&entry.thumbnail) != cache.join(format!("{}.png", entry.digest))
        })
    {
        return Err(tr("shell-invalid-wallpaper-gallery-entries").into());
    }
    Ok(gallery)
}

pub(crate) async fn run_child(
    mut command: tokio::process::Command,
    input: Vec<u8>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    limit: Duration,
    output_limit: u64,
) -> Result<(bool, Vec<u8>), String> {
    command
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| tr("shell-cannot-start-the-image-helper"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| tr("shell-image-helper-has-no-input"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| tr("shell-image-helper-has-no-result"))?
        .take(output_limit + 1);
    let mut output = Vec::new();
    let operation = async {
        stdin.write_all(&input).await?;
        drop(stdin);
        tokio::try_join!(child.wait(), stdout.read_to_end(&mut output))
    };
    let (status, _) = tokio::select! {
        result=operation=>result.map_err(|_| tr("shell-image-operation-stopped"))?,
        _=cancel.changed()=>{let _=child.start_kill();let _=child.wait().await;return Err(tr("shell-image-operation-canceled").into());},
        ()=tokio::time::sleep(limit)=>{let _=child.start_kill();let _=child.wait().await;return Err(tr("shell-image-operation-timed-out").into());},
    };
    if output.len() as u64 > output_limit {
        return Err(tr("shell-image-result-is-too-large").into());
    }
    Ok((status.success(), output))
}
