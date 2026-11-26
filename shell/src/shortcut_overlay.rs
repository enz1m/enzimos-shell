use super::{Message as ShellMessage, Shell, window_settings};
use enzui_shell::{i18n::tr, shortcut_map};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    key_badge::enz_key_badge,
    modal_host::{ModalHostEvent, enz_modal_host, focus_wrap_operation},
    modal_surface::enz_modal_surface,
    scroll_view::enz_scroll_view,
    search_field::enz_search_field,
    text::{TextSize, enz_text},
};
use iced::{
    Alignment, Element, Length, Size, Task,
    widget::{Column, Space, column, row},
    window,
};

#[derive(Default)]
pub(super) struct State {
    pub(super) window: Option<window::Id>,
    pending: bool,
    revision: u64,
    size: Size,
    bindings: Vec<shortcut_map::Binding>,
    key_labels: shortcut_map::KeyLabels,
    query: String,
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Prepared(u64, Result<(Size, LiveBindings), String>),
    Refreshed(u64, Result<LiveBindings, String>),
    KeyLabels(shortcut_map::KeyLabels),
    Filter(String),
    Settings,
    Host(ModalHostEvent),
}

type LiveBindings = (Vec<shortcut_map::Binding>, shortcut_map::KeyLabels);

impl Shell {
    pub(super) fn shortcuts_toggle(&mut self) -> Task<ShellMessage> {
        if self.shortcut_overlay.window.is_some() || self.shortcut_overlay.pending {
            self.shortcuts_close()
        } else {
            self.shortcuts_show()
        }
    }

    pub(super) fn shortcuts_show(&mut self) -> Task<ShellMessage> {
        if self.shortcut_overlay.window.is_some() || self.shortcut_overlay.pending {
            return Task::none();
        }
        self.shortcut_overlay.pending = true;
        self.shortcut_overlay.query.clear();
        let revision = self.shortcut_overlay.revision;
        let compositor = self.compositor.clone();
        let preferences = self
            .desktop_applied
            .as_ref()
            .unwrap_or(&self.desired_settings.compositor)
            .clone();
        Task::batch([
            self.close_search(),
            self.force_close_launcher(),
            self.force_close_overview(),
            self.force_close_notifications(),
            Task::perform(
                async move {
                    let size = compositor.prepare_shortcuts_overlay().await?;
                    let entries = shortcut_map::live_bindings(&compositor, &preferences).await?;
                    Ok((size, entries))
                },
                move |result| ShellMessage::ShortcutOverlay(Message::Prepared(revision, result)),
            ),
        ])
    }

    pub(super) fn shortcuts_close(&mut self) -> Task<ShellMessage> {
        self.shortcut_overlay.pending = false;
        self.shortcut_overlay.revision = self.shortcut_overlay.revision.wrapping_add(1);
        self.shortcut_overlay
            .window
            .take()
            .map_or_else(Task::none, window::close)
    }

    pub(super) fn shortcuts_refresh(&self) -> Task<ShellMessage> {
        if self.shortcut_overlay.window.is_none() {
            return Task::none();
        }
        let revision = self.shortcut_overlay.revision;
        let compositor = self.compositor.clone();
        let preferences = self
            .desktop_applied
            .as_ref()
            .unwrap_or(&self.desired_settings.compositor)
            .clone();
        Task::perform(
            async move { shortcut_map::live_bindings(&compositor, &preferences).await },
            move |result| ShellMessage::ShortcutOverlay(Message::Refreshed(revision, result)),
        )
    }

    pub(super) fn shortcuts_overlay_message(&mut self, message: Message) -> Task<ShellMessage> {
        match message {
            Message::Prepared(revision, result) if revision == self.shortcut_overlay.revision => {
                self.shortcut_overlay.pending = false;
                match result {
                    Ok((size, (bindings, keys))) => {
                        self.shortcut_overlay.key_labels = keys;
                        self.shortcut_overlay.size = size;
                        self.shortcut_overlay.bindings = bindings;
                        let (id, open) = window::open(window_settings(
                            "dev.enzui.Shortcuts",
                            size.width,
                            size.height,
                        ));
                        self.shortcut_overlay.window = Some(id);
                        open.map(ShellMessage::Opened)
                    }
                    Err(error) => {
                        self.notice = Some(error);
                        Task::none()
                    }
                }
            }
            Message::Refreshed(revision, Ok((bindings, keys)))
                if revision == self.shortcut_overlay.revision =>
            {
                self.shortcut_overlay.key_labels = keys;
                self.shortcut_overlay.bindings = bindings;
                Task::none()
            }
            Message::KeyLabels(keys) => {
                self.shortcut_overlay.key_labels = keys;
                Task::none()
            }
            Message::Filter(query) => {
                self.shortcut_overlay.query = query;
                Task::none()
            }
            Message::Host(ModalHostEvent::Dismissed(_)) => self.shortcuts_close(),
            Message::Host(event) => {
                focus_wrap_operation(event).map_or_else(Task::none, iced::advanced::widget::operate)
            }
            Message::Settings => self
                .shortcuts_close()
                .chain(Task::done(ShellMessage::SettingsPage("keyboard/shortcuts"))),
            _ => Task::none(),
        }
    }

    pub(super) fn shortcuts_overlay_view(&self) -> Element<'_, ShellMessage> {
        let size = self.shortcut_overlay.size;
        let width = (size.width - 32.0).clamp(1.0, 1440.0);
        let list = self.shortcut_overlay.groups(width - 48.0);
        let label = tr("shell-keyboard-shortcuts");
        let search = enz_search_field(&self.shortcut_overlay.query, |query| {
            ShellMessage::ShortcutOverlay(Message::Filter(query))
        })
        .placeholder(tr("shell-shortcuts-filter"))
        .id("shortcuts-overlay-filter");
        let settings = enz_button(
            tr("shell-shortcuts-settings"),
            ShellMessage::ShortcutOverlay(Message::Settings),
        )
        .kind(ButtonKind::Secondary)
        .id("shortcuts-overlay-settings".into())
        .into_element();
        let content = column![
            enz_text(label)
                .size(TextSize::Title2)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            search.into_element(),
            enz_scroll_view(list).height(Length::Fill).into_element(),
            row![Space::new().width(Length::Fill), settings],
        ]
        .spacing(16);
        let surface = enz_modal_surface(content.into())
            .width(width)
            .height((size.height - 48.0).clamp(1.0, 880.0))
            .padding(24.0)
            .on_dismiss(|| {
                ShellMessage::ShortcutOverlay(Message::Host(ModalHostEvent::Dismissed(
                    enzui_core::modal::DismissCause::CloseButton,
                )))
            })
            .into_element();
        enz_modal_host(
            Space::new().width(Length::Fill).height(Length::Fill),
            Some(surface),
            |event| ShellMessage::ShortcutOverlay(Message::Host(event)),
        )
        .label(label)
        .into_element()
    }
}

impl State {
    fn groups(&self, width: f32) -> Element<'_, ShellMessage> {
        const GAP: f32 = 24.0;
        let columns = if width >= 3.0 * 325.0 + 2.0 * GAP {
            3
        } else if width >= 2.0 * 325.0 + GAP {
            2
        } else {
            1
        };
        let query = self.query.trim().to_lowercase();
        let mut groups = Vec::new();
        for bindings in self.bindings.chunk_by(|a, b| a.section == b.section) {
            let section = &bindings[0].section;
            let matches_section = section.to_lowercase().contains(&query);
            let entries = bindings
                .iter()
                .filter_map(|binding| {
                    let caps = self.key_labels.caps(&binding.chord);
                    (matches_section
                        || binding.name.to_lowercase().contains(&query)
                        || caps.join("+").to_lowercase().contains(&query)
                        || caps.join(" + ").to_lowercase().contains(&query))
                    .then_some((binding, caps))
                })
                .collect::<Vec<_>>();
            if !entries.is_empty() {
                groups.push((section, entries));
            }
        }
        if groups.is_empty() {
            return enz_text(tr("shell-shortcuts-no-matches")).into_element();
        }

        let mut placement: Vec<_> = (0..groups.len()).collect();
        if columns > 1 {
            placement.sort_by_key(|&index| std::cmp::Reverse(groups[index].1.len()));
        }
        let mut assigned = vec![Vec::new(); columns];
        let mut heights = vec![0; columns];
        for index in placement {
            let column = (0..columns)
                .min_by_key(|&column| heights[column])
                .unwrap_or(0);
            assigned[column].push(index);
            heights[column] += groups[index].1.len() + 2;
        }
        for column in &mut assigned {
            column.sort_unstable();
        }
        assigned.sort_by_key(|column| column.first().copied().unwrap_or(usize::MAX));

        row(assigned.into_iter().map(|column| {
            Column::with_children(column.into_iter().map(|index| {
                let (section, entries) = &groups[index];
                let heading = enz_text(section.as_str())
                    .size(TextSize::Footnote)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element();
                let rows = entries.iter().map(|(binding, keys)| {
                    let caps = row(keys.iter().map(|key| {
                        enz_key_badge(key.clone())
                            .height(20.0)
                            .padding(5.0)
                            .into_element()
                    }))
                    .spacing(3);
                    row![
                        enz_text(binding.name.as_str())
                            .size(TextSize::Footnote)
                            .width(Length::Fill)
                            .into_element(),
                        caps,
                    ]
                    .align_y(Alignment::Center)
                    .spacing(6)
                    .into()
                });
                Column::with_children(std::iter::once(heading).chain(rows))
                    .spacing(2)
                    .into()
            }))
            .width(Length::FillPortion(1))
            .spacing(12)
            .into()
        }))
        .width(Length::Fill)
        .spacing(GAP)
        .into()
    }
}
