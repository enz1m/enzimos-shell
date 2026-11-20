use super::{Message as ParentMessage, SettingsApp};
use enzui_core::{
    EditableShortcutCatalog, ShortcutBinding, ShortcutKey, ShortcutOverrides, named_key_label,
    persist::Persistable,
};
use enzui_shell::i18n::{self, tr};
use enzui_shell::{settings::Change, shortcut_map};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    editable_shortcuts::{EditableShortcutsEvent, enz_editable_shortcuts},
    key_capture_chip::KeyCapture,
    text::{TextSize, enz_text},
};
use iced::{
    Element, Task,
    widget::{column, container},
};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub(super) struct State {
    catalog: EditableShortcutCatalog<&'static str>,
    // The same shortcuts split by section for display, each without its
    // section repeated under every row.
    groups: Vec<(&'static str, EditableShortcutCatalog<&'static str>)>,
    // The stored choices the catalog was built from.
    built: BTreeMap<String, String>,
    locale_generation: u64,
    armed: Option<String>,
    feedback: Option<(String, String)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            catalog: catalog(&BTreeMap::new()),
            groups: groups(&BTreeMap::new()),
            built: BTreeMap::new(),
            locale_generation: i18n::localizer().generation(),
            armed: None,
            feedback: None,
        }
    }
}

fn catalog(choices: &BTreeMap<String, String>) -> EditableShortcutCatalog<&'static str> {
    catalog_of(choices, None)
}

// The sections in the order their first shortcut appears.
fn sections() -> Vec<&'static str> {
    let mut sections: Vec<&'static str> = Vec::new();
    for action in shortcut_map::ACTIONS {
        if !sections.contains(&action.section) {
            sections.push(action.section);
        }
    }
    sections
}

fn groups(
    choices: &BTreeMap<String, String>,
) -> Vec<(&'static str, EditableShortcutCatalog<&'static str>)> {
    sections()
        .into_iter()
        .map(|section| (section, catalog_of(choices, Some(section))))
        .collect()
}

// Every shortcut, or one section's under its own heading (so its rows carry
// no section line).
fn catalog_of(
    choices: &BTreeMap<String, String>,
    section: Option<&str>,
) -> EditableShortcutCatalog<&'static str> {
    let defaults = shortcut_map::ACTIONS
        .iter()
        .filter(|action| section.is_none_or(|section| action.section == section))
        .map(|action| {
            let primary =
                shortcut_map::from_chord(action.default).expect("Default shortcuts parse");
            ShortcutBinding::new(action.id, action.id, primary)
                .with_description(i18n::label(action.name))
                .with_section(if section.is_some() {
                    ""
                } else {
                    i18n::label(action.section)
                })
        })
        .collect::<Vec<_>>();
    // All at once through the overrides' stored form, since a swap of two
    // chords cannot be made one rebind at a time.
    let hex = |text: &str| {
        text.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let mut body = String::from("enzui-shortcut-overrides-v1\n");
    for (id, shortcut) in choices
        .iter()
        .filter_map(|(id, chord)| Some((id, shortcut_map::from_chord(chord)?)))
    {
        let (kind, key) = match shortcut.key() {
            ShortcutKey::Character(key) => ("c", key.to_string()),
            ShortcutKey::Named(key) => match named_key_label(*key) {
                Some(label) => ("n", label.to_owned()),
                None => continue,
            },
        };
        let _ = writeln!(
            body,
            "override\t{}\t{}\t{kind}\t{}",
            hex(id),
            shortcut.modifiers().bits(),
            hex(&key)
        );
    }
    let overrides = ShortcutOverrides::persist_deserialize(&body).unwrap_or_default();
    EditableShortcutCatalog::from_overrides(defaults, overrides)
}

// Pauses or resumes the desktop's shortcuts, when Settings was started by
// the desktop and can reach its compositor.
fn pause(on: bool) -> Task<ParentMessage> {
    let Ok(compositor) = enzui_shell::compositor::Compositor::from_environment() else {
        return Task::none();
    };
    Task::perform(
        async move { compositor.shortcut_recording(on).await },
        |_| ParentMessage::Noop,
    )
}

impl SettingsApp {
    // Rebuilds the shortcut rows when the stored choices changed.
    pub(super) fn sync_shortcuts(&mut self) {
        if self.shortcuts.built != self.settings.compositor.shortcuts
            || self.shortcuts.locale_generation != i18n::localizer().generation()
        {
            self.shortcuts.locale_generation = i18n::localizer().generation();
            self.shortcuts.built = self.settings.compositor.shortcuts.clone();
            self.shortcuts.catalog = catalog(&self.shortcuts.built);
            self.shortcuts.groups = groups(&self.shortcuts.built);
        }
    }
    pub(super) fn shortcuts_message(
        &mut self,
        event: EditableShortcutsEvent,
    ) -> Task<ParentMessage> {
        self.sync_shortcuts();
        let recording = matches!(
            event,
            EditableShortcutsEvent::Capture {
                event: KeyCapture::Started,
                ..
            }
        );
        let pause = if recording != self.shortcuts.armed.is_some() {
            pause(recording)
        } else {
            Task::none()
        };
        let task = match event {
            EditableShortcutsEvent::Capture {
                action,
                event: KeyCapture::Started,
            } => {
                self.shortcuts.armed = Some(action);
                self.shortcuts.feedback = None;
                Task::none()
            }
            EditableShortcutsEvent::Capture {
                event: KeyCapture::Cancelled,
                ..
            } => {
                self.shortcuts.armed = None;
                Task::none()
            }
            EditableShortcutsEvent::Capture {
                action,
                event: KeyCapture::Cleared,
            }
            | EditableShortcutsEvent::Reset { action } => {
                self.shortcuts.armed = None;
                self.shortcuts.feedback = None;
                self.update(ParentMessage::Change(Change::Shortcut(action, None)))
            }
            EditableShortcutsEvent::Capture {
                action,
                event: KeyCapture::Candidate(shortcut),
            } => {
                self.shortcuts.armed = None;
                let mut candidate = catalog(&self.shortcuts.built);
                match candidate.try_rebind(&action, shortcut.clone(), shortcut_map::validity) {
                    Ok(()) => {
                        self.shortcuts.feedback = None;
                        let chord = shortcut_map::to_chord(&shortcut);
                        self.update(ParentMessage::Change(Change::Shortcut(action, chord)))
                    }
                    Err(refusal) => {
                        self.shortcuts.feedback = Some((action, refusal.message()));
                        Task::none()
                    }
                }
            }
            EditableShortcutsEvent::ResetAll => {
                self.shortcuts.armed = None;
                self.shortcuts.feedback = None;
                self.update(ParentMessage::Change(Change::Shortcut(String::new(), None)))
            }
        };
        Task::batch([pause, task])
    }
    pub(super) fn shortcuts_view(&self) -> Element<'_, ParentMessage> {
        let palette = self.tokens().palette;
        let scale = self.tokens().typography.scale_factor().factor();
        let feedback = self
            .shortcuts
            .feedback
            .as_ref()
            .map(|(action, message)| (action.as_str(), message.as_str()));
        let mut groups = column![].spacing(20);
        for (section, catalog) in &self.shortcuts.groups {
            let pane = enz_editable_shortcuts(
                catalog,
                self.shortcuts.armed.as_deref(),
                ParentMessage::Shortcuts,
            )
            .feedback(feedback)
            .heading(false)
            .into_element();
            groups = groups.push(
                column![
                    container(
                        enz_text(i18n::label(section))
                            .size(TextSize::Footnote)
                            .weight_rung(enzui_theme::WeightRung::Semibold)
                            .muted_on(palette.background)
                            .into_element()
                    )
                    .padding([0, 4]),
                    super::sections::group_card(pane),
                ]
                .spacing(8),
            );
        }
        // Only worth offering once something was changed.
        if !self.settings.compositor.shortcuts.is_empty() {
            groups = groups.push(super::sections::group_card(
                crate::settings_app::settings_row(tr("shell-restore-every-default-shortcut"))
                    .icon("rotate-ccw")
                    .control(
                        enz_button(
                            tr("shell-reset-all"),
                            ParentMessage::Shortcuts(EditableShortcutsEvent::ResetAll),
                        )
                        .kind(ButtonKind::Secondary)
                        .id("shortcuts-reset-all".into())
                        .enabled(self.controls_live())
                        .into_element(),
                    )
                    .into_element(),
            ));
        }
        #[cfg(test)]
        self.built_sections
            .borrow_mut()
            .insert("keyboard/shortcuts".to_owned());
        super::sections::grouped_section_for_scaled(
            scale,
            "keyboard/shortcuts",
            self.mark_setting("keyboard/shortcuts", groups),
        )
        .title(tr("shell-keyboard-shortcuts"))
        .description(tr("shell-choose-a-shortcut-then-press-the-new-keys"))
        .into_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each section's group holds exactly its own shortcuts, every shortcut
    /// appears once, and a group's rows do not repeat the section name.
    ///
    /// Sabotage: build the groups without the section filter.
    #[test]
    fn shortcut_groups_split_every_shortcut_by_section() {
        let mut choices = BTreeMap::new();
        choices.insert("lock".to_owned(), "SUPER + K".to_owned());
        let groups = groups(&choices);
        let mut seen = Vec::new();
        for (section, catalog) in &groups {
            for row in catalog.rows() {
                let action = shortcut_map::ACTIONS
                    .iter()
                    .find(|action| action.id == row.id)
                    .unwrap();
                assert_eq!(action.section, *section);
                assert!(row.section.is_empty());
                assert_eq!(row.overridden, row.id == "lock", "{}", row.id);
                seen.push(row.id);
            }
        }
        assert_eq!(seen.len(), shortcut_map::ACTIONS.len());
        assert_eq!(groups.first().map(|group| group.0), Some("Windows"));
    }
}
