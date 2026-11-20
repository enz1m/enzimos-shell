use super::*;
use enzui_shell::i18n::tr;
use iced::widget::column;

#[derive(Default)]
pub(super) struct ScreenReader {
    // `None` until read; then whether Orca is installed (`Some(None)` when
    // not) and running.
    state: Option<Option<bool>>,
    // Whether Settings can reach the desktop to start or stop it.
    reachable: bool,
    busy: bool,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum ScreenReaderMessage {
    Read(Option<bool>, bool),
    Toggle,
    Done(Result<(), String>, Option<bool>),
}

fn message(message: ScreenReaderMessage) -> Parent {
    Parent::System(Message::ScreenReader(message))
}

impl SettingsApp {
    pub(super) fn screen_reader_read(&mut self) -> Task<Parent> {
        if self.system.screen_reader.busy {
            return Task::none();
        }
        if self.system.backend.demo {
            // Sample state: nothing on this computer is started or stopped.
            let state = &mut self.system.screen_reader;
            state.state.get_or_insert(Some(false));
            state.reachable = true;
            return Task::none();
        }
        Task::perform(
            async {
                let reachable = enzui_shell::compositor::Compositor::from_environment().is_ok();
                let state =
                    tokio::task::spawn_blocking(enzui_shell::compositor::screen_reader_state)
                        .await
                        .ok()
                        .flatten();
                (state, reachable)
            },
            |(state, reachable)| message(ScreenReaderMessage::Read(state, reachable)),
        )
    }
    pub(super) fn screen_reader_message(&mut self, event: ScreenReaderMessage) -> Task<Parent> {
        let live = self.controls_live();
        let demo = self.system.backend.demo;
        let state = &mut self.system.screen_reader;
        match event {
            ScreenReaderMessage::Read(running, reachable) if !state.busy => {
                state.state = Some(running);
                state.reachable = reachable;
            }
            ScreenReaderMessage::Read(..) => {}
            ScreenReaderMessage::Toggle => {
                if state.busy || !state.reachable || !live {
                    return Task::none();
                }
                if demo {
                    if let Some(Some(running)) = state.state.as_mut() {
                        *running = !*running;
                    }
                    return Task::none();
                }
                let Ok(compositor) = enzui_shell::compositor::Compositor::from_environment() else {
                    state.reachable = false;
                    return Task::none();
                };
                state.busy = true;
                return Task::perform(
                    async move {
                        let result = compositor.toggle_screen_reader().await;
                        // Orca takes a moment to start or stop.
                        tokio::time::sleep(Duration::from_millis(1200)).await;
                        let state = tokio::task::spawn_blocking(
                            enzui_shell::compositor::screen_reader_state,
                        )
                        .await
                        .ok()
                        .flatten();
                        (result, state)
                    },
                    |(result, state)| message(ScreenReaderMessage::Done(result, state)),
                );
            }
            ScreenReaderMessage::Done(result, running) => {
                state.busy = false;
                state.state = Some(running);
                if let Err(error) = result {
                    self.error = Some(error);
                }
            }
        }
        Task::none()
    }
    fn screen_reader_row<'a>(&self) -> Element<'a, Parent> {
        let state = &self.system.screen_reader;
        let row = crate::settings_app::settings_row(tr("shell-screen-reader"))
            .icon("audio-lines")
            .id_prefix("control-accessibility/screen_reader")
            .subtitle_wrap(true);
        let row = match state.state {
            // Not installed: say so instead of showing a switch that cannot work.
            Some(None) => row
                .subtitle(tr("shell-install-orca-to-have-the-screen-read-aloud"))
                .into_element(),
            Some(Some(running)) => row
                .subtitle(if state.reachable {
                    tr("shell-reads-the-screen-aloud-super-alt-s-also-starts-or-stops-it-here-and-on-the")
                } else {
                    tr("shell-press-super-alt-s-to-start-or-stop-it-here-and-on-the-lock-screen")
                })
                .switch(running, |_| message(ScreenReaderMessage::Toggle))
                .enabled(state.reachable && !state.busy && self.controls_live())
                .into_element(),
            None => row
                .subtitle(tr("shell-reads-the-screen-aloud-super-alt-s-also-starts-or-stops-it-here-and-on-the"))
                .enabled(false)
                .into_element(),
        };
        self.mark_setting("accessibility/screen_reader", row)
    }
    pub(super) fn accessibility_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let mut rows = self.compositor_rows();
        let mut take = |route: &'static str| {
            rows.remove(route)
                .expect("the accessibility catalog contains each displayed row")
        };
        let seeing = column![take("windows/zoom"), take("windows/cursor_size")].spacing(2);
        let reduced = self.theme.reduced_motion();
        let motion =
            self.switch_setting("accessibility/reduce_motion", reduced, Parent::ReduceMotion);
        let typing =
            self.switch_setting("accessibility/keyboard", self.settings.osk_button, |on| {
                Parent::Change(enzui_shell::settings::Change::OskButton(on))
            });
        let menu = self.switch_setting(
            "accessibility/menu",
            self.settings.accessibility_menu,
            |on| Parent::Change(enzui_shell::settings::Change::AccessibilityMenu(on)),
        );
        let seeing = seeing.push(motion).push(self.screen_reader_row());
        let bar = column![typing, menu].spacing(2);
        // Settings that help here but live on other pages.
        let link = |route: &'static str, title: &'static str, icon: &'static str| {
            let entry =
                enzui_shell::settings_catalog::find(route).expect("a linked setting is registered");
            crate::settings_app::settings_row(tr(title))
                .icon(icon)
                .subtitle(entry.category().title())
                .action("", Parent::Reveal(route))
                .id(format!("accessibility-related-{route}").into())
                .into_element()
        };
        let related = column![
            link("sizing/text_size", "shell-text-size", "a-large-small"),
            link("appearance/style", "shell-high-contrast", "contrast"),
            link("keyboard/shortcuts", "shell-keyboard-shortcuts", "keyboard"),
        ]
        .spacing(2);
        let body = body
            .push(
                self.section_card_for("windows/zoom", seeing)
                    .title(tr("shell-seeing"))
                    .description(tr("shell-zoom-follows-the-pointer-super-alt-and-super-alt-zoom-in-and-out-super-alt-8"))
                    .into_element(),
            )
            .push(
                self.section_card_for("accessibility/keyboard", bar)
                    .title(tr("shell-bar-buttons"))
                    .description(tr("shell-reach-the-on-screen-keyboard-and-accessibility-options-from-the-bar"))
                    .into_element(),
            )
            .push(
                self.section_card(related)
                    .title(tr("shell-related-settings"))
                    .description(tr("shell-these-live-on-other-pages"))
                    .into_element(),
            );
        match self.desktop_status_note() {
            Some(note) => body.push(note),
            None => body,
        }
    }
}
