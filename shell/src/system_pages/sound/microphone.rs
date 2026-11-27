use super::audio_test::{
    self, CaptureEvent, Clip, FLOOR_DB, Meter, PlaybackEvent, TEST_CHUNKS, TEST_SECONDS,
};
use super::*;
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    progress_bar::{ProgressTone, enz_progress_bar},
};
use iced::{Length, task::Handle};
use std::path::PathBuf;
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum Event {
    Visible(bool),
    Focused(iced::window::Id, bool),
    Start,
    Stop,
    Replay,
    Discard,
    Retry,
    Capture(u64, CaptureEvent),
    Playback(u64, PlaybackEvent),
}

pub(in crate::settings_app::system_pages) struct Microphone {
    tool: Option<PathBuf>,
    pub(in crate::settings_app::system_pages) route_changing: bool,
    visible: bool,
    focused: bool,
    sequence: u64,
    session: Option<Session>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Route {
    input: u64,
    output: Option<u64>,
    muted: bool,
}

struct Capture {
    id: u64,
    record: watch::Sender<Option<u64>>,
    _work: Handle,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Recording {
        id: u64,
        chunks: usize,
    },
    Playing {
        id: u64,
        progress: f32,
        _work: Handle,
    },
}

struct Session {
    route: Route,
    capture: Option<Capture>,
    meter: Option<Meter>,
    phase: Phase,
    clip: Option<Clip>,
    capture_failed: bool,
    playback_failed: bool,
}

fn send(event: Event) -> Parent {
    Parent::System(Message::Microphone(event))
}

impl Microphone {
    pub(in crate::settings_app::system_pages) fn new(tool: Option<PathBuf>) -> Self {
        Self {
            tool,
            route_changing: false,
            visible: false,
            focused: true,
            sequence: 0,
            session: None,
        }
    }
}

impl SettingsApp {
    pub(in crate::settings_app) fn sync_microphone(&mut self) -> Task<Parent> {
        let mic = &mut self.system.microphone;
        mic.visible &=
            self.page == Category::Sound && !self.closing && self.filter.trim().is_empty();
        let route = (mic.visible && mic.focused && mic.tool.is_some())
            .then(|| selected_route(&self.system))
            .flatten();
        let mic = &mut self.system.microphone;
        if route != mic.session.as_ref().map(|session| session.route) {
            mic.session = route.map(|route| Session {
                route,
                capture: None,
                meter: None,
                phase: Phase::Idle,
                clip: None,
                capture_failed: false,
                playback_failed: false,
            });
        }
        let (Some(session), Some(tool)) = (mic.session.as_mut(), mic.tool.as_ref()) else {
            return Task::none();
        };
        if session.route.muted
            || session.capture.is_some()
            || session.capture_failed
            || matches!(session.phase, Phase::Playing { .. })
        {
            return Task::none();
        }
        mic.sequence = mic.sequence.wrapping_add(1);
        let id = mic.sequence;
        let (record, requests) = watch::channel(None);
        let (task, work) = Task::run(
            audio_test::capture(tool.clone(), session.route.input, requests),
            move |event| send(Event::Capture(id, event)),
        )
        .abortable();
        session.capture = Some(Capture {
            id,
            record,
            _work: work.abort_on_drop(),
        });
        task
    }

    pub(in crate::settings_app::system_pages) fn microphone_event(
        &mut self,
        event: Event,
    ) -> Task<Parent> {
        let mic = &mut self.system.microphone;
        match event {
            Event::Visible(visible) => mic.visible = visible,
            Event::Focused(id, focused) if self.window == Some(id) => mic.focused = focused,
            _ => {}
        }
        let (Some(session), Some(tool)) = (mic.session.as_mut(), mic.tool.as_ref()) else {
            return Task::none();
        };
        match event {
            Event::Start
                if matches!(session.phase, Phase::Idle)
                    && session.meter.is_some()
                    && session.route.output.is_some() =>
            {
                if let Some(capture) = &session.capture {
                    mic.sequence = mic.sequence.wrapping_add(1);
                    let id = mic.sequence;
                    capture.record.send_replace(Some(id));
                    session.phase = Phase::Recording { id, chunks: 0 };
                    session.clip = None;
                    session.playback_failed = false;
                }
            }
            Event::Stop => {
                if let Some(capture) = &session.capture {
                    capture.record.send_replace(None);
                }
                session.phase = Phase::Idle;
            }
            Event::Discard if matches!(session.phase, Phase::Idle) => {
                session.clip = None;
                session.playback_failed = false;
            }
            Event::Retry => session.capture_failed = false,
            Event::Capture(id, event) if session.capture.as_ref().is_some_and(|c| c.id == id) => {
                match event {
                    CaptureEvent::Meter(meter, progress) => {
                        session.meter = Some(meter);
                        if let (Phase::Recording { id, chunks }, Some((recording, done))) =
                            (&mut session.phase, progress)
                            && *id == recording
                        {
                            *chunks = done;
                        }
                    }
                    CaptureEvent::Recorded(recording, clip) if matches!(session.phase, Phase::Recording { id, .. } if id == recording) =>
                    {
                        session.clip = Some(clip);
                        session.phase = Phase::Idle;
                        return session.play(tool, &mut mic.sequence);
                    }
                    CaptureEvent::Failed => {
                        session.capture = None;
                        session.meter = None;
                        session.phase = Phase::Idle;
                        session.capture_failed = true;
                    }
                    CaptureEvent::Recorded(..) => {}
                }
            }
            Event::Replay if matches!(session.phase, Phase::Idle) => {
                return session.play(tool, &mut mic.sequence);
            }
            Event::Playback(playback, event)
                if let Phase::Playing { id, progress, .. } = &mut session.phase
                    && playback == *id =>
            {
                match event {
                    PlaybackEvent::Progress(fraction) => *progress = fraction,
                    PlaybackEvent::Finished(ok) => {
                        session.phase = Phase::Idle;
                        session.playback_failed = !ok;
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn microphone_test_view<'a>(&self, sound: &Sound) -> Element<'a, Parent> {
        let mic = &self.system.microphone;
        let session = mic.session.as_ref();
        let meter = session.and_then(|session| session.meter);
        let phase = session.map_or(&Phase::Idle, |session| &session.phase);
        let capture_failed = session.is_some_and(|s| s.capture_failed);
        let clip = session.and_then(|session| session.clip.as_ref());
        let clipping = meter.is_some_and(|meter| meter.clipping);
        let value = meter.map_or_else(
            || "—".into(),
            |meter| {
                if meter.peak_db <= FLOOR_DB {
                    tr!("shell-mic-dbfs", "level" => "≤ −60")
                } else {
                    tr!("shell-mic-dbfs", "level" => (meter.peak_db.round() as i32).to_string().replace('-', "−"))
                }
            },
        );
        let level = enz_progress_bar(meter.map_or(0.0, |meter| meter.bar))
            .label(tr("shell-mic-live-level"))
            .value_text(&value)
            .bar_height(6.0)
            .tone(if clipping {
                ProgressTone::Danger
            } else {
                ProgressTone::Accent
            })
            .into_element();
        let mut body = iced::widget::column![level].spacing(8).width(Length::Fill);
        let hint = if mic.tool.is_none() {
            "shell-mic-test-unavailable"
        } else if sound.input.is_some_and(|input| input.muted) {
            "shell-mic-input-muted"
        } else if capture_failed {
            "shell-mic-capture-failed"
        } else if session.is_some_and(|s| s.playback_failed) {
            "shell-mic-playback-failed"
        } else if clipping {
            "shell-mic-clipping"
        } else if meter.is_some_and(|meter| meter.quiet) {
            "shell-mic-too-quiet"
        } else if let Some(hint) = clip.and_then(audio_test::recorded_hint) {
            hint
        } else if matches!(phase, Phase::Playing { .. }) {
            "shell-mic-meter-paused"
        } else if session.is_some_and(|s| s.route.output.is_none()) {
            "shell-mic-select-output"
        } else if selected_route(&self.system).is_none() {
            "shell-mic-select-input"
        } else {
            "shell-mic-test-description"
        };
        body = body.push(self.caption(tr(hint)));

        match phase {
            Phase::Recording { chunks, .. } => {
                let remaining = (TEST_CHUNKS - chunks).div_ceil(TEST_CHUNKS / TEST_SECONDS);
                let label = tr!("shell-mic-recording", "seconds" => remaining.max(1) as i64);
                body = body.push(test_progress(&label, *chunks as f32 / TEST_CHUNKS as f32));
            }
            Phase::Playing { progress, .. } => {
                body = body.push(test_progress(tr("shell-mic-playing"), *progress));
            }
            Phase::Idle => {}
        }

        let mut buttons = row![].spacing(8);
        if !matches!(phase, Phase::Idle) {
            buttons = buttons.push(test_button("shell-mic-stop", Event::Stop, true));
        } else {
            if capture_failed {
                buttons = buttons.push(test_button("shell-mic-retry", Event::Retry, true));
            } else {
                let enabled = meter.is_some() && session.is_some_and(|s| s.route.output.is_some());
                buttons = buttons.push(test_button("shell-mic-test", Event::Start, enabled));
            }
            if clip.is_some() {
                buttons = buttons
                    .push(test_button("shell-mic-replay", Event::Replay, true))
                    .push(test_button("shell-mic-discard", Event::Discard, true));
            }
        }
        self.mark_setting(
            "sound/microphone_test",
            container(body.push(buttons))
                .padding([10, 12])
                .width(Length::Fill),
        )
    }
}

fn selected_route(system: &State) -> Option<Route> {
    if system.microphone.route_changing {
        return None;
    }
    let Some(Reading::Sound(sound)) = system.readings.get(&Panel::Sound) else {
        return None;
    };
    let devices = sound.devices.as_ref()?;
    let selected = |kind| {
        devices
            .nodes
            .iter()
            .find(|device| device.kind == kind && devices.selected(device))
            .map(|device| device.serial)
    };
    let muted = system
        .drafts
        .iter()
        .find_map(|request| match request {
            Request::InputMute(muted) => Some(*muted),
            _ => None,
        })
        .unwrap_or_else(|| sound.input.is_some_and(|input| input.muted));
    Some(Route {
        input: selected(Kind::Input)?,
        output: selected(Kind::Output),
        muted,
    })
}

impl Session {
    fn play(&mut self, tool: &std::path::Path, sequence: &mut u64) -> Task<Parent> {
        let (Some(output), Some(clip)) = (self.route.output, self.clip.clone()) else {
            return Task::none();
        };
        self.capture = None;
        self.meter = None;
        self.playback_failed = false;
        *sequence = sequence.wrapping_add(1);
        let id = *sequence;
        let (task, work) = Task::run(
            audio_test::playback(tool.to_owned(), output, clip),
            move |event| send(Event::Playback(id, event)),
        )
        .abortable();
        self.phase = Phase::Playing {
            id,
            progress: 0.0,
            _work: work.abort_on_drop(),
        };
        task
    }
}

fn test_progress<'a>(label: &str, fraction: f32) -> Element<'a, Parent> {
    enz_progress_bar(fraction)
        .label(label)
        .bar_height(4.0)
        .into_element()
}

fn test_button<'a>(label: &'static str, event: Event, enabled: bool) -> Element<'a, Parent> {
    let kind = if matches!(event, Event::Start | Event::Replay) {
        ButtonKind::Primary
    } else {
        ButtonKind::Quiet
    };
    enz_button(tr(label), send(event))
        .kind(kind)
        .enabled(enabled)
        .into_element()
}
