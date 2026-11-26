use super::{Message as ParentMessage, SettingsApp, settings_row};
use enzui_shell::{
    compositor::Compositor,
    i18n::{self, tr},
    settings::Change,
    shortcut_map::{self, CustomShortcut, CustomTarget},
};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    key_capture_chip::{KeyCapture, enz_key_capture_chip},
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::{
    Element, Length, Subscription, Task,
    keyboard::Modifiers,
    widget::{column, container, row},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DRAFT: &str = "new-custom-shortcut";
const TARGET_MENU: &str = "custom-shortcut-target";

#[derive(Default)]
pub(super) struct State {
    armed: Option<String>,
    modifiers: Modifiers,
    recording: bool,
    switching: bool,
    capture_serial: u64,
    feedback: Option<(String, String)>,
    draft: Option<(String, CustomShortcut)>,
    confirm_reset: bool,
}

#[derive(Debug, Clone)]
pub(super) enum Message {
    Capture(String, KeyCapture),
    Modifiers(Modifiers),
    Recording(bool, Result<(), String>),
    Timeout(u64),
    CancelCapture,
    Reset(String),
    ConfirmReset(bool),
    ResetAll,
    Show,
    Shown(Result<(), String>),
    Edit(Option<String>),
    Name(String),
    Command(String),
    Target(Option<String>),
    Save,
    Cancel,
    Remove(String),
}

fn event(message: Message) -> ParentMessage {
    ParentMessage::Shortcuts(message)
}

fn feedback<'a>(error: &'a str) -> Element<'a, ParentMessage> {
    enz_text(error).size(TextSize::Footnote).into_element()
}

fn button(
    label: &'static str,
    id: impl Into<iced::advanced::widget::Id>,
    message: Message,
    enabled: bool,
) -> Element<'static, ParentMessage> {
    enz_button(tr(label), event(message))
        .id(id.into())
        .kind(ButtonKind::Secondary)
        .enabled(enabled)
        .into_element()
}

impl SettingsApp {
    pub(super) fn shortcut_capture_subscription(&self) -> Subscription<ParentMessage> {
        if self.shortcuts.armed.is_none() {
            return Subscription::none();
        }
        iced::event::listen_raw(|event_data, _, _| match event_data {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(event(Message::Modifiers(modifiers)))
            }
            iced::Event::Window(
                iced::window::Event::Unfocused | iced::window::Event::CloseRequested,
            ) => Some(event(Message::CancelCapture)),
            _ => None,
        })
    }

    fn sync_recording(&mut self) -> Task<ParentMessage> {
        let on = self.shortcuts.armed.is_some();
        if on == self.shortcuts.recording || self.shortcuts.switching {
            return Task::none();
        }
        self.shortcuts.switching = true;
        Task::perform(
            async move {
                match Compositor::from_environment() {
                    Ok(compositor) => compositor.shortcut_recording(on).await,
                    Err(_) => Ok(()),
                }
            },
            move |result| event(Message::Recording(on, result)),
        )
    }

    pub(super) fn cancel_shortcut_capture(&mut self) -> Task<ParentMessage> {
        self.shortcuts.armed = None;
        self.shortcuts.modifiers = Modifiers::empty();
        self.sync_recording()
    }

    fn save_shortcut(&mut self, id: String, chord: Option<String>) -> Task<ParentMessage> {
        let mut preferences = self.settings.compositor.clone();
        match preferences.set_shortcut(&id, chord.as_deref()) {
            Ok(()) => {
                self.shortcuts.feedback = None;
                self.update(ParentMessage::Change(Change::Shortcut(id, chord)))
            }
            Err(reason) => {
                self.shortcuts.feedback = Some((id, reason));
                Task::none()
            }
        }
    }

    pub(super) fn shortcuts_message(&mut self, message: Message) -> Task<ParentMessage> {
        match message {
            Message::Show => Task::perform(
                async {
                    Compositor::from_environment()?
                        .show_shortcuts_overlay()
                        .await
                },
                |result| event(Message::Shown(result)),
            ),
            Message::Shown(result) => {
                self.error = result.err();
                Task::none()
            }
            Message::Capture(id, KeyCapture::Started) => {
                self.shortcuts.armed = Some(id);
                self.shortcuts.modifiers = Modifiers::empty();
                self.shortcuts.feedback = None;
                self.shortcuts.capture_serial = self.shortcuts.capture_serial.wrapping_add(1);
                let serial = self.shortcuts.capture_serial;
                Task::batch([
                    self.sync_recording(),
                    Task::perform(
                        async { tokio::time::sleep(Duration::from_secs(14)).await },
                        move |()| event(Message::Timeout(serial)),
                    ),
                ])
            }
            Message::Capture(id, capture) if self.shortcuts.armed.as_ref() == Some(&id) => {
                let resume = self.cancel_shortcut_capture();
                let chord = match capture {
                    KeyCapture::Cleared => String::new(),
                    KeyCapture::Candidate(key) => match shortcut_map::validity(&key) {
                        enzui_core::ShortcutValidity::Valid => {
                            shortcut_map::to_chord(&key).unwrap_or_default()
                        }
                        enzui_core::ShortcutValidity::Invalid(reason)
                        | enzui_core::ShortcutValidity::Reserved(reason) => {
                            self.shortcuts.feedback = Some((id, reason));
                            return resume;
                        }
                    },
                    KeyCapture::Cancelled | KeyCapture::Started => return resume,
                };
                if id == DRAFT {
                    if let Some((id, draft)) = &mut self.shortcuts.draft {
                        let mut candidate = draft.clone();
                        candidate.chord.clone_from(&chord);
                        let mut preferences = self.settings.compositor.clone();
                        let conflict = preferences.set_custom_shortcut(id, Some(&candidate)).err();
                        if let Some(reason) = conflict.filter(|_| !draft.name.trim().is_empty()
                            && !matches!(&draft.target, CustomTarget::Command(command) if command.trim().is_empty())) {
                            self.shortcuts.feedback = Some((DRAFT.into(), reason));
                        } else {
                            draft.chord = chord;
                        }
                    }
                    resume
                } else {
                    let save = self.save_shortcut(id, Some(chord));
                    Task::batch([resume, save])
                }
            }
            Message::Modifiers(modifiers) if self.shortcuts.armed.is_some() => {
                self.shortcuts.modifiers = modifiers;
                Task::none()
            }
            Message::Recording(on, result) => {
                self.shortcuts.switching = false;
                match result {
                    Ok(()) => {
                        self.shortcuts.recording = on;
                        self.sync_recording()
                    }
                    Err(reason) => {
                        if let Some(id) = self.shortcuts.armed.take() {
                            self.shortcuts.feedback = Some((id, reason));
                        }
                        Task::none()
                    }
                }
            }
            Message::Timeout(serial) if serial == self.shortcuts.capture_serial => {
                if let Some(id) = self.shortcuts.armed.take() {
                    self.shortcuts.feedback =
                        Some((id, tr("shell-shortcuts-capture-timeout").into()));
                }
                self.cancel_shortcut_capture()
            }
            Message::CancelCapture => self.cancel_shortcut_capture(),
            Message::Reset(id) => {
                let resume = self.cancel_shortcut_capture();
                let save = self.save_shortcut(id, None);
                Task::batch([resume, save])
            }
            Message::ConfirmReset(confirm) => {
                self.shortcuts.confirm_reset = confirm;
                Task::none()
            }
            Message::ResetAll => {
                self.shortcuts.confirm_reset = false;
                self.shortcuts.draft = None;
                let resume = self.cancel_shortcut_capture();
                let save = self.save_shortcut(String::new(), None);
                Task::batch([resume, save])
            }
            Message::Edit(existing) => {
                self.shortcuts.feedback = None;
                self.shortcuts.draft = match existing {
                    Some(id) => self
                        .settings
                        .compositor
                        .custom_shortcuts
                        .get(&id)
                        .cloned()
                        .map(|shortcut| (id, shortcut)),
                    None => {
                        let nonce = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos();
                        let id = format!("custom-{nonce:x}{:x}", std::process::id());
                        Some((
                            id,
                            CustomShortcut {
                                name: String::new(),
                                chord: String::new(),
                                target: CustomTarget::Command(String::new()),
                            },
                        ))
                    }
                };
                self.cancel_shortcut_capture()
            }
            Message::Name(name) => {
                if let Some((_, draft)) = &mut self.shortcuts.draft {
                    draft.name = name;
                }
                Task::none()
            }
            Message::Command(command) => {
                if let Some((_, draft)) = &mut self.shortcuts.draft {
                    draft.target = CustomTarget::Command(command);
                }
                Task::none()
            }
            Message::Target(action) => {
                self.menu = None;
                if let Some((_, draft)) = &mut self.shortcuts.draft {
                    draft.target = action.map_or_else(
                        || CustomTarget::Command(String::new()),
                        CustomTarget::Action,
                    );
                }
                Task::none()
            }
            Message::Save => {
                let Some((id, draft)) = self.shortcuts.draft.as_ref() else {
                    return Task::none();
                };
                let mut preferences = self.settings.compositor.clone();
                if let Err(reason) = preferences.set_custom_shortcut(id, Some(draft)) {
                    self.shortcuts.feedback = Some((DRAFT.into(), reason));
                    return Task::none();
                }
                let (id, draft) = (id.clone(), draft.clone());
                self.shortcuts.draft = None;
                self.shortcuts.feedback = None;
                let resume = self.cancel_shortcut_capture();
                let save = self.update(ParentMessage::Change(Change::CustomShortcut(
                    id,
                    Some(draft),
                )));
                Task::batch([resume, save])
            }
            Message::Cancel => {
                self.shortcuts.draft = None;
                self.shortcuts.feedback = None;
                self.cancel_shortcut_capture()
            }
            Message::Remove(id) => {
                if self
                    .shortcuts
                    .draft
                    .as_ref()
                    .is_some_and(|(editing, _)| editing == &id)
                {
                    self.shortcuts.draft = None;
                }
                self.update(ParentMessage::Change(Change::CustomShortcut(id, None)))
            }
            _ => Task::none(),
        }
    }

    fn shortcut_chip(&self, id: &str, name: &str, chord: &str) -> Element<'static, ParentMessage> {
        let armed = self.shortcuts.armed.as_deref() == Some(id);
        let prompt = if armed {
            let modifiers = self.shortcuts.modifiers;
            [
                (Modifiers::LOGO, "Super"),
                (Modifiers::CTRL, "Ctrl"),
                (Modifiers::ALT, "Alt"),
                (Modifiers::SHIFT, tr("shell-shortcuts-shift")),
            ]
            .into_iter()
            .filter_map(|(flag, label)| modifiers.contains(flag).then_some(label))
            .chain([tr("shell-shortcuts-press-keys")])
            .collect::<Vec<_>>()
            .join(" + ")
        } else {
            String::new()
        };
        let key = id.to_owned();
        enz_key_capture_chip(name, move |action| {
            event(Message::Capture(key.clone(), action))
        })
        .id(format!("capture-{id}").into())
        .armed(armed)
        .capture_hint(prompt)
        .empty_hint(shortcut_map::badge(chord))
        .invalid(
            self.shortcuts
                .feedback
                .as_ref()
                .is_some_and(|(key, _)| key == id),
        )
        .enabled(self.controls_live())
        .into()
    }

    fn shortcut_feedback(&self, id: &str) -> Option<Element<'_, ParentMessage>> {
        self.shortcuts
            .feedback
            .as_ref()
            .filter(|(key, _)| key == id)
            .map(|(_, reason)| feedback(reason))
    }

    fn custom_shortcut_editor<'a>(
        &'a self,
        draft: &'a CustomShortcut,
    ) -> Element<'a, ParentMessage> {
        let options = std::iter::once((
            tr("shell-shortcuts-command").to_owned(),
            event(Message::Target(None)),
        ))
        .chain(shortcut_map::CUSTOM_ACTIONS.iter().filter_map(|(id, _)| {
            Some((
                i18n::label(shortcut_map::action(id)?.name).to_owned(),
                event(Message::Target(Some((*id).into()))),
            ))
        }))
        .collect::<Vec<_>>();
        let selection = match &draft.target {
            CustomTarget::Command(_) => tr("shell-shortcuts-command"),
            CustomTarget::Action(id) => {
                shortcut_map::action(id).map_or("", |action| i18n::label(action.name))
            }
        };
        let width =
            super::components::value_menu_width(options.iter().map(|(label, _)| label.as_str()));
        let target = settings_row(tr("shell-shortcuts-run"))
            .accessory_width(width)
            .value_menu(
                selection,
                self.menu.as_deref() == Some(TARGET_MENU),
                ParentMessage::Menu(TARGET_MENU.into(), true),
                ParentMessage::Menu(TARGET_MENU.into(), false),
                options,
            )
            .into_element();
        let mut fields = column![
            enz_text_field(tr("shell-shortcuts-name"), "", &draft.name, |name| event(
                Message::Name(name)
            ))
            .into_element(),
            target,
        ]
        .spacing(10);
        if let CustomTarget::Command(command) = &draft.target {
            fields = fields.push(
                enz_text_field(tr("shell-shortcuts-command"), "", command, |command| {
                    event(Message::Command(command))
                })
                .into_element(),
            );
        }
        fields = fields.push(
            settings_row(tr("shell-keyboard-shortcut"))
                .control(self.shortcut_chip(DRAFT, tr("shell-keyboard-shortcut"), &draft.chord))
                .into_element(),
        );
        if let Some(error) = self.shortcut_feedback(DRAFT) {
            fields = fields.push(error);
        }
        fields
            .push(
                row![
                    button(
                        "shell-cancel",
                        "shortcut-editor-cancel",
                        Message::Cancel,
                        true
                    ),
                    enz_button(tr("shell-save"), event(Message::Save))
                        .id("shortcut-editor-save".into())
                        .enabled(self.controls_live())
                        .into_element(),
                ]
                .spacing(8),
            )
            .into()
    }

    pub(super) fn shortcuts_view(&self) -> Element<'_, ParentMessage> {
        let preferences = &self.settings.compositor;
        let live = self.controls_live();
        let palette = self.tokens().palette;
        let heading = |title: &str| {
            container(
                enz_text(title.to_owned())
                    .size(TextSize::Footnote)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .muted_on(palette.background)
                    .into_element(),
            )
            .padding([0, 4])
        };
        let reference = super::sections::group_card(
            settings_row(tr("shell-shortcuts-reference"))
                .icon("keyboard")
                .control(button(
                    "shell-shortcuts-show",
                    "shortcut-reference-show",
                    Message::Show,
                    Compositor::from_environment().is_ok(),
                ))
                .into_element(),
        );
        let mut groups = column![self.mark_setting("keyboard/shortcuts", reference)].spacing(20);
        let mut custom = column![
            settings_row(tr("shell-shortcuts-custom"))
                .control(button(
                    "shell-shortcuts-add",
                    "shortcut-custom-add",
                    Message::Edit(None),
                    live && preferences.custom_shortcuts.len() < shortcut_map::MAX_CUSTOM
                ))
                .into_element()
        ]
        .spacing(8);
        if let Some((_, draft)) = &self.shortcuts.draft {
            custom = custom.push(container(self.custom_shortcut_editor(draft)).padding(12));
        }
        for (id, shortcut) in &preferences.custom_shortcuts {
            let controls = row![
                enz_text(shortcut_map::badge(&shortcut.chord))
                    .size(TextSize::Footnote)
                    .into_element(),
                button(
                    "shell-edit",
                    format!("shortcut-edit-{id}"),
                    Message::Edit(Some(id.clone())),
                    live
                ),
                button(
                    "shell-remove",
                    format!("shortcut-remove-{id}"),
                    Message::Remove(id.clone()),
                    live
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);
            let target = match &shortcut.target {
                CustomTarget::Command(command) => command.as_str(),
                CustomTarget::Action(id) => {
                    shortcut_map::action(id).map_or("", |action| i18n::label(action.name))
                }
            };
            custom = custom.push(
                settings_row(shortcut.name.as_str())
                    .subtitle(target)
                    .control(controls)
                    .into_element(),
            );
        }
        groups = groups.push(super::sections::group_card(custom));
        if !preferences.shortcuts.is_empty() || !preferences.custom_shortcuts.is_empty() {
            let mut reset = column![
                settings_row(tr("shell-restore-every-default-shortcut"))
                    .control(button(
                        "shell-reset-all",
                        "shortcut-reset-all",
                        Message::ConfirmReset(true),
                        live && !self.shortcuts.confirm_reset
                    ))
                    .into_element()
            ];
            if self.shortcuts.confirm_reset {
                reset = reset.push(
                    container(
                        column![
                            enz_text(tr("shell-shortcuts-reset-confirm")).into_element(),
                            row![
                                button(
                                    "shell-cancel",
                                    "shortcut-reset-cancel",
                                    Message::ConfirmReset(false),
                                    true
                                ),
                                button(
                                    "shell-reset-all",
                                    "shortcut-reset-confirm",
                                    Message::ResetAll,
                                    live
                                ),
                            ]
                            .spacing(8)
                        ]
                        .spacing(8),
                    )
                    .padding(12),
                );
            }
            groups = groups.push(super::sections::group_card(reset));
        }
        let mut remaining = shortcut_map::ACTIONS;
        while let Some(action) = remaining.first() {
            let count = remaining
                .iter()
                .take_while(|other| other.section == action.section)
                .count();
            let (actions, rest) = remaining.split_at(count);
            remaining = rest;
            let mut rows = column![].spacing(2);
            for action in actions {
                let name = i18n::label(action.name);
                let chord = shortcut_map::chord(action, &preferences.shortcuts);
                let mut controls = row![self.shortcut_chip(action.id, name, chord)].spacing(8);
                if preferences.shortcuts.contains_key(action.id) {
                    controls = controls.push(button(
                        "shell-shortcuts-reset",
                        format!("shortcut-reset-{}", action.id),
                        Message::Reset(action.id.into()),
                        live,
                    ));
                }
                rows = rows.push(settings_row(name).control(controls).into_element());
                if let Some(error) = self.shortcut_feedback(action.id) {
                    rows = rows.push(container(error).padding([2, 12]));
                }
            }
            groups = groups.push(
                column![
                    heading(i18n::label(action.section)),
                    super::sections::group_card(rows)
                ]
                .spacing(8),
            );
        }
        #[cfg(test)]
        self.built_sections
            .borrow_mut()
            .insert("keyboard/shortcuts".to_owned());
        super::sections::grouped_section_for_scaled(
            self.tokens().typography.scale_factor().factor(),
            "keyboard/shortcuts",
            groups.width(Length::Fill),
        )
        .title(tr("shell-keyboard-shortcuts"))
        .description(tr("shell-shortcuts-capture-instructions"))
        .into_element()
    }
}
