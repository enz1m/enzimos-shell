mod audio;
mod rescore;

use super::{Message as AssistantMessage, client, message};
use crate::{Message as ShellMessage, Shell};
use enzui_shell::i18n::tr;
use iced::{Subscription, Task, task::Handle, window};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Tap,
    Hold,
    Always,
}

impl Mode {
    pub(super) fn label(self) -> &'static str {
        tr(match self {
            Self::Tap => "shell-voice-tap",
            Self::Hold => "shell-voice-hold",
            Self::Always => "shell-voice-always",
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Toggle,
    Mode(Mode),
    Mic,
    Hold(bool),
    Key(window::Id, bool),
    Mute,
    Silence,
    Confirm,
    UseOriginal,
    Dismiss,
    Rearm(u64),
    Tick,
    Warmed(u64, bool),
    Partial(u64, String),
    Recorded(u64, Result<bool, &'static str>),
    Recognized(u64, Option<rescore::Transcript>),
    Played(u64, u64, bool),
}

fn event(voice: Message) -> ShellMessage {
    message(AssistantMessage::Voice(voice))
}

#[derive(Clone)]
struct Clip {
    session: String,
    turn: String,
    sequence: u64,
    text: String,
}

impl Clip {
    fn parse(value: &Value) -> Option<Self> {
        let text = value["text"].as_str()?;
        if text.len() > 240 || text.contains(['\n', '\r', '\0']) {
            return None;
        }
        Some(Self {
            session: value["session_id"].as_str()?.to_owned(),
            turn: value["turn_id"].as_str()?.to_owned(),
            sequence: value["sequence"].as_u64()?,
            text: text.to_owned(),
        })
    }

    fn ack(self, status: &'static str) -> Task<ShellMessage> {
        Task::future(client::request(
            "voice.ack",
            json!({"session_id": self.session, "turn_id": self.turn, "sequence": self.sequence, "status": status}),
        )).discard()
    }
}

pub(super) struct Review {
    pub(super) text: String,
    pub(super) original: Option<String>,
    session: Option<String>,
    turn: Option<String>,
}

struct Capture {
    stop: Option<oneshot::Sender<bool>>,
    _task: Handle,
    automatic: bool,
    session: Option<String>,
    turn: Option<String>,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) enabled: bool,
    pub(super) mode: Mode,
    pub(super) muted: bool,
    pub(super) error: Option<&'static str>,
    pub(super) review: Option<Review>,
    pub(super) tick: u32,
    pub(super) recording: bool,
    pub(super) recognizing: bool,
    pub(super) partial: String,
    epoch: u64,
    capture_id: u64,
    listen_after: Option<Instant>,
    capture: Option<Capture>,
    current: Option<(Clip, Handle)>,
    queued: Option<Clip>,
    warm: audio::Warm,
    warming: Option<Handle>,
}

impl State {
    pub(super) fn speaking(&self) -> bool {
        self.current.is_some()
    }

    pub(super) fn epoch(&self) -> Option<u64> {
        self.enabled.then_some(self.epoch)
    }

    fn stop_capture(&mut self) {
        self.capture_id += 1;
        self.capture = None;
        self.recording = false;
        self.recognizing = false;
        self.partial.clear();
    }

    fn finish_capture(&mut self) {
        if let Some(stop) = self
            .capture
            .as_mut()
            .and_then(|capture| capture.stop.take())
        {
            let _ = stop.send(true);
        }
        self.recording = false;
    }

    fn stop_playback(&mut self) -> Task<ShellMessage> {
        let active = self.current.take().map_or_else(Task::none, |(clip, task)| {
            task.abort();
            self.listen_after = Some(Instant::now() + Duration::from_millis(350));
            clip.ack("interrupted")
        });
        let queued = self
            .queued
            .take()
            .map_or_else(Task::none, |clip| clip.ack("discarded"));
        Task::batch([active, queued])
    }

    pub(super) fn end(&mut self) -> Task<ShellMessage> {
        self.epoch += 1;
        self.enabled = false;
        self.stop_capture();
        self.review = None;
        self.warming = None;
        self.warm = Default::default();
        self.stop_playback()
    }

    pub(super) fn turn_event(&mut self, value: &Value) -> Task<ShellMessage> {
        match value["type"].as_str() {
            Some("voice.speech") => {
                let Some(clip) = Clip::parse(value) else {
                    return Task::none();
                };
                if !self.enabled
                    || self
                        .capture
                        .as_ref()
                        .is_some_and(|capture| !capture.automatic)
                {
                    return clip.ack("discarded");
                }
                if self.current.is_some() {
                    return self
                        .queued
                        .replace(clip)
                        .map_or_else(Task::none, |old| old.ack("discarded"));
                }
                self.start_playback(clip)
            }
            Some("turn.started") if self.enabled => self.rearm(),
            Some("turn.completed") if value["status"] != "completed" => {
                Task::batch([self.stop_playback(), self.rearm()])
            }
            _ => Task::none(),
        }
    }

    pub(super) fn turn_failed(&mut self) -> Task<ShellMessage> {
        Task::batch([self.stop_playback(), self.rearm()])
    }

    pub(super) fn new_turn(&mut self) -> Task<ShellMessage> {
        self.stop_capture();
        self.review = None;
        self.stop_playback()
    }

    fn start_playback(&mut self, clip: Clip) -> Task<ShellMessage> {
        // Half-duplex recording keeps loudspeaker output away from recognition.
        self.stop_capture();
        let epoch = self.epoch;
        let sequence = clip.sequence;
        let (task, handle) = Task::perform(
            audio::play(Arc::clone(&self.warm), clip.text.clone()),
            move |result| event(Message::Played(epoch, sequence, result.is_ok())),
        )
        .abortable();
        self.current = Some((clip, handle.abort_on_drop()));
        task
    }

    fn rearm(&self) -> Task<ShellMessage> {
        let epoch = self.epoch;
        let wait = self
            .listen_after
            .and_then(|after| after.checked_duration_since(Instant::now()));
        Task::perform(
            async move {
                if let Some(wait) = wait {
                    tokio::time::sleep(wait).await;
                }
            },
            move |()| event(Message::Rearm(epoch)),
        )
    }

    pub(super) fn subscription(&self, animate: bool) -> Subscription<ShellMessage> {
        if !self.enabled {
            return Subscription::none();
        }
        Subscription::batch([
            if animate {
                iced::time::every(Duration::from_millis(40)).map(|_| event(Message::Tick))
            } else {
                Subscription::none()
            },
            if self.mode == Mode::Hold && !self.muted {
                use iced::keyboard::{Event, Key, key::Named};
                iced::event::listen_with_interest(iced::event::Interest::KEYBOARD, |raw, _, id| {
                    match raw {
                        iced::Event::Keyboard(Event::KeyPressed {
                            key: Key::Named(Named::F9),
                            modifiers,
                            ..
                        }) if modifiers.is_empty() => Some(event(Message::Key(id, true))),
                        iced::Event::Keyboard(Event::KeyReleased {
                            key: Key::Named(Named::F9),
                            ..
                        }) => Some(event(Message::Key(id, false))),
                        _ => None,
                    }
                })
            } else {
                Subscription::none()
            },
        ])
    }
}

fn addressed(text: &str) -> Option<&str> {
    let (wake, remainder) = text.split_once(char::is_whitespace)?;
    if wake
        .trim_end_matches([',', ':', '.', '!', '?'])
        .eq_ignore_ascii_case("assistant")
    {
        let text = remainder.trim();
        return (!text.is_empty()).then_some(text);
    }
    None
}

impl Shell {
    pub(super) fn assistant_voice(&mut self, action: Message) -> Task<ShellMessage> {
        if let Message::Toggle = action {
            let voice = &mut self.assistant.voice;
            if voice.enabled {
                return voice.end();
            }
            if !self
                .assistant
                .panel
                .as_ref()
                .is_some_and(|pane| pane.visible)
            {
                return Task::none();
            }
            voice.enabled = true;
            voice.muted = false;
            voice.error = None;
            voice.mode = Mode::Tap;
            let epoch = voice.epoch;
            let (task, handle) =
                Task::perform(audio::warm(Arc::clone(&voice.warm)), move |result| {
                    event(Message::Warmed(epoch, result.is_ok()))
                })
                .abortable();
            voice.warming = Some(handle.abort_on_drop());
            return task;
        }
        let voice = &mut self.assistant.voice;
        if !voice.enabled {
            return Task::none();
        }
        match action {
            Message::Mode(mode) => {
                voice.stop_capture();
                voice.mode = mode;
                voice.review = None;
                voice.error = None;
                voice.rearm()
            }
            Message::Mute => {
                voice.muted = !voice.muted;
                voice.stop_capture();
                voice.review = None;
                if voice.muted {
                    Task::none()
                } else {
                    voice.rearm()
                }
            }
            Message::Silence => Task::batch([voice.stop_playback(), voice.rearm()]),
            Message::Dismiss => {
                voice.review = None;
                voice.rearm()
            }
            Message::Confirm | Message::UseOriginal => {
                let Some(text) = voice.review.take() else {
                    return Task::none();
                };
                let same_chat = text.session == self.assistant.session;
                let active = self
                    .assistant
                    .running
                    .as_ref()
                    .and_then(|run| run.turn_id.as_ref());
                if !same_chat || active.is_some_and(|id| Some(id) != text.turn.as_ref()) {
                    return voice.rearm();
                }
                let selected = if matches!(action, Message::UseOriginal) {
                    text.original.unwrap_or(text.text)
                } else {
                    text.text
                };
                self.assistant_submit(selected, true)
            }
            Message::Mic if voice.mode == Mode::Tap && voice.recording => {
                voice.finish_capture();
                Task::none()
            }
            Message::Mic if voice.mode == Mode::Tap => self.assistant_capture(false),
            Message::Hold(pressed) if voice.mode == Mode::Hold => {
                if pressed {
                    self.assistant_capture(false)
                } else {
                    if voice.recording {
                        voice.finish_capture();
                    }
                    Task::none()
                }
            }
            Message::Key(id, pressed)
                if self
                    .assistant
                    .panel
                    .as_ref()
                    .is_some_and(|pane| pane.window == id) =>
            {
                self.assistant_voice(Message::Hold(pressed))
            }
            Message::Rearm(epoch)
                if voice.epoch == epoch
                    && voice.mode == Mode::Always
                    && voice.review.is_none()
                    && !voice.speaking() =>
            {
                self.assistant_capture(true)
            }
            Message::Warmed(epoch, ready) if voice.epoch == epoch => {
                voice.warming = None;
                if !ready {
                    voice.error = Some("shell-voice-output-error");
                }
                Task::none()
            }
            Message::Partial(id, text) if id == voice.capture_id => {
                if voice
                    .capture
                    .as_ref()
                    .is_some_and(|capture| capture.automatic)
                {
                    voice.partial = addressed(&text).unwrap_or_default().to_owned();
                } else {
                    voice.partial = text;
                }
                Task::none()
            }
            Message::Recorded(id, result) if id == voice.capture_id => {
                voice.recording = false;
                match result {
                    Ok(true) => {
                        voice.recognizing = true;
                        Task::none()
                    }
                    Ok(false) => {
                        voice.stop_capture();
                        voice.rearm()
                    }
                    Err(key) => {
                        voice.stop_capture();
                        voice.error = Some(key);
                        Task::none()
                    }
                }
            }
            Message::Recognized(id, text) if id == voice.capture_id => {
                let Some(capture) = voice.capture.take() else {
                    return Task::none();
                };
                voice.recognizing = false;
                voice.recording = false;
                voice.partial.clear();
                let Some(rescore::Transcript { text, original }) = text else {
                    voice.error = Some("shell-voice-recognition-error");
                    return Task::none();
                };
                if text.is_empty() {
                    return voice.rearm();
                }
                let same_context = capture.session == self.assistant.session
                    && capture.turn.as_deref()
                        == self
                            .assistant
                            .running
                            .as_ref()
                            .and_then(|run| run.turn_id.as_deref());
                if capture.automatic {
                    voice.review = addressed(&text).map(|text| Review {
                        text: text.to_owned(),
                        original: original.as_deref().and_then(addressed).map(str::to_owned),
                        session: capture.session,
                        turn: capture.turn,
                    });
                    return if voice.review.is_some() {
                        Task::none()
                    } else {
                        voice.rearm()
                    };
                }
                if !same_context || original.is_some() {
                    voice.review = Some(Review {
                        text,
                        original,
                        session: capture.session,
                        turn: capture.turn,
                    });
                    return Task::none();
                }
                self.assistant_submit(text, true)
            }
            Message::Played(epoch, sequence, success)
                if voice.epoch == epoch
                    && voice
                        .current
                        .as_ref()
                        .is_some_and(|(clip, _)| clip.sequence == sequence) =>
            {
                let Some((clip, _)) = voice.current.take() else {
                    return Task::none();
                };
                voice.listen_after = Some(Instant::now() + Duration::from_millis(350));
                if !success {
                    voice.error = Some("shell-voice-output-error");
                }
                let ack = clip.ack(if success { "played" } else { "interrupted" });
                let next = voice.queued.take().map_or_else(Task::none, |clip| {
                    if success {
                        voice.start_playback(clip)
                    } else {
                        clip.ack("discarded")
                    }
                });
                Task::batch([ack, next, voice.rearm()])
            }
            Message::Tick => {
                voice.tick = voice.tick.wrapping_add(1);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    fn assistant_capture(&mut self, automatic: bool) -> Task<ShellMessage> {
        let voice = &mut self.assistant.voice;
        if voice.muted
            || voice.capture.is_some()
            || self.assistant.confirming_full_send
            || self.assistant.approval_busy
            || self.assistant.steering
        {
            return Task::none();
        }
        if self
            .assistant
            .running
            .as_ref()
            .is_some_and(|run| run.turn_id.is_none())
        {
            return Task::none();
        }
        if automatic
            && (voice.speaking()
                || voice
                    .listen_after
                    .is_some_and(|until| Instant::now() < until))
        {
            return voice.rearm();
        }
        let interrupt = if automatic {
            Task::none()
        } else {
            voice.stop_playback()
        };
        let delay = voice
            .listen_after
            .and_then(|after| after.checked_duration_since(Instant::now()));
        let (stop, stopped) = oneshot::channel();
        voice.capture_id += 1;
        let id = voice.capture_id;
        voice.recording = true;
        voice.error = None;
        voice.review = None;
        voice.partial.clear();
        let previous = self
            .assistant
            .transcript
            .rows
            .iter()
            .rev()
            .find_map(|row| {
                if let super::transcript::Body::User { text, .. } = &row.body {
                    Some(text.chars().take(512).collect::<String>())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let stream = iced::stream::channel::<ShellMessage>(
            2,
            async move |mut output: iced::futures::channel::mpsc::Sender<ShellMessage>| {
                use iced::futures::SinkExt as _;
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                if let Some(socket) = audio::streaming_socket().await {
                    let result = audio::streaming(socket, automatic, stopped, |text| {
                        let mut output = output.clone();
                        async move {
                            let _ = output.send(event(Message::Partial(id, text))).await;
                        }
                    })
                    .await;
                    let text = match result {
                        Ok((text, alternatives)) => {
                            if !text.is_empty() && (!automatic || addressed(&text).is_some()) {
                                let _ = output.send(event(Message::Recorded(id, Ok(true)))).await;
                            }
                            Some(
                                rescore::select(text, alternatives, &previous, |candidate| {
                                    !automatic || addressed(candidate).is_some()
                                })
                                .await,
                            )
                        }
                        Err(_) => None,
                    };
                    let _ = output.send(event(Message::Recognized(id, text))).await;
                    return;
                }
                match audio::capture(automatic, stopped).await {
                    Ok(Some(wav)) => {
                        let _ = output.send(event(Message::Recorded(id, Ok(true)))).await;
                        let text = audio::transcribe(wav, !automatic).await.map(Into::into);
                        let _ = output.send(event(Message::Recognized(id, text))).await;
                    }
                    Ok(None) => {
                        let _ = output.send(event(Message::Recorded(id, Ok(false)))).await;
                    }
                    Err(_) => {
                        let _ = output
                            .send(event(Message::Recorded(id, Err("shell-voice-input-error"))))
                            .await;
                    }
                }
            },
        );
        let (task, handle) = Task::run(stream, std::convert::identity).abortable();
        voice.capture = Some(Capture {
            stop: Some(stop),
            _task: handle.abort_on_drop(),
            automatic,
            session: self.assistant.session.clone(),
            turn: self
                .assistant
                .running
                .as_ref()
                .and_then(|run| run.turn_id.clone()),
        });
        Task::batch([interrupt, task])
    }

    pub(super) fn finish_voice_hold(&mut self) {
        let voice = &mut self.assistant.voice;
        if voice.mode == Mode::Hold && voice.recording {
            voice.stop_capture();
        }
    }
}
