use super::{Message, parent::ParentLease};
use enzui_shell::arm_delay::ArmGate;
use enzui_shell::desktop_portal::{
    ApplicationChoice, PortalDialog, PortalDialogAnswer, PortalDialogKind,
};
use enzui_shell::{i18n::tr, tr};
use enzui_theme::{EnzTheme, WeightRung};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    icon::{IconSource, enz_icon},
    search_field::enz_search_field,
    sidebar_row::enz_sidebar_row,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::widget::{Space, column, container, image, row, scrollable, svg};
use iced::{Alignment, Element, Length, window};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub(super) enum Action {
    Search(String),
    Select(String),
    Rename(String),
    Accept,
}

pub(super) struct Prompt {
    pub request: PortalDialog,
    pub gate: ArmGate,
    pub parent: Option<Arc<ParentLease>>,
    pub awaiting_token: bool,
    reply: oneshot::Sender<PortalDialogAnswer>,
    query: String,
    selected: Option<String>,
}

impl Prompt {
    pub(super) fn new(request: PortalDialog, reply: oneshot::Sender<PortalDialogAnswer>) -> Self {
        let mut prompt = Self {
            request,
            reply,
            gate: ArmGate::default(),
            parent: None,
            awaiting_token: false,
            query: String::new(),
            selected: None,
        };
        prompt.select_available();
        prompt
    }

    fn select_available(&mut self) {
        let PortalDialogKind::OpenWith {
            choices,
            last_choice,
            ..
        } = &self.request.kind
        else {
            return;
        };
        if !choices
            .iter()
            .any(|choice| self.selected.as_ref() == Some(&choice.id))
        {
            self.selected = choices
                .iter()
                .find(|choice| {
                    last_choice.as_deref().is_some_and(|last| {
                        choice.id.trim_end_matches(".desktop") == last.trim_end_matches(".desktop")
                    })
                })
                .or_else(|| choices.first())
                .map(|choice| choice.id.clone());
        }
    }

    pub(super) fn replace_choices(&mut self, replacement: Vec<ApplicationChoice>) {
        if let PortalDialogKind::OpenWith { choices, .. } = &mut self.request.kind {
            *choices = replacement;
            self.select_available();
        }
    }

    pub(super) fn edit(&mut self, action: Action) {
        match action {
            Action::Search(query) => self.query = query,
            Action::Select(id) => self.selected = Some(id),
            Action::Rename(next) => {
                if let PortalDialogKind::Install {
                    name,
                    editable: true,
                    ..
                } = &mut self.request.kind
                {
                    *name = next
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(256)
                        .collect();
                }
            }
            Action::Accept => {}
        }
    }

    fn matches_query(&self, choice: &ApplicationChoice) -> bool {
        let query = self.query.to_lowercase();
        choice.name.to_lowercase().contains(&query) || choice.id.to_lowercase().contains(&query)
    }

    fn chosen(&self) -> Option<String> {
        let PortalDialogKind::OpenWith { choices, .. } = &self.request.kind else {
            return None;
        };
        let selected = self.selected.as_ref()?;
        choices
            .iter()
            .any(|choice| choice.id == *selected && self.matches_query(choice))
            .then(|| selected.clone())
    }

    pub(super) fn answer(&self, token: Option<String>) -> Option<PortalDialogAnswer> {
        match &self.request.kind {
            PortalDialogKind::OpenWith { .. } => Some(PortalDialogAnswer::Application {
                id: self.chosen()?,
                activation_token: token,
            }),
            PortalDialogKind::Install { name, .. } => {
                (!name.trim().is_empty()).then(|| PortalDialogAnswer::Install(name.clone()))
            }
            PortalDialogKind::ShareAccount { .. } => Some(PortalDialogAnswer::ShareAccount),
        }
    }

    pub(super) fn send(self, answer: PortalDialogAnswer) {
        let _ = self.reply.send(answer);
    }

    pub(super) fn title(&self) -> &'static str {
        match &self.request.kind {
            PortalDialogKind::OpenWith { .. } => tr("shell-portal-open-with"),
            PortalDialogKind::Install {
                target: Some(_), ..
            } => tr("shell-portal-install-web-app"),
            PortalDialogKind::Install { .. } => tr("shell-portal-install-app"),
            PortalDialogKind::ShareAccount { .. } => tr("shell-portal-share-account"),
        }
    }

    pub(super) fn view(&self, window: window::Id, theme: EnzTheme) -> Element<'_, Message> {
        let palette = theme.palette;
        let action = move |action| Message::Prompt(window, action);
        let label = |text: String| {
            enz_text(text)
                .wrap(true)
                .muted_on(palette.surface)
                .into_element()
        };
        let (content, accept): (Element<'_, Message>, _) = match &self.request.kind {
            PortalDialogKind::OpenWith { item, choices, .. } => {
                let mut list = column![].spacing(6).width(Length::Fill);
                let mut count = 0;
                for choice in choices.iter().filter(|choice| self.matches_query(choice)) {
                    count += 1;
                    let id = choice.id.clone();
                    let mut app_row =
                        enz_sidebar_row(choice.name.clone(), action(Action::Select(id.clone())))
                            .id(format!("portal-app-{id}").into())
                            .selected(self.selected.as_ref() == Some(&choice.id))
                            .icon("app-window");
                    if let Some(icon) = &choice.icon {
                        let source = if icon.extension().is_some_and(|extension| extension == "svg")
                        {
                            IconSource::from(svg::Handle::from_path(icon))
                        } else {
                            IconSource::from(image::Handle::from_path(icon))
                        };
                        app_row = app_row.icon_source(source);
                    }
                    list = list.push(app_row.into_element());
                }
                if count == 0 {
                    list = list.push(label(tr("shell-no-matching-applications").into()));
                }
                let search =
                    enz_search_field(&self.query, move |query| action(Action::Search(query)))
                        .placeholder(tr("shell-search-apps"))
                        .on_clear(action(Action::Search(String::new())))
                        .into_element();
                (
                    column![
                        label(item.clone()),
                        search,
                        scrollable(list).height(Length::Fill),
                    ]
                    .spacing(12)
                    .into(),
                    tr("shell-open"),
                )
            }
            PortalDialogKind::Install {
                name,
                editable,
                target,
                preview,
            } => {
                let header = if self.request.app.is_empty() {
                    tr("shell-portal-install-description").to_owned()
                } else {
                    tr!("shell-portal-install-requester", "app" => self.request.app.clone())
                };
                let name_field = if *editable {
                    enz_text_field(tr("shell-portal-launcher-name"), "", name, move |name| {
                        action(Action::Rename(name))
                    })
                    .on_submit(action(Action::Accept))
                    .into_element()
                } else {
                    enz_text(name.clone()).wrap(true).into_element()
                };
                let mut details = column![
                    label(header),
                    row![image(preview.clone()).width(72).height(72), name_field,]
                        .spacing(16)
                        .align_y(Alignment::Center),
                ]
                .spacing(14);
                if let Some(target) = target {
                    details = details.push(label(target.clone()));
                }
                (
                    scrollable(details).height(Length::Fill).into(),
                    tr("shell-install"),
                )
            }
            PortalDialogKind::ShareAccount {
                user,
                name,
                reason,
                avatar,
            } => {
                let header = if self.request.app.is_empty() {
                    tr("shell-portal-account-description").to_owned()
                } else {
                    tr!("shell-portal-account-requester", "app" => self.request.app.clone())
                };
                let account = row![
                    picture(avatar.as_deref(), 64.0, "circle-user-round"),
                    column![
                        enz_text(name.clone())
                            .weight_rung(WeightRung::Semibold)
                            .wrap(true)
                            .into_element(),
                        label(user.clone()),
                    ]
                    .spacing(4),
                ]
                .spacing(14)
                .align_y(Alignment::Center);
                let mut details = column![label(header)].spacing(12);
                if let Some(reason) = reason {
                    details = details.push(label(reason.clone()));
                }
                details = details.push(account);
                (
                    scrollable(details).height(Length::Fill).into(),
                    tr("shell-allow"),
                )
            }
        };
        container(
            column![
                enz_text(self.title())
                    .size(TextSize::Title3)
                    .weight_rung(WeightRung::Semibold)
                    .into_element(),
                content,
                row![
                    Space::new().width(Length::Fill),
                    enz_button(tr("shell-cancel"), Message::Cancel(window))
                        .kind(ButtonKind::Secondary)
                        .into_element(),
                    enz_button(accept, action(Action::Accept))
                        .enabled(
                            self.gate.armed()
                                && !self.awaiting_token
                                && self.answer(None).is_some()
                        )
                        .into_element(),
                ]
                .spacing(8),
            ]
            .spacing(14),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(24)
        .style(move |_| container::Style {
            background: Some(palette.surface.into()),
            ..Default::default()
        })
        .into()
    }
}

fn picture<'a>(path: Option<&Path>, size: f32, fallback: &str) -> Element<'a, Message> {
    match path {
        Some(path) if path.extension().is_some_and(|extension| extension == "svg") => {
            svg(svg::Handle::from_path(path))
                .width(size)
                .height(size)
                .into()
        }
        Some(path) => image(image::Handle::from_path(path))
            .width(size)
            .height(size)
            .into(),
        None => enz_icon(fallback).size(size).into_element(),
    }
}
