//! First-login launch policy and model download handoff.
//!
//! The completion record suppresses automatic launches even if its contents are unreadable.

use serde_json::json;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The app id Welcome registers its window and desktop entry under.
pub const APP_ID: &str = "dev.enzui.Welcome";
/// The argument the autostart entry passes.
pub const FIRST_LOGIN_ARGUMENT: &str = "--first-login";
/// Completion record relative to the user's configuration directory.
pub const RECORD: &str = "enzimos/welcome.json";

/// How Welcome was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    /// From the session's autostart entry.
    FirstLogin,
    /// From the app list or a terminal.
    Manual,
}

impl Launch {
    #[must_use]
    pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Self {
        if args.into_iter().any(|arg| arg == FIRST_LOGIN_ARGUMENT) {
            Self::FirstLogin
        } else {
            Self::Manual
        }
    }
}

/// The session facts that decide a first-login launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    /// `ENZUI_DESKTOP_SESSION=installed`: a greetd login, not a preview.
    pub installed: bool,
    /// A remote-access desktop, which is not the account's first sitting.
    pub remote: bool,
}

impl Session {
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            installed: std::env::var("ENZUI_DESKTOP_SESSION").as_deref() == Ok("installed"),
            remote: std::env::var("ENZUI_REMOTE_SESSION").as_deref() == Ok("1"),
        }
    }
}

/// Whether Welcome shows its window. A manual launch always does; a
/// first-login launch only in a local installed session with no record.
#[must_use]
pub fn opens(launch: Launch, session: Session, completed: bool) -> bool {
    match launch {
        Launch::Manual => true,
        Launch::FirstLogin => session.installed && !session.remote && !completed,
    }
}

/// The completion record's path, or `None` without an absolute home.
#[must_use]
pub fn record_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
        .map(|config| config.join(RECORD))
}

/// Whether Welcome finished or was skipped on this account.
#[must_use]
pub fn completed(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// What the account chose about AI when Welcome ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ai {
    Skipped,
    Configured,
}

/// The completion record's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub ai: Ai,
    /// Models Settings accepted for download.
    pub queued: Vec<String>,
}

impl Completion {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "version": 1,
            "ai": match self.ai {
                Ai::Skipped => "skipped",
                Ai::Configured => "configured",
            },
            "queued": self.queued,
        })
    }
}

/// Writes the record atomically; writing it again replaces it.
///
/// # Errors
///
/// The folder or file cannot be created, written, synced or renamed.
pub fn record(path: &Path, completion: &Completion) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the record has no folder"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let temporary = path.with_extension("json.new");
    let _ = std::fs::remove_file(&temporary);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&temporary)?;
        let mut bytes = serde_json::to_vec_pretty(&completion.to_json())?;
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// What a catalog model does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Text,
    Speech,
}

/// A model `enzimos-inference-models catalog` offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOffer {
    pub id: String,
    pub title: String,
    /// As the tool prints it, such as `487 MB`.
    pub size: String,
    pub kind: ModelKind,
    pub added: bool,
}

/// Reads the catalog: id, title, size, languages, `added` or `available`,
/// then an optional `text` or `speech`; rows without the kind are speech.
#[must_use]
pub fn parse_catalog(text: &str) -> Vec<ModelOffer> {
    crate::local_ai::parse_catalog(text)
        .into_iter()
        .filter(|offer| model_id(&offer.id) && !offer.needs_runtime)
        .map(|offer| ModelOffer {
            id: offer.id,
            title: offer.title,
            size: offer.size,
            kind: if offer.speech {
                ModelKind::Speech
            } else {
                ModelKind::Text
            },
            added: offer.added,
        })
        .collect()
}

/// Model ids Settings accepts on its command line: 1 to 64 of `a-z`, `0-9`,
/// `.`, `_` and `-`, starting with a letter or digit.
#[must_use]
pub fn model_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// The model registry tool.
pub const MODELS_TOOL: &str = crate::local_ai::MODELS_TOOL;

/// Reads the catalog from the model tool; empty when it is not installed.
#[must_use]
pub fn read_catalog() -> Vec<ModelOffer> {
    std::process::Command::new(MODELS_TOOL)
        .arg("catalog")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_catalog(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

/// The Settings executable beside this one, as installed or built.
#[must_use]
pub fn settings_program() -> PathBuf {
    std::env::var_os("ENZUI_SHELL_BIN_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_owned))
        })
        .unwrap_or_else(|| PathBuf::from("/usr/lib/enzui/bin"))
        .join("enzui-settings")
}

/// The Settings command that queues `ids` for download; `None` for an empty
/// or invalid list.
#[must_use]
pub fn queue_command(program: &Path, ids: &[String]) -> Option<std::process::Command> {
    if ids.is_empty() || ids.len() > 2 || !ids.iter().all(|id| model_id(id)) {
        return None;
    }
    let mut command = std::process::Command::new(program);
    command
        .arg("--queue-models")
        .args(ids)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    Some(command)
}

/// Asks Settings to download and then select `ids`. Success means Settings
/// accepted a durable job, which reports its progress in the bar.
///
/// # Errors
///
/// The list is invalid, Settings could not start, or it refused the job.
pub fn queue_models(program: &Path, ids: &[String]) -> Result<(), String> {
    let mut command = queue_command(program, ids).ok_or_else(|| "invalid model list".to_owned())?;
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(&output.stderr);
    Err(reason
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Settings refused the downloads")
        .trim()
        .trim_start_matches("enzui-settings: ")
        .chars()
        .take(200)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_is_atomic_private_and_repeatable() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(RECORD);
        assert!(!completed(&path));
        let skipped = Completion {
            ai: Ai::Skipped,
            queued: Vec::new(),
        };
        record(&path, &skipped).unwrap();
        assert!(completed(&path));
        let configured = Completion {
            ai: Ai::Configured,
            queued: vec!["qwen3.5-0.8b".into()],
        };
        record(&path, &configured).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["ai"], "configured");
        assert_eq!(saved["queued"], json!(["qwen3.5-0.8b"]));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
