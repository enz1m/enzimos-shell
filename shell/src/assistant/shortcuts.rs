use enzui_shell::{
    i18n::tr,
    settings::Change,
    shortcut_map::{self, AssistantShortcut, KeyLabels},
};
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    key_capture_chip::{KeyCapture, enz_key_capture_chip},
    preferences_row::enz_preferences_row,
    text::enz_text,
};
use iced::{
    Element, Event, Length, Subscription, Task,
    event::{Interest, Status},
    keyboard::{self, Key, Modifiers, key::Physical},
    widget::{column, container, row, scrollable},
    window,
};
use std::time::Duration;

use super::{Message as Assistant, message, perform, resize};
use crate::{Message as ShellMessage, Shell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Toggle,
    Panel(AssistantShortcut),
}

impl Action {
    fn id(self) -> String {
        match self {
            Self::Toggle => "toggle".into(),
            Self::Panel(action) => format!("{action:?}").to_lowercase(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Toggle => tr("shell-assistant-shortcut-toggle"),
            Self::Panel(action) => action.label(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Capture(Action, KeyCapture),
    Key(window::Id, Key, Physical, Modifiers, Status),
    Modifiers(window::Id, Modifiers),
    Labels(KeyLabels),
    Recording(bool, Result<(), String>),
    TimedOut(u64),
    Activate(AssistantShortcut),
    Reset(Action),
}

#[derive(Default)]
pub(super) struct State {
    pub armed: Option<Action>,
    serial: u64,
    recording: bool,
    switching: bool,
    modifiers: Modifiers,
    feedback: Option<(Action, String)>,
    labels: KeyLabels,
}

pub(super) fn event(event: Message) -> ShellMessage {
    message(Assistant::Shortcut(event))
}

pub(super) fn subscription() -> Subscription<ShellMessage> {
    iced::event::listen_with_interest(Interest::KEYBOARD, |raw, status, window| match raw {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            repeat: false,
            ..
        }) => Some(event(Message::Key(
            window,
            key,
            physical_key,
            modifiers,
            status,
        ))),
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(event(Message::Modifiers(window, modifiers)))
        }
        _ => None,
    })
}

impl Shell {
    fn sync_assistant_shortcut_recording(&mut self) -> Task<ShellMessage> {
        let state = &mut self.assistant.shortcut_editor;
        let on = state.armed.is_some();
        if on == state.recording || state.switching {
            return Task::none();
        }
        state.switching = true;
        let compositor = self.compositor.clone();
        perform(async move {
            Assistant::Shortcut(Message::Recording(
                on,
                compositor.shortcut_recording(on).await,
            ))
        })
    }

    pub(super) fn cancel_assistant_shortcut(&mut self) -> Task<ShellMessage> {
        self.assistant.shortcut_editor.armed = None;
        self.assistant.shortcut_editor.modifiers = Modifiers::empty();
        self.sync_assistant_shortcut_recording()
    }

    fn save_assistant_shortcut(
        &mut self,
        action: Action,
        chord: Option<String>,
    ) -> Task<ShellMessage> {
        let change = match action {
            Action::Toggle => Change::Shortcut("assistant".into(), chord),
            Action::Panel(action) => Change::AssistantShortcut(action, chord),
        };
        let mut settings = self.desired_settings.clone();
        if let Err(error) = settings.preview(change.clone()) {
            self.assistant.shortcut_editor.feedback = Some((action, error));
            return Task::none();
        }
        self.assistant.shortcut_editor.feedback = None;
        self.save_setting(change)
    }

    pub(super) fn assistant_shortcut(&mut self, event: Message) -> Task<ShellMessage> {
        let panel = self
            .assistant
            .panel
            .as_ref()
            .filter(|pane| pane.visible)
            .map(|pane| pane.window);
        let state = &mut self.assistant.shortcut_editor;
        match event {
            Message::Capture(action, KeyCapture::Started) if self.assistant.shortcuts => {
                state.armed = Some(action);
                state.modifiers = Modifiers::empty();
                state.feedback = None;
                state.serial = state.serial.wrapping_add(1);
                let serial = state.serial;
                Task::batch([
                    self.sync_assistant_shortcut_recording(),
                    perform(async move {
                        tokio::time::sleep(Duration::from_secs(14)).await;
                        Assistant::Shortcut(Message::TimedOut(serial))
                    }),
                ])
            }
            Message::Capture(action, KeyCapture::Cancelled) if state.armed == Some(action) => {
                self.cancel_assistant_shortcut()
            }
            Message::Capture(action, KeyCapture::Cleared) if state.armed == Some(action) => {
                self.assistant_shortcut(Message::Reset(action))
            }
            Message::Labels(labels) => {
                state.labels = labels;
                Task::none()
            }
            Message::Modifiers(id, modifiers) if state.armed.is_some() && panel == Some(id) => {
                self.assistant.shortcut_editor.modifiers = modifiers;
                Task::none()
            }
            Message::Key(id, key, physical, modifiers, status) if panel == Some(id) => {
                if let Some(action) = self.assistant.shortcut_editor.armed {
                    let candidate = match action {
                        Action::Toggle => shortcut_map::capture_chord(&key, physical, modifiers),
                        Action::Panel(_) => {
                            if key == Key::Named(keyboard::key::Named::Escape)
                                && modifiers.is_empty()
                            {
                                return self.cancel_assistant_shortcut();
                            }
                            shortcut_map::assistant_chord(&key, physical, modifiers)
                        }
                    };
                    let Some(candidate) = candidate else {
                        return Task::none();
                    };
                    let resume = self.cancel_assistant_shortcut();
                    let save = match candidate {
                        Ok(chord) => self.save_assistant_shortcut(action, Some(chord)),
                        Err(error) => {
                            self.assistant.shortcut_editor.feedback = Some((action, error));
                            Task::none()
                        }
                    };
                    return Task::batch([resume, save]);
                }
                if status == Status::Ignored
                    && let Some(Ok(chord)) =
                        shortcut_map::assistant_chord(&key, physical, modifiers)
                    && let Some((&action, _)) = self
                        .desired_settings
                        .assistant_shortcuts
                        .iter()
                        .find(|(action, saved)| {
                            matches!(
                                action,
                                AssistantShortcut::Close | AssistantShortcut::FocusNext
                            ) && **saved == chord
                        })
                {
                    return self.assistant_shortcut(Message::Activate(action));
                }
                Task::none()
            }
            Message::Recording(on, result) => {
                state.switching = false;
                match result {
                    Ok(()) => {
                        state.recording = on;
                        self.sync_assistant_shortcut_recording()
                    }
                    Err(error) => {
                        if let Some(action) = state.armed.take() {
                            state.feedback = Some((action, error));
                        }
                        Task::none()
                    }
                }
            }
            Message::TimedOut(serial) if state.serial == serial && state.armed.is_some() => {
                if let Some(action) = state.armed {
                    state.feedback = Some((action, tr("shell-shortcuts-capture-timeout").into()));
                }
                self.cancel_assistant_shortcut()
            }
            Message::Activate(AssistantShortcut::Close) => {
                if let Some(id) = self.assistant.panel.as_ref().map(|pane| pane.window) {
                    self.assistant_escape(id)
                } else {
                    Task::none()
                }
            }
            Message::Activate(AssistantShortcut::FocusNext) => {
                iced::widget::operation::focus_next()
            }
            Message::Reset(action) => Task::batch([
                self.cancel_assistant_shortcut(),
                self.save_assistant_shortcut(action, None),
            ]),
            _ => Task::none(),
        }
    }

    pub(super) fn assistant_shortcuts(&self) -> Element<'_, ShellMessage> {
        let editor = &self.assistant.shortcut_editor;
        let settings = &self.desired_settings;
        let mut rows = column![].spacing(4);
        for action in
            std::iter::once(Action::Toggle).chain(AssistantShortcut::ALL.map(Action::Panel))
        {
            let (chord, custom) = match action {
                Action::Toggle => (
                    settings
                        .compositor
                        .shortcuts
                        .get("assistant")
                        .map_or("SUPER + SHIFT + A", String::as_str),
                    settings.compositor.shortcuts.contains_key("assistant"),
                ),
                Action::Panel(action) => (
                    action.chord(&settings.assistant_shortcuts),
                    settings.assistant_shortcuts.contains_key(&action),
                ),
            };
            let name = action.label();
            let armed = editor.armed == Some(action);
            let error = editor
                .feedback
                .as_ref()
                .filter(|(editing, _)| *editing == action)
                .map(|(_, error)| error);
            let hint =
                shortcut_map::modifier_labels(enzui_core::Modifier::from_iced(editor.modifiers))
                    .chain([tr("shell-shortcuts-press-keys").to_owned()])
                    .collect::<Vec<_>>()
                    .join(" + ");
            let chip = enz_key_capture_chip(name, move |capture| {
                event(Message::Capture(action, capture))
            })
            .id(format!("assistant-shortcut-{}", action.id()).into())
            .armed(armed)
            .empty_hint(editor.labels.badge(chord))
            .capture_hint(hint)
            .invalid(error.is_some());
            let mut controls = row![chip].align_y(iced::Alignment::Center).spacing(4);
            if custom {
                controls = controls.push(
                    enz_button(tr("shell-shortcuts-reset"), event(Message::Reset(action)))
                        .kind(ButtonKind::Quiet)
                        .size(ButtonSize::Sm)
                        .into_element(),
                );
            }
            rows = rows.push(enz_preferences_row(name).control(controls).into_element());
            if let Some(error) = error {
                rows = rows.push(
                    container(enz_text(error).size_px(12.0).wrap(true).into_element())
                        .padding([2, 12]),
                );
            }
        }
        let width_buttons = row![
            enz_button(
                tr("shell-assistant-narrow"),
                message(Assistant::Resize(resize::Message::Step(false)))
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
            enz_button(
                tr("shell-assistant-widen"),
                message(Assistant::Resize(resize::Message::Step(true)))
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
        ]
        .spacing(6);
        rows = rows.push(
            enz_preferences_row(tr("shell-assistant-resize-label"))
                .control(width_buttons)
                .into_element(),
        );
        scrollable(rows)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
