use super::{Message as ShellMessage, Shell, window_settings};
use enzui_shell::{i18n::tr, shortcut_map};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    modal_host::{ModalHostEvent, enz_modal_host, focus_wrap_operation},
    modal_surface::enz_modal_surface,
    scroll_view::enz_scroll_view,
    search_field::enz_search_field,
    shortcut_list::{ShortcutEntry, enz_shortcut_list},
    text::{TextSize, enz_text},
};
use iced::{
    Element, Length, Size, Task,
    widget::{Space, column, row},
    window,
};

#[derive(Default)]
pub(super) struct State {
    pub(super) window: Option<window::Id>,
    pending: bool,
    revision: u64,
    size: Size,
    bindings: Vec<shortcut_map::Binding>,
    query: String,
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Prepared(u64, Result<(Size, Vec<shortcut_map::Binding>), String>),
    Refreshed(u64, Result<Vec<shortcut_map::Binding>, String>),
    Filter(String),
    Settings,
    Host(ModalHostEvent),
}

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
                    Ok((size, bindings)) => {
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
            Message::Refreshed(revision, Ok(bindings))
                if revision == self.shortcut_overlay.revision =>
            {
                self.shortcut_overlay.bindings = bindings;
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
        let query = self.shortcut_overlay.query.trim().to_lowercase();
        let entries = self
            .shortcut_overlay
            .bindings
            .iter()
            .filter_map(|binding| {
                let badge = shortcut_map::badge(&binding.chord);
                [&binding.name, &binding.section, &badge]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&query))
                    .then(|| {
                        ShortcutEntry::new(&binding.section, badge, &binding.name)
                            .stable_key(&binding.id)
                    })
            })
            .collect::<Vec<_>>();
        let list: Element<'_, ShellMessage> = if entries.is_empty() {
            enz_text(tr("shell-shortcuts-no-matches")).into_element()
        } else {
            enz_shortcut_list(entries).into_element()
        };
        let label = tr("shell-keyboard-shortcuts");
        let search = enz_search_field(&self.shortcut_overlay.query, |query| {
            ShellMessage::ShortcutOverlay(Message::Filter(query))
        })
        .placeholder(tr("shell-shortcuts-filter"));
        let settings = enz_button(
            tr("shell-shortcuts-settings"),
            ShellMessage::ShortcutOverlay(Message::Settings),
        )
        .kind(ButtonKind::Secondary)
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
        let size = self.shortcut_overlay.size;
        let surface = enz_modal_surface(content.into())
            .width((size.width - 32.0).clamp(1.0, 640.0))
            .height((size.height - 48.0).clamp(1.0, 680.0))
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
