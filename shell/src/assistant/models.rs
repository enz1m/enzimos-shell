use enzui_shell::{agent_harness, i18n::tr};
use enzui_widgets::{
    icon::enz_icon,
    popover::enz_popover,
    pressable::{PressableStyle, enz_pressable},
    text::enz_text,
    text_field::enz_text_field,
    toolbar_icon::enz_toolbar_icon,
};
use iced::{
    Alignment, Element, Length, Task,
    futures::{StreamExt, stream},
    widget::{column, container, row, scrollable, space},
};

use super::{COMPOSER, Message as Assistant, message, perform};
use crate::{Message as ShellMessage, Shell};

const SEARCH: &str = "assistant-model-search";
const LIST: &str = "assistant-model-list";
const TRIGGER: &str = "assistant-model-selector";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Provider {
    id: String,
    kind: String,
    models: Vec<String>,
}

impl Provider {
    fn label(&self) -> &str {
        match self.kind.as_str() {
            "local" => tr("shell-on-device-ai"),
            "chatgpt-login" => "ChatGPT",
            _ => &self.id,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Open,
    Close,
    Loaded(u64, Result<Vec<Provider>, String>),
    Search(String),
    Provider(Option<String>),
    Choose(Option<Choice>),
}

#[derive(Default)]
pub(super) struct State {
    pub selected: Option<Choice>,
    open: bool,
    serial: u64,
    provider: Option<String>,
    query: String,
    catalogue: Option<Result<Vec<Provider>, String>>,
}

impl State {
    pub(super) fn close(&mut self) {
        self.open = false;
    }
}

async fn catalogue() -> Result<Vec<Provider>, String> {
    let (gateway, providers) = tokio::task::spawn_blocking(|| {
        let setup =
            agent_harness::summary(&agent_harness::load()?.unwrap_or_else(agent_harness::starter));
        let providers = agent_harness::chat_providers(&setup)?;
        Ok::<_, String>((setup.gateway, providers))
    })
    .await
    .map_err(|_| tr("shell-the-ai-gateway-did-not-list-its-models").to_owned())??;

    let mut providers = stream::iter(providers.into_iter().map(|(id, kind)| {
        let gateway = gateway.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let mut models = agent_harness::models(&gateway, &id)?;
                models.sort_unstable_by_key(|model| model.to_lowercase());
                models.dedup();
                Ok::<_, String>(Provider { id, kind, models })
            })
            .await
            .ok()?
            .ok()
        }
    }))
    .buffer_unordered(4)
    .filter_map(async |provider| provider)
    .collect::<Vec<_>>()
    .await;
    providers.retain(|provider| !provider.models.is_empty() || provider.kind == "local");
    providers.sort_unstable_by_key(|provider| {
        (provider.kind != "local", provider.label().to_lowercase())
    });
    Ok(providers)
}

fn event(event: Message) -> ShellMessage {
    message(Assistant::Models(event))
}

fn reset_scroll() -> Task<ShellMessage> {
    iced::widget::operation::snap_to(
        LIST,
        scrollable::RelativeOffset::START,
        iced::widget::operation::Animation::Instant,
    )
}

impl Shell {
    pub(super) fn assistant_models(&mut self, event: Message) -> Task<ShellMessage> {
        let state = &mut self.assistant.model_picker;
        match event {
            Message::Open
                if !state.open
                    && self.assistant.running.is_none()
                    && !self.assistant.approval_busy =>
            {
                state.open = true;
                state.serial = state.serial.wrapping_add(1);
                state.query.clear();
                state.provider = None;
                state.catalogue = None;
                let serial = state.serial;
                Task::batch([
                    perform(async move {
                        Assistant::Models(Message::Loaded(serial, catalogue().await))
                    }),
                    iced::widget::operation::focus(SEARCH),
                ])
            }
            Message::Close if state.open => {
                state.close();
                iced::widget::operation::focus(TRIGGER)
            }
            Message::Loaded(serial, result) if serial == state.serial && state.open => {
                state.catalogue = Some(result);
                Task::none()
            }
            Message::Search(query) if state.open => {
                state.query = query;
                reset_scroll()
            }
            Message::Provider(provider) if state.open => {
                state.provider = provider;
                Task::batch([reset_scroll(), iced::widget::operation::focus(SEARCH)])
            }
            Message::Choose(choice)
                if state.open
                    && self.assistant.running.is_none()
                    && !self.assistant.approval_busy =>
            {
                state.selected = choice;
                state.close();
                if self.assistant.session.is_some() {
                    let reset = self.assistant_message(Assistant::NewChat);
                    Task::batch([reset, self.assistant_warmup(None)])
                } else {
                    Task::batch([
                        iced::widget::operation::focus(COMPOSER),
                        self.assistant_warmup(None),
                    ])
                }
            }
            _ => Task::none(),
        }
    }

    pub(super) fn assistant_model_picker(&self) -> Element<'_, ShellMessage> {
        let assistant = &self.assistant;
        let state = &assistant.model_picker;
        let active = assistant.session.is_some();
        let selected = if active {
            assistant.model.as_str()
        } else {
            state
                .selected
                .as_ref()
                .map_or("", |choice| choice.model.as_str())
        };
        let label = if selected.is_empty() || selected == "local" {
            tr("shell-assistant-model-default")
        } else {
            selected
        };
        let trigger = enz_pressable(
            row![
                enz_text(label)
                    .size_px(12.0)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .into_element(),
                enz_icon("chevron-down").size(10.0).into_element(),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
            event(if state.open {
                Message::Close
            } else {
                Message::Open
            }),
        )
        .id(TRIGGER.into())
        .aria_label(format!("{} · {label}", tr("shell-assistant-choose-model")))
        .style(PressableStyle::Plain)
        .enabled(assistant.running.is_none() && !assistant.approval_busy)
        .into_element();
        let trigger = container(trigger).width(132);
        enz_popover(
            trigger,
            state.open.then(|| self.assistant_model_menu()),
            || event(Message::Close),
        )
        .side(enzui_core::placement::Side::Top)
        .alignment(enzui_core::placement::Alignment::End)
        .scope("assistant-model-menu".into())
        .label(tr("shell-assistant-choose-model"))
        .width((self.assistant_body_width() - 32.0).clamp(150.0, 304.0))
        .into_element()
    }

    fn assistant_model_menu(&self) -> Element<'_, ShellMessage> {
        let state = &self.assistant.model_picker;
        let mut menu = column![].spacing(6);
        if state.provider.is_some() {
            let provider = state
                .catalogue
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .and_then(|providers| {
                    providers
                        .iter()
                        .find(|provider| Some(&provider.id) == state.provider.as_ref())
                });
            menu = menu.push(
                row![
                    enz_toolbar_icon("arrow-left", event(Message::Provider(None)))
                        .aria_label(tr("shell-assistant-model-providers"))
                        .into_element(),
                    enz_text(provider.map_or("", Provider::label)).into_element(),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            );
        }
        if self.assistant.session.is_some() {
            menu = menu.push(
                enz_text(tr("shell-assistant-model-new-chat"))
                    .size_px(12.0)
                    .wrap(true)
                    .into_element(),
            );
        }
        let mut search = enz_text_field(
            "",
            tr("shell-assistant-search-models"),
            &state.query,
            |query| event(Message::Search(query)),
        )
        .id(SEARCH.into())
        .aria_label(tr("shell-assistant-search-models"));

        let mut options = Vec::new();
        match state.catalogue.as_ref() {
            Some(Ok(providers)) => {
                let query = state.query.trim().to_lowercase();
                if state.provider.is_none() && query.is_empty() {
                    options.push((
                        tr("shell-assistant-model-default").to_owned(),
                        Message::Choose(None),
                        false,
                    ));
                }
                if providers.len() > 1 && state.provider.is_none() {
                    for provider in providers {
                        if provider.label().to_lowercase().contains(&query)
                            || provider
                                .models
                                .iter()
                                .any(|model| model.to_lowercase().contains(&query))
                        {
                            options.push((
                                provider.label().to_owned(),
                                Message::Provider(Some(provider.id.clone())),
                                true,
                            ));
                        }
                    }
                } else {
                    for provider in providers.iter().filter(|provider| {
                        state.provider.as_ref().is_none_or(|id| &provider.id == id)
                    }) {
                        let provider_matches = provider.label().to_lowercase().contains(&query);
                        for model in provider.models.iter().filter(|model| {
                            provider_matches || model.to_lowercase().contains(&query)
                        }) {
                            options.push((
                                model.clone(),
                                Message::Choose(Some(Choice {
                                    provider: provider.id.clone(),
                                    model: model.clone(),
                                })),
                                false,
                            ));
                        }
                    }
                }
            }
            Some(Err(error)) => {
                return menu
                    .push(search.into_element())
                    .push(enz_text(error).size_px(12.0).wrap(true).into_element())
                    .into();
            }
            None => {
                return menu
                    .push(search.into_element())
                    .push(enz_text(tr("shell-loading")).size_px(12.0).into_element())
                    .into();
            }
        }
        if let [(_, action, _)] = options.as_slice() {
            search = search.on_submit(event(action.clone()));
        }
        menu = menu.push(search.into_element());
        if options.is_empty() {
            return menu
                .push(
                    enz_text(tr("shell-assistant-model-no-matches"))
                        .size_px(12.0)
                        .into_element(),
                )
                .into();
        }

        let height = (options.len().min(7) * 30) as f32;
        let list = options.into_iter().enumerate().fold(
            column![],
            |list, (index, (label, action, group))| {
                let caption = enz_text(label.clone())
                    .size_px(13.0)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .into_element();
                let content = row![
                    caption,
                    space().width(Length::Fill),
                    if group {
                        enz_icon("chevron-right").size(13.0).into_element()
                    } else {
                        space().into()
                    },
                ]
                .align_y(Alignment::Center);
                list.push(
                    enz_pressable(container(content).padding([0, 7]).into(), event(action))
                        .id(format!("assistant-model-option-{index}").into())
                        .aria_label(label)
                        .style(PressableStyle::Plain)
                        .width(Length::Fill)
                        .min_height(30.0)
                        .min_height_center()
                        .into_element(),
                )
            },
        );
        menu.push(scrollable(list).id(LIST).height(height)).into()
    }
}
