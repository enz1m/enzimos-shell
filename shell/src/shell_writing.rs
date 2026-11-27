use super::{Message, Shell, window_settings};
use enzui_shell::{
    compositor::Compositor,
    i18n::tr,
    local_ai, session_lock,
    writing_tools::{self, Action, MAX_INSTRUCTION, MAX_SELECTION, Preferences, Selection},
};
use enzui_widgets::{button::enz_button, text::enz_text, text_field::enz_text_field_owned};
use iced::{
    Element, Length, Task,
    widget::{column, container, row, scrollable},
    window,
};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const CLASS: &str = "dev.enzui.WritingTools";
const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 370.0;

#[derive(Default)]
pub(crate) struct WritingTools {
    cycle: u64,
    opening: bool,
    window: Option<window::Id>,
    selection: Option<Selection>,
    model: Option<String>,
    instruction: String,
    last_action: Option<Action>,
    generating: bool,
    result: Option<String>,
    status: Option<String>,
    delivery: Option<(u64, String)>,
}

#[derive(Clone, Debug)]
pub(crate) enum WritingMessage {
    Opened(u64, Result<Selection, String>),
    Model(u64, Result<String, String>),
    Describe(String),
    Run(Action),
    Retry,
    Result(u64, Result<String, String>),
    Replace,
    Copy,
    Copied(u64, bool, bool),
    Close,
}

impl WritingTools {
    pub(crate) fn owns(&self, id: window::Id) -> bool {
        self.window == Some(id)
    }

    fn accepts_result(&self, cycle: u64) -> bool {
        self.cycle == cycle && self.generating
    }

    fn reset(&mut self) -> Option<window::Id> {
        let id = self.window;
        *self = Self {
            cycle: self.cycle.wrapping_add(1),
            ..Self::default()
        };
        id
    }
}

impl Shell {
    pub(crate) fn open_writing_tools(&mut self) -> Task<Message> {
        if self.writing.window.is_some() || self.writing.opening {
            return self.close_writing_tools();
        }
        if session_lock::locking() || !local_ai::enabled() || !Preferences::load().enabled {
            return Task::none();
        }
        self.writing.reset();
        self.writing.opening = true;
        let cycle = self.writing.cycle;
        let compositor = self.compositor.clone();
        let (metrics, bar) = (self.metrics, self.bar);
        Task::perform(
            async move {
                let selection = capture(&compositor).await;
                compositor
                    .prepare_overlay(CLASS, WIDTH, HEIGHT, true, metrics, bar)
                    .await?;
                selection
            },
            move |result| Message::Writing(WritingMessage::Opened(cycle, result)),
        )
    }

    pub(crate) fn close_writing_tools(&mut self) -> Task<Message> {
        let cancel = self.cancel_writing_selection();
        Task::batch([
            cancel,
            self.writing.reset().map_or_else(Task::none, window::close),
        ])
    }

    fn cancel_writing_selection(&self) -> Task<Message> {
        let Some(token) = self
            .writing
            .selection
            .as_ref()
            .and_then(|selection| selection.token)
        else {
            return Task::none();
        };
        let compositor = self.compositor.clone();
        Task::future(async move {
            let _ = compositor.writing_request(&format!("cancel {token}")).await;
        })
        .discard()
    }

    pub(crate) fn writing_window(&mut self, event: &window::Event) -> Task<Message> {
        if let window::Event::Closed = event {
            let Some((token, result)) = self.writing.delivery.take() else {
                let cancel = self.cancel_writing_selection();
                self.writing.reset();
                return cancel;
            };
            self.writing.reset();
            let compositor = self.compositor.clone();
            return Task::perform(
                async move {
                    if !replace(&compositor, token, &result).await && !session_lock::locking() {
                        let message = if copy_text(&result).await {
                            tr("writing-copied-fallback")
                        } else {
                            tr("writing-copy-error")
                        };
                        enzui_shell::backup_notices::post_shell_notice(message.into()).await
                    } else {
                        Ok(())
                    }
                },
                Message::NoticePosted,
            );
        }
        if self.writing.delivery.is_none()
            && matches!(
                event,
                window::Event::Unfocused | window::Event::CloseRequested
            )
        {
            return self.close_writing_tools();
        }
        Task::none()
    }

    pub(crate) fn writing_message(&mut self, message: WritingMessage) -> Task<Message> {
        if session_lock::locking() {
            return self.close_writing_tools();
        }
        let state = &mut self.writing;
        match message {
            WritingMessage::Opened(cycle, result) if state.cycle == cycle && state.opening => {
                state.opening = false;
                match result {
                    Ok(selection) => state.selection = Some(selection),
                    Err(error) => state.status = Some(error),
                }
                let (id, open) = window::open(window_settings(CLASS, WIDTH, HEIGHT));
                state.window = Some(id);
                let models = if state.selection.is_some() {
                    Task::perform(
                        async {
                            tokio::task::spawn_blocking(|| {
                                let (default, models) = writing_tools::models()?;
                                match Preferences::load().model {
                                    Some(model) if models.contains(&model) => Ok(model),
                                    Some(_) => Err(tr("writing-model-missing").into()),
                                    None => Ok(default),
                                }
                            })
                            .await
                            .map_err(|_| tr("writing-local-unavailable"))?
                        },
                        move |result| Message::Writing(WritingMessage::Model(cycle, result)),
                    )
                } else {
                    Task::none()
                };
                Task::batch([open.map(Message::Opened), models])
            }
            WritingMessage::Model(cycle, result) if state.cycle == cycle => {
                match result {
                    Ok(model) => state.model = Some(model),
                    Err(error) => state.status = Some(error),
                }
                Task::none()
            }
            WritingMessage::Describe(value) if state.window.is_some() => {
                if value.len() <= MAX_INSTRUCTION {
                    state.instruction = value;
                }
                Task::none()
            }
            WritingMessage::Run(action) => self.run_writing_action(action),
            WritingMessage::Retry => state
                .last_action
                .clone()
                .map_or_else(Task::none, |action| self.run_writing_action(action)),
            WritingMessage::Result(cycle, result) if state.accepts_result(cycle) => {
                state.generating = false;
                match result {
                    Ok(result) => state.result = Some(result),
                    Err(error) => state.status = Some(error),
                }
                Task::none()
            }
            WritingMessage::Replace | WritingMessage::Copy => {
                let (Some(id), Some(result)) = (state.window, state.result.as_ref()) else {
                    return Task::none();
                };
                if state.delivery.is_some() {
                    return Task::none();
                }
                let replacing = matches!(message, WritingMessage::Replace);
                if replacing
                    && let Some(token) = state
                        .selection
                        .as_ref()
                        .and_then(|selection| selection.token)
                {
                    state.delivery = Some((token, result.clone()));
                    return window::close(id);
                }
                let cycle = state.cycle;
                iced::clipboard::write(result.clone()).map(move |result| {
                    Message::Writing(WritingMessage::Copied(cycle, result.is_ok(), replacing))
                })
            }
            WritingMessage::Copied(cycle, success, replacing) if state.cycle == cycle => {
                state.status = Some(
                    tr(match (success, replacing) {
                        (true, true) => "writing-copied-fallback",
                        (true, false) => "writing-copied",
                        (false, _) => "writing-copy-error",
                    })
                    .into(),
                );
                Task::none()
            }
            WritingMessage::Close => self.close_writing_tools(),
            _ => Task::none(),
        }
    }

    fn run_writing_action(&mut self, action: Action) -> Task<Message> {
        let state = &mut self.writing;
        let (Some(model), Some(selection)) = (&state.model, &state.selection) else {
            return Task::none();
        };
        if state.generating
            || state.delivery.is_some()
            || !Preferences::load().enabled
            || matches!(&action, Action::Custom(text) if text.trim().is_empty())
        {
            return Task::none();
        }
        let (model, text) = (model.clone(), selection.text.clone());
        let cycle = state.cycle;
        state.last_action = Some(action.clone());
        state.generating = true;
        state.status = None;
        state.result = None;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || writing_tools::generate(&model, &action, &text))
                    .await
                    .unwrap_or_else(|_| Err(tr("writing-generation-error").into()))
            },
            move |result| Message::Writing(WritingMessage::Result(cycle, result)),
        )
    }

    pub(crate) fn writing_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        let state = &self.writing;
        if !state.owns(id) {
            return None;
        }
        let palette = self.window_tokens(Some(id)).palette;
        let send = Message::Writing;
        let button = |label, message| enz_button(label, send(message)).into_element();
        let mut heading = row![
            enz_text(tr("writing-tools"))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            iced::widget::Space::new().width(Length::Fill),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(10);
        if let Some(model) = &state.model {
            heading = heading.push(
                enz_text(enzui_shell::tr!("writing-on-device-model", "model" => model.as_str()))
                    .into_element(),
            );
        }
        let heading = heading.push(button(tr("shell-close"), WritingMessage::Close));
        let mut body = column![heading].spacing(10);
        if state.selection.is_some() {
            let ready = state.model.is_some() && !state.generating;
            let action = |key, action| {
                enz_button(tr(key), send(WritingMessage::Run(action)))
                    .enabled(ready)
                    .into_element()
            };
            body = body.push(
                row![
                    action("writing-proofread", Action::Proofread),
                    action("writing-summarize", Action::Summarize),
                    action("writing-list", Action::List),
                ]
                .spacing(6),
            );
            body = body.push(
                row![
                    enz_text(tr("writing-rewrite")).into_element(),
                    action("writing-friendly", Action::Friendly),
                    action("writing-professional", Action::Professional),
                    action("writing-concise", Action::Concise),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            );
            body = body.push(
                row![
                    container(
                        enz_text_field_owned(
                            "",
                            tr("writing-describe"),
                            state.instruction.as_str(),
                            move |value| send(WritingMessage::Describe(value)),
                        )
                        .aria_label(tr("writing-describe"))
                        .into_element(),
                    )
                    .width(Length::Fill),
                    enz_button(
                        tr("writing-generate"),
                        send(WritingMessage::Run(Action::Custom(
                            state.instruction.clone()
                        )))
                    )
                    .enabled(ready && !state.instruction.trim().is_empty())
                    .into_element()
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            );
        }

        let preview = state.result.as_deref().or_else(|| {
            state
                .selection
                .as_ref()
                .map(|selection| selection.text.as_str())
        });
        let content = if state.generating {
            tr("writing-generating")
        } else if state.model.is_none() && state.status.is_none() {
            tr("writing-loading-model")
        } else {
            preview.unwrap_or("")
        };
        body = body.push(scrollable(enz_text(content).into_element()).height(Length::Fill));
        if state.result.is_some() {
            body = body.push(
                row![
                    button(tr("writing-replace"), WritingMessage::Replace),
                    button(tr("writing-copy"), WritingMessage::Copy),
                    button(tr("writing-retry"), WritingMessage::Retry),
                ]
                .spacing(6),
            );
        } else if state.last_action.is_some() && !state.generating {
            body = body.push(button(tr("writing-retry"), WritingMessage::Retry));
        }
        if let Some(status) = &state.status {
            body = body.push(enz_text(status.as_str()).into_element());
        }
        let content = container(body)
            .padding(enzui_shell::panel_style::INSET)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                text_color: Some(palette.text),
                ..Default::default()
            });
        Some(self.transient_glass(id, content, 22.0))
    }
}

async fn capture(compositor: &Compositor) -> Result<Selection, String> {
    let reply = compositor
        .writing_request("capture")
        .await
        .map_err(|_| tr("writing-selection-unavailable"))?;
    if let Some(token) = reply
        .strip_prefix("primary ")
        .and_then(|value| value.parse::<u64>().ok())
    {
        let result = tokio::time::timeout(Duration::from_secs(2), read_primary())
            .await
            .ok()
            .flatten()
            .ok_or_else(|| tr("writing-select-text"))?;
        let request = writing_payload(&format!("verify-primary {token} "), &result);
        let reply = compositor.writing_request(&request).await;
        match reply.as_deref() {
            Ok("editable") => {
                return Selection::new(Some(token), result)
                    .ok_or_else(|| tr("writing-select-text").into());
            }
            Ok("ok") => {
                return Selection::new(None, result)
                    .ok_or_else(|| tr("writing-select-text").into());
            }
            _ => {}
        }
    }
    Selection::from_reply(&reply).ok_or_else(|| tr("writing-select-text").into())
}

async fn read_primary() -> Option<String> {
    let mut child = tokio::process::Command::new("wl-paste")
        .args(["--primary", "--no-newline", "--type", "text"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut text = String::new();
    child
        .stdout
        .take()?
        .take(MAX_SELECTION as u64 + 1)
        .read_to_string(&mut text)
        .await
        .ok()?;
    if text.len() <= MAX_SELECTION && child.wait().await.ok()?.success() {
        Some(text)
    } else {
        None
    }
}

fn writing_payload(prefix: &str, text: &str) -> String {
    use std::fmt::Write as _;
    let mut request = String::with_capacity(prefix.len() + text.len() * 2);
    request.push_str(prefix);
    for byte in text.bytes() {
        let _ = write!(request, "{byte:02x}");
    }
    request
}

async fn replace(compositor: &Compositor, token: u64, text: &str) -> bool {
    let request = writing_payload(&format!("replace {token} "), text);
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(55)).await;
        if session_lock::locking() {
            return false;
        }
        match compositor.writing_request(&request).await.as_deref() {
            Ok("ok") => return true,
            Ok("pending") => {}
            _ => return false,
        }
    }
    let _ = compositor.writing_request(&format!("cancel {token}")).await;
    false
}

async fn copy_text(text: &str) -> bool {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut child = tokio::process::Command::new("wl-copy")
            .args(["--type", "text/plain;charset=utf-8"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(text.as_bytes()).await.ok()?;
        Some(child.wait().await.ok()?.success())
    })
    .await
    .ok()
    .flatten()
        == Some(true)
}

#[cfg(test)]
mod tests {
    use super::WritingTools;

    #[test]
    fn generator_reply_cannot_replace_a_new_popovers_preview() {
        let mut state = WritingTools {
            generating: true,
            ..WritingTools::default()
        };
        let old_cycle = state.cycle;
        assert!(state.accepts_result(old_cycle));
        state.reset();
        state.generating = true;
        assert!(state.accepts_result(state.cycle));
        assert!(!state.accepts_result(old_cycle));
    }
}
