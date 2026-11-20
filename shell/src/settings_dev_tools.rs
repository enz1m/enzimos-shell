use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::tr;

#[derive(Default)]
pub(super) struct DevToolsLaunch {
    epoch: u64,
    pending: Option<(u64, window::Id)>,
    closed: bool,
    error: Option<String>,
}
impl DevToolsLaunch {
    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }
    fn begin(&mut self, window: Option<window::Id>) -> Option<(u64, window::Id)> {
        if self.closed || self.pending.is_some() {
            return None;
        }
        let window = window?;
        self.epoch = self.epoch.checked_add(1)?;
        let request = (self.epoch, window);
        self.pending = Some(request);
        Some(request)
    }
    fn take(&mut self, epoch: u64, window: window::Id, current: Option<window::Id>) -> bool {
        if self.closed || current != Some(window) || self.pending != Some((epoch, window)) {
            return false;
        }
        self.pending = None;
        true
    }
    pub(super) fn cancel(&mut self, closed: bool) {
        self.pending = None;
        self.closed |= closed;
    }
}

fn command(executable: &std::path::Path, token: Option<String>) -> std::process::Command {
    let mut command = std::process::Command::new(executable);
    command
        .env_remove("XDG_ACTIVATION_TOKEN")
        .env_remove("DESKTOP_STARTUP_ID");
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
    command
}

impl SettingsApp {
    pub(super) fn open_dev_tools(&mut self) -> Task<Message> {
        if self.closing {
            return Task::none();
        }
        let Some((epoch, window)) = self.dev_tools_launch.begin(self.window) else {
            return Task::none();
        };
        window::request_activation_token(window).map(move |result| {
            Message::DevToolsToken(
                epoch,
                window,
                result.ok().map(iced::window::ActivationToken::into_raw),
            )
        })
    }
    pub(super) fn dev_tools_token(
        &mut self,
        epoch: u64,
        window: window::Id,
        token: Option<String>,
    ) -> Task<Message> {
        if self.closing || !self.dev_tools_launch.take(epoch, window, self.window) {
            return Task::none();
        }
        let result = (|| {
            let executable = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .ok_or_else(|| tr("shell-settings-executable-has-no-parent").to_owned())?
                .join("enzui-dev-tools");
            let mut child = command(&executable, token)
                .spawn()
                .map_err(|e| e.to_string())?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        })();
        self.dev_tools_launch_settled(result);
        Task::none()
    }
    pub(super) fn dev_tools_launch_settled(&mut self, result: Result<(), String>) {
        if let Some(previous) = self.dev_tools_launch.error.take()
            && self.error.as_ref() == Some(&previous)
        {
            self.error = None;
        }
        if let Err(error) = result {
            let error = tr!("shell-could-not-open-dev-tools", "arg0" => error);
            self.error = Some(error.clone());
            self.dev_tools_launch.error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_dev_tools_behaviour() {
        // Tokens bind the original window and only one pending request.
        {
            let mut launch = DevToolsLaunch::default();
            let window = window::Id::unique();
            assert!(launch.begin(None).is_none());
            let (epoch, id) = launch.begin(Some(window)).unwrap();
            assert!(launch.begin(Some(window)).is_none());
            assert!(!launch.take(epoch, window, Some(window::Id::unique())));
            assert!(!launch.take(epoch + 1, window, Some(window)));
            assert!(launch.take(epoch, id, Some(window)));
            assert!(!launch.take(epoch, id, Some(window)));
        }
        // Navigation and close cannot launch from old token completion.
        {
            let mut launch = DevToolsLaunch::default();
            let window = window::Id::unique();
            let (epoch, id) = launch.begin(Some(window)).unwrap();
            launch.cancel(false);
            let (new_epoch, _) = launch.begin(Some(window)).unwrap();
            assert!(new_epoch > epoch);
            assert!(!launch.take(epoch, id, Some(window)));
            launch.cancel(true);
            assert!(!launch.take(new_epoch, id, Some(window)));
            assert!(launch.begin(Some(window)).is_none());
        }
        // Launch command replaces inherited activation data without a shell.
        {
            for token in [None, Some("fresh-window-token".to_owned())] {
                let command = command(
                    std::path::Path::new("/prepared/bin/enzui-dev-tools"),
                    token.clone(),
                );
                assert_eq!(command.get_program(), "/prepared/bin/enzui-dev-tools");
                assert_eq!(command.get_args().count(), 0);
                let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
                assert_eq!(
                    env.get(std::ffi::OsStr::new("DESKTOP_STARTUP_ID")),
                    Some(&None)
                );
                assert_eq!(
                    env.get(std::ffi::OsStr::new("XDG_ACTIVATION_TOKEN")),
                    Some(&token.as_deref().map(std::ffi::OsStr::new))
                );
            }
        }
    }
}
