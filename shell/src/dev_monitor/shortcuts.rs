use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::path::{Component, Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutKind {
    Folder,
    Terminal,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Launch {
    pub argv: Vec<String>,
    pub directory: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Shortcuts {
    pub pid: Option<u32>,
    pub start_time: Option<u64>,
    pub directory: String,
    pub folder_uri: String,
    pub folder: Launch,
    pub terminal: Launch,
}
fn directory_valid(value: &str) -> bool {
    Path::new(value).is_absolute()
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}
fn file_uri(directory: &str) -> String {
    let encoded = directory
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    format!("file://{encoded}")
}
impl Shortcuts {
    pub(super) fn parse(text: &str, expected: &Target) -> Result<Self, String> {
        let value: Self = super::parse(text)?;
        if let Target::Process(id) = expected
            && (value.pid != Some(id.pid) || value.start_time != Some(id.start_time))
        {
            return Err(tr("shell-process-shortcut-identity-changed").into());
        }
        if !directory_valid(&value.directory)
            || value.folder_uri != file_uri(&value.directory)
            || value.folder.directory != value.directory
            || value.terminal.directory != value.directory
            || value.folder.argv != ["/usr/bin/xdg-open", value.folder_uri.as_str()]
            || value.terminal.argv
                != [
                    "/usr/bin/terminal-emulator",
                    "--working-directory",
                    value.directory.as_str(),
                ]
        {
            return Err(tr("shell-dev-monitor-returned-an-unsupported-launch-descriptor").into());
        }
        Ok(value)
    }
    pub fn launch(self, kind: ShortcutKind) -> Launch {
        match kind {
            ShortcutKind::Folder => self.folder,
            ShortcutKind::Terminal => self.terminal,
        }
    }
}
impl Launch {
    pub fn start(self, token: Option<String>) -> Result<(), String> {
        if crate::desktop_session::mode().nested() {
            return Err(tr("shell-developer-launches-are-not-enabled-in-this-preview").into());
        }
        if !directory_valid(&self.directory) || !Path::new(&self.directory).is_dir() {
            return Err(tr("shell-the-reviewed-project-directory-is-no-longer-available").into());
        }
        let folder = self.argv == ["/usr/bin/xdg-open", file_uri(&self.directory).as_str()];
        let terminal = self.argv
            == [
                "/usr/bin/terminal-emulator",
                "--working-directory",
                self.directory.as_str(),
            ];
        if !folder && !terminal {
            return Err(tr("shell-unsupported-developer-launch").into());
        }
        if terminal && crate::desktop_session::mode() != crate::desktop_session::Mode::Installed {
            return Err(tr("shell-project-terminal-launch-requires-the-installed-terminal-working-directory-co").into());
        }
        let mut command = std::process::Command::new(&self.argv[0]);
        command
            .args(&self.argv[1..])
            .current_dir(&self.directory)
            .env_remove("XDG_ACTIVATION_TOKEN")
            .env_remove("DESKTOP_STARTUP_ID");
        if let Some(token) = token {
            command.env("XDG_ACTIVATION_TOKEN", token);
        }
        let mut child = command.spawn().map_err(|error| tr!("shell-could-not-start-the-reviewed-application", "arg0" => format!("{:}", error)))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(directory: &str) -> serde_json::Value {
        let uri = file_uri(directory);
        serde_json::json!({"pid":42,"start_time":123,"directory":directory,"folder_uri":uri,
            "folder":{"argv":["/usr/bin/xdg-open",uri],"directory":directory},
            "terminal":{"argv":["/usr/bin/terminal-emulator","--working-directory",directory],"directory":directory}})
    }
    #[test]
    fn descriptors_keep_exact_data_arguments_and_reject_identity_command_cwd_or_uri_changes() {
        let target = Target::Process(ProcessId {
            pid: 42,
            start_time: 123,
        });
        let directory = "/tmp/a space/$(touch sentinel)#?";
        let value = fixture(directory);
        let parsed = Shortcuts::parse(&value.to_string(), &target).unwrap();
        assert_eq!(parsed.terminal.argv[2], directory);
        assert_eq!(
            parsed.folder_uri,
            "file:///tmp/a%20space/%24%28touch%20sentinel%29%23%3F"
        );
        for (key, replacement) in [
            ("pid", serde_json::json!(43)),
            ("start_time", serde_json::json!(124)),
            ("directory", serde_json::json!("/tmp/../etc")),
            ("folder_uri", serde_json::json!("https://example.com")),
        ] {
            let mut bad = value.clone();
            bad[key] = replacement;
            assert!(Shortcuts::parse(&bad.to_string(), &target).is_err());
        }
        let mut bad = value.clone();
        bad["terminal"]["argv"] = serde_json::json!(["/usr/bin/sh", "-c", "touch sentinel"]);
        assert!(Shortcuts::parse(&bad.to_string(), &target).is_err());
        let mut bad = value.clone();
        bad["folder"]["directory"] = serde_json::json!("/other");
        assert!(Shortcuts::parse(&bad.to_string(), &target).is_err());
        let mut bad = value.clone();
        bad["folder"]["argv"] = serde_json::json!(["/usr/bin/xdg-open", "--", "file:///tmp"]);
        assert!(Shortcuts::parse(&bad.to_string(), &target).is_err());
        let container = Target::Container(ContainerId {
            engine: "podman".into(),
            socket: "/run/qa.sock".into(),
            id: "a".repeat(64),
            name: "web".into(),
        });
        let mut container_value = value;
        container_value.as_object_mut().unwrap().remove("pid");
        container_value
            .as_object_mut()
            .unwrap()
            .remove("start_time");
        assert!(Shortcuts::parse(&container_value.to_string(), &container).is_ok());
    }
}
