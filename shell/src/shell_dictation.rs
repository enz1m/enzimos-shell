use super::{Message, Shell, window_settings};
use enzui_core::live_region::Announcement;
use enzui_shell::{compositor::ControlGeometry, i18n::tr, reveal::Attachment, settings::BarEdge};
use enzui_widgets::{
    button::enz_button,
    frame_clock::{FrameClockPhase, enz_frame_clock},
    icon::enz_icon,
    progress_bar::enz_progress_bar,
    progress_text::enz_progress_text,
    semantics_container::SemanticsContainer,
    text::{TextSize, enz_text},
};
use iced::advanced::{Layout, Renderer as _, Widget, layout, renderer, widget::Tree};
use iced::futures::SinkExt as _;
use iced::widget::{column, container, row};
use iced::{Alignment, Color, Element, Length, Rectangle, Size, Subscription, Task, mouse, window};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

const INTRO: Duration = Duration::from_millis(450);
const RESULT: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Status {
    #[default]
    Idle,
    Permission,
    Listening,
    Transcribing,
    Inserted,
    Copied,
    NoMicrophone,
    NoModel,
    NothingHeard,
    Disabled,
    Failed,
}

impl Status {
    fn parse(line: &str) -> Option<Self> {
        Some(match line {
            "idle" => Self::Idle,
            "permission" => Self::Permission,
            "listening" => Self::Listening,
            "transcribing" => Self::Transcribing,
            "inserted" => Self::Inserted,
            "copied" => Self::Copied,
            "no-microphone" => Self::NoMicrophone,
            "no-model" => Self::NoModel,
            "nothing-heard" => Self::NothingHeard,
            "disabled" => Self::Disabled,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle | Self::Permission => "",
            Self::Listening => tr("shell-dictation-listening"),
            Self::Transcribing => tr("shell-dictation-transcribing"),
            Self::Inserted => tr("shell-dictation-inserted"),
            Self::Copied => tr("shell-dictation-copied"),
            Self::NoMicrophone => tr("shell-dictation-no-microphone"),
            Self::NoModel => tr("shell-dictation-no-model"),
            Self::NothingHeard => tr("shell-dictation-nothing-heard"),
            Self::Disabled => tr("shell-dictation-disabled"),
            Self::Failed => tr("shell-dictation-failed"),
        }
    }

    fn surface(self) -> Option<Surface> {
        match self {
            Self::Idle => None,
            Self::Permission => Some(Surface::Permission),
            _ => Some(Surface::Indicator),
        }
    }

    fn finished(self) -> bool {
        self.surface() == Some(Surface::Indicator)
            && !matches!(self, Self::Listening | Self::Transcribing)
    }

    fn error(self) -> bool {
        self.finished() && !matches!(self, Self::Inserted | Self::Copied)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Surface {
    Indicator,
    Permission,
}

impl Surface {
    fn class(self) -> &'static str {
        match self {
            Self::Indicator => "dev.enzui.Dictation",
            Self::Permission => "dev.enzui.DictationPermission",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Event {
    State(Status),
    Level(u8),
    Placed(Surface, BarEdge, Result<ControlGeometry, String>),
    Reveal(window::Id),
    BeginWave(u64),
    Expire(u64),
    Hidden(window::Id),
    Permission(bool),
    PermissionSent,
}

#[derive(Default)]
pub(super) struct Dictation {
    status: Status,
    painted: Status,
    serial: u64,
    placing: bool,
    revealed: bool,
    indicator: Option<(window::Id, BarEdge, Attachment)>,
    permission: Option<window::Id>,
    permission_answer: Option<bool>,
    wave: bool,
    level: Rc<Cell<f32>>,
    phase: FrameClockPhase,
}

impl Dictation {
    pub(super) fn indicator(&self, id: window::Id) -> bool {
        self.indicator.is_some_and(|(window, ..)| window == id)
    }

    pub(super) fn permission(&self, id: window::Id) -> bool {
        self.permission == Some(id)
    }

    pub(super) fn owns(&self, id: window::Id) -> bool {
        self.indicator(id) || self.permission(id)
    }

    fn timer(&self) -> Task<Message> {
        let (delay, event) = match self.status {
            Status::Listening => (INTRO, Event::BeginWave(self.serial)),
            status if status.finished() => (RESULT, Event::Expire(self.serial)),
            _ => return Task::none(),
        };
        Task::perform(tokio::time::sleep(delay), move |()| {
            Message::Dictation(event)
        })
    }
}

fn socket() -> Option<PathBuf> {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    runtime
        .is_absolute()
        .then(|| runtime.join("enzimos-dictation/control.sock"))
}

fn updates() -> impl iced::futures::Stream<Item = Event> {
    iced::stream::channel(8, async |mut output| {
        let Some(path) = socket() else { return };
        loop {
            if let Ok(mut stream) = UnixStream::connect(&path).await
                && stream.write_all(b"subscribe\n").await.is_ok()
            {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                loop {
                    line.clear();
                    if !matches!((&mut reader).take(64).read_line(&mut line).await, Ok(n) if n > 0)
                        || !line.ends_with('\n')
                    {
                        break;
                    }
                    let line = line.trim_end();
                    let event = if let Some(status) = Status::parse(line) {
                        Event::State(status)
                    } else if let Some(level) = line
                        .strip_prefix("level ")
                        .and_then(|number| number.parse().ok())
                    {
                        Event::Level(level)
                    } else {
                        break;
                    };
                    if output.send(event).await.is_err() {
                        return;
                    }
                }
                if output.send(Event::State(Status::Idle)).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

async fn send_permission(allowed: bool) -> std::io::Result<()> {
    let path = socket().ok_or(std::io::ErrorKind::NotFound)?;
    let mut stream = UnixStream::connect(path).await?;
    stream
        .write_all(if allowed { b"allow\n" } else { b"deny\n" })
        .await
}

impl Shell {
    pub(super) fn dictation_subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(updates),
            if self.dictation.indicator.is_some() && !self.dictation.revealed {
                iced::event::listen_raw(|event, _, id| match event {
                    iced::Event::Window(window::Event::RedrawRequested(_)) => {
                        Some(Event::Reveal(id))
                    }
                    _ => None,
                })
            } else {
                Subscription::none()
            },
        ])
        .map(Message::Dictation)
    }

    pub(super) fn dictation_message(&mut self, event: Event) -> Task<Message> {
        let id = self.dictation.indicator.map(|(id, ..)| id);
        self.redraw_only = id;
        match event {
            Event::State(status) => self.dictation_state(status),
            Event::Level(level) if self.dictation.status == Status::Listening => {
                self.dictation.level.set(f32::from(level.min(100)) / 100.0);
                self.quiet = !self.dictation.wave || !self.tokens().motion.reduced_motion;
                Task::none()
            }
            Event::Placed(kind, bar, geometry) => {
                self.dictation.placing = false;
                if self.dictation.status.surface() != Some(kind) {
                    return self.place_dictation();
                }
                let geometry = match geometry {
                    Ok(geometry) => geometry,
                    Err(error) => {
                        eprintln!("enzui-shell dictation: {error}");
                        return Task::none();
                    }
                };
                let (id, open) = window::open(window_settings(
                    kind.class(),
                    geometry.width,
                    geometry.height,
                ));
                match kind {
                    Surface::Indicator => {
                        self.dictation.indicator = Some((id, bar, geometry.attachment));
                        self.dictation.revealed = false;
                    }
                    Surface::Permission => self.dictation.permission = Some(id),
                }
                open.map(Message::Opened)
            }
            Event::Reveal(id) if self.dictation.indicator(id) && !self.dictation.revealed => {
                self.dictation.revealed = true;
                self.dictation.timer()
            }
            Event::BeginWave(serial)
                if serial == self.dictation.serial
                    && self.dictation.status == Status::Listening =>
            {
                self.dictation.wave = true;
                Task::none()
            }
            Event::Expire(serial) if serial == self.dictation.serial => {
                self.dictation_state(Status::Idle)
            }
            Event::Hidden(id)
                if self.dictation.indicator(id)
                    && self.dictation.status.surface() != Some(Surface::Indicator) =>
            {
                self.dictation.indicator = None;
                window::close(id)
            }
            Event::Permission(allowed) if self.dictation.status == Status::Permission => {
                let Some(id) = self.dictation.permission else {
                    return Task::none();
                };
                self.dictation.permission_answer = Some(allowed);
                window::close(id)
            }
            _ => {
                self.quiet = true;
                Task::none()
            }
        }
    }

    fn dictation_state(&mut self, status: Status) -> Task<Message> {
        if self.dictation.status == status && !status.finished() {
            self.quiet = true;
            return Task::none();
        }
        self.dictation.status = status;
        self.dictation.serial = self.dictation.serial.wrapping_add(1);
        if status.surface() == Some(Surface::Indicator) {
            self.dictation.painted = status;
            self.dictation.wave = false;
            self.dictation.level.set(0.0);
            self.dictation.phase.set(0.0);
        }
        let timer = if self.dictation.indicator.is_some() && self.dictation.revealed {
            self.dictation.timer()
        } else {
            Task::none()
        };
        let close = self
            .dictation
            .permission
            .filter(|_| status != Status::Permission)
            .map_or_else(Task::none, window::close);
        let indicator = if status.surface() != Some(Surface::Indicator) && !self.dictation.revealed
        {
            self.dictation
                .indicator
                .take()
                .map_or_else(Task::none, |(id, ..)| window::close(id))
        } else {
            Task::none()
        };
        Task::batch([timer, close, indicator, self.place_dictation()])
    }

    pub(super) fn place_dictation(&mut self) -> Task<Message> {
        let Some(kind) = self.dictation.status.surface() else {
            return Task::none();
        };
        let exists = match kind {
            Surface::Indicator => self.dictation.indicator.is_some(),
            Surface::Permission => self.dictation.permission.is_some(),
        };
        if exists || self.dictation.placing || (kind == Surface::Indicator && !self.bar_ready) {
            return Task::none();
        }
        self.dictation.placing = true;
        let (metrics, bar, compositor) = (self.metrics, self.bar, self.compositor.clone());
        let scale = metrics.text_scale().max(1.0);
        let (width, height) = match kind {
            Surface::Indicator => (156.0 * scale, 34.0 * scale),
            Surface::Permission => (350.0 * scale, 146.0 * scale),
        };
        Task::perform(
            async move {
                compositor
                    .prepare_overlay(
                        kind.class(),
                        width,
                        height,
                        kind == Surface::Permission,
                        metrics,
                        bar,
                    )
                    .await
            },
            move |result| Message::Dictation(Event::Placed(kind, bar, result)),
        )
    }

    pub(super) fn dictation_closed(&mut self, id: window::Id) -> Task<Message> {
        if self.dictation.indicator(id) {
            self.dictation.indicator = None;
            return Task::none();
        }
        self.dictation.permission = None;
        let answer = self.dictation.permission_answer.take().unwrap_or(false);
        if self.dictation.status != Status::Permission {
            return Task::none();
        }
        Task::perform(
            async move {
                if let Err(error) = send_permission(answer).await {
                    eprintln!("enzui-shell dictation permission: {error}");
                }
            },
            |()| Message::Dictation(Event::PermissionSent),
        )
    }

    pub(super) fn dictation_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if self.dictation.permission(id) {
            return Some(self.dictation_permission_view());
        }
        let (window, bar, attachment) = self.dictation.indicator?;
        if window != id {
            return None;
        }
        let status = self.dictation.painted;
        let theme = self.tokens();
        let palette = theme.palette;
        let radius = f32::from(self.metrics.radius);
        let bottom = bar == BarEdge::Bottom;
        let content: Element<'_, Message> = match status {
            Status::Listening if self.dictation.wave && theme.motion.reduced_motion => container(
                enz_progress_bar(self.dictation.level.get())
                    .bar_height(4.0)
                    .into_element(),
            )
            .width(70)
            .into(),
            Status::Listening if self.dictation.wave => self.dictation_wave(palette.accent),
            Status::Transcribing => enz_progress_text(status.label())
                .size(TextSize::Caption)
                .backdrop(palette.surface)
                .into_element(),
            _ => {
                let (icon, color) = match status {
                    Status::Listening => ("mic", palette.accent),
                    Status::Inserted | Status::Copied => ("check", palette.tone_success_fg),
                    _ => ("circle-alert", palette.tone_error_fg),
                };
                row![
                    enz_icon(icon).size(15.0).tint(color).into_element(),
                    enz_text(status.label())
                        .size(TextSize::Caption)
                        .into_element(),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into()
            }
        };
        let announcement = if status.error() {
            Announcement::Alert
        } else {
            Announcement::Status
        };
        let status_region: Element<'_, Message> =
            SemanticsContainer::leaf(content, announcement.region(status.label()))
                .id("dictation-status".into())
                .into();
        let content = container(status_region)
            .center(Length::Fill)
            .padding(if bottom {
                iced::Padding {
                    top: attachment.join(radius),
                    ..iced::Padding::ZERO
                }
            } else {
                iced::Padding {
                    bottom: attachment.join(radius),
                    ..iced::Padding::ZERO
                }
            });
        let mut popup = self.popup(
            id,
            self.dictation.revealed && self.dictation.status.surface() == Some(Surface::Indicator),
            attachment,
            Message::Dictation(Event::Hidden(id)),
        );
        popup.bottom = bottom;
        if matches!(attachment, Attachment::Island { .. }) {
            popup.radius = 18.0 * self.metrics.text_scale().max(1.0);
        }
        popup.glass = None;
        Some(crate::region::popup(content.into(), popup))
    }

    fn dictation_wave(&self, accent: Color) -> Element<'_, Message> {
        let phase = Rc::clone(&self.dictation.phase);
        let wave = Element::new(Wave {
            accent,
            phase: Rc::clone(&phase),
            level: Rc::clone(&self.dictation.level),
        });
        enz_frame_clock(wave, phase)
            .active(self.dictation.status == Status::Listening)
            .reduced_motion(false)
            .into_element()
    }

    fn dictation_permission_view(&self) -> Element<'_, Message> {
        let palette = self.tokens().palette;
        let content = column![
            enz_text(tr("shell-dictation-permission")).into_element(),
            enz_text(tr("shell-dictation-permission-detail"))
                .size(TextSize::Caption)
                .into_element(),
            row![
                enz_button(
                    tr("shell-dictation-deny"),
                    Message::Dictation(Event::Permission(false))
                )
                .id("dictation-permission-deny".into())
                .into_element(),
                enz_button(
                    tr("shell-dictation-allow"),
                    Message::Dictation(Event::Permission(true))
                )
                .id("dictation-permission-allow".into())
                .into_element(),
            ]
            .spacing(8),
        ]
        .spacing(10);
        container(content)
            .padding(18)
            .center(Length::Fill)
            .style(move |_| container::Style {
                background: Some(palette.surface.into()),
                text_color: Some(palette.text),
                ..Default::default()
            })
            .into()
    }
}

struct Wave {
    accent: Color,
    phase: FrameClockPhase,
    level: Rc<Cell<f32>>,
}

#[derive(Default)]
struct WaveState {
    time: Cell<f32>,
    envelope: Cell<f32>,
}

impl Widget<Message, iced::Theme, iced::Renderer> for Wave {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<WaveState>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(WaveState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(59.0.into(), 23.0.into())
    }

    fn layout(&mut self, tree: &mut Tree, _: &iced::Renderer, limits: &layout::Limits) {
        tree.size = layout::atomic(limits, 59.0, 23.0);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        layout: Layout,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<WaveState>();
        let time = self.phase.get();
        let elapsed = (time - state.time.replace(time)).clamp(0.0, 0.1);
        let level = self.level.get();
        let last = state.envelope.get();
        let rate = if level > last { 24.0 } else { 11.0 };
        let envelope = level + (last - level) * (-rate * elapsed).exp();
        state.envelope.set(envelope);

        let bounds = layout.bounds();
        for index in 0..9 {
            let offset = index as f32 - 4.0;
            let pulse = 0.55 + 0.45 * (time * 12.0 - offset * 0.85).cos();
            let taper = 1.0 - offset.abs() * 0.15;
            let height = 3.0 + 20.0 * envelope * taper * pulse;
            let color = Color {
                a: 1.0 - offset.abs() * 0.11,
                ..self.accent
            };
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: bounds.x + index as f32 * 7.0,
                        y: bounds.center_y() - height / 2.0,
                        width: 3.0,
                        height,
                    },
                    border: iced::border::rounded(2),
                    ..Default::default()
                },
                color,
            );
        }
    }
}
