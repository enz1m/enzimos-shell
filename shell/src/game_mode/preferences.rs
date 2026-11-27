use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionKey {
    KeepAwake,
    SplitLock,
    Priority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub keep_awake: bool,
    pub disable_splitlock: bool,
    pub priority: u8,
    /// The system's GameMode starts enzimOS's Performance power-mode script,
    /// the OS's single owner of CPU policy while a game is active.
    pub performance_profile: bool,
    pub error: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            keep_awake: true,
            disable_splitlock: true,
            priority: 0,
            performance_profile: false,
            error: None,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".config"))
        })
        .map(|p| p.join("gamemode.ini"))
}

pub fn read() -> Preferences {
    let mut prefs = Preferences::default();
    for path in [
        PathBuf::from("/usr/share/gamemode/gamemode.ini"),
        PathBuf::from("/etc/gamemode.ini"),
    ]
    .into_iter()
    .chain(path())
    {
        match load(&path) {
            Ok(Some(text)) => {
                if let Err(error) = apply(&text, &mut prefs) {
                    prefs.error = Some(error);
                }
            }
            Ok(None) => {}
            Err(error) => prefs.error = Some(error),
        }
    }
    prefs
}

fn apply(text: &str, prefs: &mut Preferences) -> Result<(), String> {
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if line.starts_with([';', '#']) {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if section == "[custom]" {
            if key.trim() == "start"
                && value.trim() == "/usr/lib/enzimos/gamemode-performance start"
            {
                prefs.performance_profile = true;
            }
            continue;
        }
        if section != "[general]" {
            continue;
        }
        match key.trim() {
            "inhibit_screensaver" => prefs.keep_awake = boolean(value)?,
            "disable_splitlock" => prefs.disable_splitlock = boolean(value)?,
            "renice" => {
                prefs.priority = value
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|n| *n <= 20)
                    .ok_or_else(|| tr("shell-unknown-gamemode-priority-value"))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn boolean(value: &str) -> Result<bool, String> {
    match value.trim() {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(tr("shell-unknown-gamemode-boolean-value").into()),
    }
}

fn load(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(
                tr!("shell-gamemode-configuration-could-not-be-read", "arg0" => format!("{:}", error)),
            );
        }
    };
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(tr("shell-gamemode-configuration-is-not-a-regular-file").into());
    }
    let mut text = String::new();
    file.take(65537)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 65536 {
        return Err(tr("shell-gamemode-configuration-exceeds-64-kib").into());
    }
    Ok(Some(text))
}

pub fn set(key: OptionKey, value: u8) -> Result<(), String> {
    set_at(
        &path().ok_or_else(|| tr("shell-gamemode-configuration-directory-is-unavailable"))?,
        key,
        value,
    )
}

fn set_at(path: &Path, key: OptionKey, value: u8) -> Result<(), String> {
    let name = match key {
        OptionKey::KeepAwake => "inhibit_screensaver",
        OptionKey::SplitLock => "disable_splitlock",
        OptionKey::Priority => "renice",
    };
    if value > if key == OptionKey::Priority { 20 } else { 1 } {
        return Err(tr("shell-gamemode-value-is-outside-its-allowed-range").into());
    }
    let before = load(path)?;
    let text = before.as_deref().unwrap_or("");
    let mut general = false;
    let mut found = false;
    let mut inserted = false;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && !trimmed.ends_with(']') {
            return Err(tr("shell-gamemode-configuration-contains-a-malformed-section").into());
        }
        if trimmed.starts_with('[') {
            if general && !inserted {
                let _ = writeln!(out, "{name}={value}");
                inserted = true;
            }
            general = trimmed == "[general]";
            if general && std::mem::replace(&mut found, true) {
                return Err(tr("shell-duplicate-gamemode-general-section").into());
            }
        }
        if general
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == name)
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if general && !inserted {
        let _ = writeln!(out, "{name}={value}");
    }
    if !found {
        let _ = write!(out, "\n[general]\n{name}={value}\n");
    }
    let directory = path
        .parent()
        .ok_or_else(|| tr("shell-gamemode-configuration-has-no-parent"))?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    use std::io::Write;
    temp.write_all(out.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    if load(path)? != before {
        return Err(tr("shell-gamemode-configuration-changed-while-saving").into());
    }
    if before.is_none() {
        temp.persist_noclobber(path).map_err(|e| e.to_string())?;
    } else {
        temp.persist(path).map_err(|e| e.to_string())?;
    }
    std::fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}
