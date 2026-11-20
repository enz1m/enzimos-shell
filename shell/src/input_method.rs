//! Fcitx5 service selection from Settings, locale or existing profile.
//! Only installed sessions may change the service; previews leave host services untouched.
//!
//! enzimOS ships a user unit for Fcitx5. Elsewhere an installed `fcitx5`
//! runs as a transient user unit of this desktop.

use enzui_shell::i18n::tr;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const UNIT: &str = "enzimos-input-method.service";
/// The transient unit `fcitx5` runs in where the OS ships no unit.
const TRANSIENT: &str = "enzui-input-method";

fn os_unit() -> bool {
    ["/etc/systemd/user", "/usr/lib/systemd/user"]
        .iter()
        .any(|dir| Path::new(dir).join(UNIT).is_file())
}

// What runs the input method: the OS unit, or the `fcitx5` program. Read
// once per process, since pages ask on every frame.
fn service() -> Option<&'static Service> {
    static SERVICE: OnceLock<Option<Service>> = OnceLock::new();
    SERVICE
        .get_or_init(|| {
            if os_unit() {
                return Some(Service::Unit);
            }
            crate::system_controls::SystemControls::from_environment()
                .tool("fcitx5")
                .map(Service::Program)
        })
        .as_ref()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Service {
    Unit,
    Program(PathBuf),
}

/// Whether Fcitx5 is installed: the OS's unit or the `fcitx5` program.
#[must_use]
pub fn available() -> bool {
    service().is_some()
}

// The `systemctl`/`systemd-run` command that turns the input method on or
// off; `running` is whether the transient unit is active now.
fn command(service: &Service, on: bool, running: bool) -> Option<Vec<OsString>> {
    let words = |words: &[&str]| words.iter().map(OsString::from).collect::<Vec<_>>();
    match (service, on, running) {
        (Service::Unit, on, _) => Some(words(&[
            "systemctl",
            "--user",
            if on { "start" } else { "stop" },
            "--no-block",
            UNIT,
        ])),
        (Service::Program(_), true, true) | (Service::Program(_), false, false) => None,
        (Service::Program(program), true, false) => {
            let mut command = words(&[
                "systemd-run",
                "--user",
                "--collect",
                "--quiet",
                &format!("--unit={TRANSIENT}"),
            ]);
            command.extend([program.as_os_str().to_owned(), "--replace".into()]);
            Some(command)
        }
        (Service::Program(_), false, true) => Some(words(&[
            "systemctl",
            "--user",
            "stop",
            &format!("{TRANSIENT}.service"),
        ])),
    }
}

/// The automatic choice for `locale` and the text of a Fcitx5 profile.
#[must_use]
pub fn automatic(locale: &str, profile: Option<&str>) -> bool {
    let language = locale.split(['_', '.', '@']).next().unwrap_or_default();
    matches!(language, "zh" | "ja" | "ko" | "yue")
        || profile.is_some_and(|text| {
            let mut item = false;
            text.lines().map(str::trim).any(|line| {
                if line.starts_with('[') {
                    item = line.contains("/Items/");
                    return false;
                }
                item && line
                    .strip_prefix("Name=")
                    .is_some_and(|name| !name.is_empty() && !name.starts_with("keyboard-"))
            })
        })
}

/// Whether the input method should run, given the account's setting.
#[must_use]
pub fn wanted(setting: Option<bool>) -> bool {
    setting.unwrap_or_else(|| {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()));
        let profile = profile_path().and_then(|path| std::fs::read_to_string(path).ok());
        automatic(locale.as_deref().unwrap_or_default(), profile.as_deref())
    })
}

fn profile_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("fcitx5/profile"))
}

/// Starts or stops the service in the user manager.
pub async fn apply(on: bool) -> Result<(), String> {
    let Some(service) = service() else {
        return Err(tr("shell-the-input-method-could-not-be-changed").to_owned());
    };
    let running = *service != Service::Unit
        && run(&[
            "systemctl".into(),
            "--user".into(),
            "is-active".into(),
            "--quiet".into(),
            format!("{TRANSIENT}.service").into(),
        ])
        .await
        .is_ok();
    match command(service, on, running) {
        Some(command) => run(&command).await,
        None => Ok(()),
    }
}

async fn run(command: &[OsString]) -> Result<(), String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| tr("shell-the-input-method-could-not-be-changed").to_owned())?;
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|_| tr("shell-the-input-method-could-not-be-changed").to_owned())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| tr("shell-the-input-method-could-not-be-changed").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without the enzimOS unit an installed `fcitx5` runs in this
    /// desktop's transient unit, started once and stopped only when it runs;
    /// with the unit, the unit is used as before.
    ///
    /// Sabotage: start the enzimOS unit whatever the service, or start the
    /// transient unit again while it runs.
    #[test]
    fn fcitx5_runs_in_a_transient_unit_without_the_enzimos_unit() {
        let program = Service::Program("/usr/bin/fcitx5".into());
        let text = |command: Option<Vec<OsString>>| {
            command.map(|words| {
                words
                    .iter()
                    .map(|word| word.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        };
        assert_eq!(
            text(command(&program, true, false)).as_deref(),
            Some(
                "systemd-run --user --collect --quiet --unit=enzui-input-method /usr/bin/fcitx5 --replace"
            )
        );
        assert_eq!(command(&program, true, true), None);
        assert_eq!(command(&program, false, false), None);
        assert_eq!(
            text(command(&program, false, true)).as_deref(),
            Some("systemctl --user stop enzui-input-method.service")
        );
        assert_eq!(
            text(command(&Service::Unit, true, false)).as_deref(),
            Some("systemctl --user start --no-block enzimos-input-method.service")
        );
    }
}
