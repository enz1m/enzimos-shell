use super::{Message as Parent, SettingsApp};
use enzui_shell::{i18n::tr, startup_apps::Candidate, system_settings::Request};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    modal_host::ModalHostEvent,
    text::enz_text,
};
use iced::{
    Element, Length, Task,
    widget::{column, scrollable},
};

#[derive(Default)]
pub(super) struct State {
    entries: Option<Vec<Candidate>>,
    query: String,
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    Open(Vec<Candidate>),
    Query(String),
    Pick(String),
    Close,
    Host(ModalHostEvent),
}
impl State {
    pub(super) fn close(&mut self) {
        *self = Self::default();
    }
}
impl SettingsApp {
    pub(super) fn startup_message(&mut self, message: Message) -> Task<Parent> {
        match message {
            Message::Open(entries)
                if self.page == super::Page::Apps
                    && self.controls_live()
                    && self.system_writes_allowed() =>
            {
                self.startup.entries = Some(entries);
                self.startup.query.clear();
                return iced::widget::operation::focus("startup-search");
            }
            Message::Query(query) => self.startup.query = query.chars().take(256).collect(),
            Message::Pick(id)
                if self.page == super::Page::Apps
                    && self.controls_live()
                    && self.system_writes_allowed() =>
            {
                if let Some(entry) = self
                    .startup
                    .entries
                    .as_ref()
                    .and_then(|entries| entries.iter().find(|entry| entry.id == id))
                    .cloned()
                {
                    self.startup.close();
                    return self.system_message(super::system_pages::Message::Request(
                        Request::StartupApp {
                            id,
                            review: entry.review,
                            enabled: true,
                            adding: true,
                        },
                    ));
                }
            }
            Message::Close | Message::Host(ModalHostEvent::Dismissed(_)) => self.startup.close(),
            Message::Host(event) => {
                return enzui_widgets::modal_host::focus_wrap_operation(event)
                    .map_or_else(Task::none, iced::advanced::widget::operate);
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn startup_dialog(&self) -> Option<Element<'_, Parent>> {
        let entries = self.startup.entries.as_ref()?;
        let query = self.startup.query.to_lowercase();
        let mut rows = column![].spacing(4);
        for entry in entries.iter().filter(|entry| {
            query.is_empty()
                || entry.name.to_lowercase().contains(&query)
                || entry.id.to_lowercase().contains(&query)
        }) {
            rows = rows.push(
                enz_button(
                    entry.name.clone(),
                    Parent::Startup(Message::Pick(entry.id.clone())),
                )
                .icon("app-window")
                .kind(ButtonKind::Quiet)
                .width(Length::Fill)
                .id(format!("startup-choice-{}", entry.id).into())
                .into_element(),
            );
        }
        let search = enzui_widgets::search_field::enz_search_field(&self.startup.query, |query| {
            Parent::Startup(Message::Query(query))
        })
        .placeholder(tr("shell-choose-an-app"))
        .id("startup-search")
        .into_element();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![enz_text(tr("shell-add-startup-app")).into_element(), search],
            scrollable(rows)
                .height(Length::Shrink)
                .id("startup-choices"),
            vec![
                enz_button(tr("shell-cancel"), Parent::Startup(Message::Close))
                    .id("startup-cancel".into())
                    .kind(ButtonKind::Secondary)
                    .into_element(),
            ],
            12.0,
            520.0,
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(480.0)
                .padding(20.0)
                .on_dismiss(|| Parent::Startup(Message::Close))
                .close_label(tr("shell-cancel"))
                .into_element(),
        )
    }
}
