#[path = "ui_controls.rs"]
mod controls;
use controls::*;

use enzui_shell::i18n::{self, tr};
use enzui_shell::session_preferences::{LockLayout, LockMaterial};
use iced::{
    Alignment, Border, Color, Element, Length, Shadow, Vector,
    widget::{Space, button, column, container, image, row, stack, text},
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Noop,
    Submit,
    PreviousUser,
    NextUser,
    NextSession,
    PowerOff,
    Restart,
    Sleep,
    TogglePower,
    ToggleKeyboard,
    Osk(Osk),
    SwitchUser,
}

/// A key of the on-screen keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Osk {
    Text(String),
    Backspace,
    Shift,
    Symbols,
}

/// What keyboard focus can rest on, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Field,
    PreviousUser,
    NextUser,
    Keyboard,
    Session,
    Restart,
    PowerOff,
    Sleep,
    PowerMenu,
    SwitchUser,
}

impl Target {
    // The message activating the target (Enter or Space).
    pub(crate) fn message(self) -> Message {
        match self {
            Self::Field => Message::Submit,
            Self::PreviousUser => Message::PreviousUser,
            Self::NextUser => Message::NextUser,
            Self::Keyboard => Message::ToggleKeyboard,
            Self::Session => Message::NextSession,
            Self::Restart => Message::Restart,
            Self::Sleep => Message::Sleep,
            Self::PowerMenu => Message::TogglePower,
            Self::PowerOff => Message::PowerOff,
            Self::SwitchUser => Message::SwitchUser,
        }
    }
}

/// What the greeter adds to the lock screen.
pub(crate) struct Greeter {
    pub users: Vec<crate::accounts::User>,
    pub user: usize,
    pub sessions: Vec<crate::accounts::Session>,
    pub session: usize,
    /// The question PAM asked, and whether the answer is secret; `None`
    /// while greetd has not asked yet.
    pub prompt: Option<(String, bool)>,
    pub notice: Option<String>,
    pub power: bool,
    /// The chosen account is already signed in here: its locked session is
    /// brought back instead of a new sign-in.
    pub existing: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Checking,
    Leaving(Instant),
}

pub(crate) struct Ui {
    pub started: Instant,
    pub phase: Phase,
    pub password: String,
    pub error: Option<(String, Instant)>,
    /// What the fingerprint reader asks for, while it waits beside the password.
    pub fingerprint: Option<String>,
    pub caps_lock: bool,
    // Eased 0..1 value of the prompt being shown, and when input last came.
    focus: f32,
    pub last_input: Option<Instant>,
    pub name: String,
    pub clock_24h: bool,
    pub reduced_motion: bool,
    pub greeter: Option<Greeter>,
    /// The desktop stays live under the lock, which fades itself instead.
    pub xray: bool,
    pub target: Target,
    /// The focus ring shows once the keyboard moved focus, not for pointers.
    pub focus_visible: bool,
    /// The on-screen keyboard, and its shift and symbol layers.
    pub keyboard: bool,
    pub power_menu: bool,
    power_reveal: f32,
    keyboard_reveal: f32,
    motion_targets: (f32, f32, f32),
    pub clock: enzui_shell::session_preferences::LockClock,
    pub shift: bool,
    pub symbols: bool,
    /// The OS can open a greeter for another user (lock screen only).
    pub can_switch: bool,
}

const INTRO: Duration = Duration::from_millis(520);
// Delay wallpaper sharpening until the desktop crossfade finishes.
const FOCUS_PULL: Duration = Duration::from_millis(1100);
const OUTRO: Duration = Duration::from_millis(520);
const SHAKE: Duration = Duration::from_millis(420);
pub(crate) const PROMPT_IDLE: Duration = Duration::from_secs(20);

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}
fn fraction(since: Instant, length: Duration, now: Instant) -> f32 {
    (now.saturating_duration_since(since).as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
}

/// Per-output images, uploaded once.
#[derive(Clone, Debug)]
pub(crate) struct Images {
    pub desktop: Option<image::Handle>,
    pub wallpaper: image::Handle,
    pub soft: image::Handle,
    pub has_clock: bool,
    pub has_date: bool,
    pub light: bool,
}

/// Black for the dim: opaque, and as an alpha ramp from top to bottom.
fn shade() -> &'static (image::Handle, image::Handle) {
    static SHADE: std::sync::OnceLock<(image::Handle, image::Handle)> = std::sync::OnceLock::new();
    SHADE.get_or_init(|| {
        const STEPS: u32 = 256;
        // Each texel holds the ramp at its centre.
        let ramp = (0..STEPS)
            .flat_map(|row| [0, 0, 0, (((row * 2 + 1) * 255 + STEPS) / (STEPS * 2)) as u8])
            .collect::<Vec<_>>();
        (
            image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
            image::Handle::from_rgba(1, STEPS, ramp),
        )
    })
}

impl Ui {
    #[cfg(feature = "greeter-preview")]
    pub(crate) fn preview_focus(&self) -> f32 {
        self.focus
    }
    pub(crate) fn new(name: String, clock_24h: bool) -> Self {
        Self {
            started: Instant::now(),
            phase: Phase::Idle,
            password: String::new(),
            error: None,
            fingerprint: None,
            caps_lock: false,
            focus: 0.0,
            last_input: None,
            name,
            clock_24h,
            reduced_motion: false,
            greeter: None,
            xray: false,
            target: Target::Field,
            focus_visible: false,
            keyboard: false,
            power_menu: false,
            power_reveal: 0.0,
            keyboard_reveal: 0.0,
            motion_targets: (0.0, 0.0, 0.0),
            clock: Default::default(),
            shift: false,
            symbols: false,
            can_switch: false,
        }
    }
    // The focusable targets present now, in Tab order.
    pub(crate) fn targets(&self) -> Vec<Target> {
        if self.power_menu {
            return vec![
                Target::PowerMenu,
                Target::Sleep,
                Target::Restart,
                Target::PowerOff,
            ];
        }
        let mut targets = vec![Target::Field];
        if self.greeter.as_ref().is_some_and(|g| g.users.len() > 1) {
            targets.extend([Target::PreviousUser, Target::NextUser]);
        }
        targets.push(Target::Keyboard);
        if let Some(greeter) = &self.greeter {
            if greeter.sessions.len() > 1 {
                targets.push(Target::Session);
            }
            if greeter.power {
                targets.push(Target::PowerMenu);
            }
        } else if self.can_switch {
            targets.push(Target::SwitchUser);
        }
        targets
    }
    pub(crate) fn move_focus(&mut self, forward: bool) {
        let targets = self.targets();
        let at = targets.iter().position(|t| *t == self.target).unwrap_or(0);
        let next = if forward {
            (at + 1) % targets.len()
        } else {
            (at + targets.len() - 1) % targets.len()
        };
        self.target = targets[next];
        self.focus_visible = true;
    }
    // Handles the lock's own controls; anything else is returned for the
    // caller (sign-in, users, sessions, power).
    pub(crate) fn local(&mut self, message: Message) -> Option<Message> {
        match message {
            Message::Noop => return None,
            Message::TogglePower => {
                self.power_menu = !self.power_menu;
                self.target = if self.power_menu {
                    Target::Sleep
                } else {
                    Target::PowerMenu
                };
            }
            Message::PowerOff | Message::Restart | Message::Sleep => {
                self.power_menu = false;
                self.target = Target::PowerMenu;
                return Some(message);
            }
            Message::ToggleKeyboard => {
                self.keyboard = !self.keyboard;
                self.shift = false;
                self.symbols = false;
            }
            Message::Osk(key) if self.phase == Phase::Idle => match key {
                Osk::Text(text) if self.password.chars().count() < 256 => {
                    self.password.push_str(&text);
                    self.error = None;
                    self.shift = false;
                }
                Osk::Text(_) => {}
                Osk::Backspace => {
                    self.password.pop();
                }
                Osk::Shift => self.shift = !self.shift,
                Osk::Symbols => {
                    self.symbols = !self.symbols;
                    self.shift = false;
                }
            },
            Message::Osk(_) => {}
            other => return Some(other),
        }
        self.last_input = Some(Instant::now());
        None
    }
    // What a screen reader announces: the latest error, PAM message or
    // Caps Lock warning.
    pub(crate) fn status(&self) -> String {
        let notice = self.greeter.as_ref().and_then(|g| g.notice.clone());
        match (&self.error, self.caps_lock, notice) {
            (Some((message, _)), _, _) => message.clone(),
            (None, _, Some(notice)) => notice,
            (None, true, None) => tr("shell-caps-lock-is-on").to_owned(),
            _ => self.fingerprint.clone().unwrap_or_default(),
        }
    }
    // The question the password field answers.
    pub(crate) fn question(&self) -> (String, bool) {
        match &self.greeter {
            Some(Greeter {
                existing: Some(_), ..
            }) => (
                tr("shell-press-enter-to-return-to-this-session").to_owned(),
                false,
            ),
            Some(Greeter {
                prompt: Some((question, secret)),
                ..
            }) => (placeholder(question), *secret),
            Some(Greeter { prompt: None, .. }) => (tr("shell-enter-password").to_owned(), true),
            None => (tr("shell-enter-password").to_owned(), true),
        }
    }
    pub(crate) fn answer_ready(&self) -> bool {
        !self.password.is_empty()
            || self
                .greeter
                .as_ref()
                .is_some_and(|g| g.users.get(g.user).is_some() || g.existing.is_some())
    }
    pub(crate) fn session_name(&self) -> Option<&str> {
        let greeter = self.greeter.as_ref()?;
        greeter
            .sessions
            .get(greeter.session)
            .map(|s| s.name.as_str())
    }
    pub(crate) fn display_name(&self) -> &str {
        self.greeter
            .as_ref()
            .and_then(|g| g.users.get(g.user))
            .map_or(self.name.as_str(), |u| u.display.as_str())
    }
    pub(crate) fn prompt_wanted(&self, now: Instant) -> bool {
        !self.password.is_empty()
            || self.keyboard
            || self.power_menu
            || (self.focus_visible && self.target != Target::Field)
            || self.phase == Phase::Checking
            || self
                .last_input
                .is_some_and(|t| now.saturating_duration_since(t) < PROMPT_IDLE)
    }
    // Advances eased values; returns whether another frame is needed.
    pub(crate) fn tick(&mut self, now: Instant, dt: f32) -> bool {
        let before = (self.focus, self.power_reveal, self.keyboard_reveal);
        let target = if self.prompt_wanted(now) { 1.0 } else { 0.0 };
        let step = if target != self.motion_targets.0 {
            dt.min(1.0 / 60.0)
        } else {
            dt
        } / 0.64;
        self.focus = if self.focus < target {
            (self.focus + step).min(target)
        } else {
            (self.focus - step).max(target)
        };
        let power_target = if self.power_menu { 1.0 } else { 0.0 };
        let keyboard_target = if self.keyboard { 1.0 } else { 0.0 };
        let power_dt = if power_target != self.motion_targets.1 {
            dt.min(1.0 / 60.0)
        } else {
            dt
        };
        let keyboard_dt = if keyboard_target != self.motion_targets.2 {
            dt.min(1.0 / 60.0)
        } else {
            dt
        };
        self.power_reveal +=
            (power_target - self.power_reveal).clamp(-power_dt / 0.22, power_dt / 0.22);
        self.keyboard_reveal +=
            (keyboard_target - self.keyboard_reveal).clamp(-keyboard_dt / 0.35, keyboard_dt / 0.35);
        if self.reduced_motion {
            self.focus = target;
            self.power_reveal = power_target;
            self.keyboard_reveal = keyboard_target;
        }
        self.motion_targets = (target, power_target, keyboard_target);
        let controls_moving = (power_target - self.power_reveal).abs() > f32::EPSILON
            || (keyboard_target - self.keyboard_reveal).abs() > f32::EPSILON;
        let intro = now.saturating_duration_since(self.started) < FOCUS_PULL;
        let shaking = self
            .error
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) < SHAKE);
        let leaving = matches!(self.phase, Phase::Leaving(_));
        before != (self.focus, self.power_reveal, self.keyboard_reveal)
            || intro
            || controls_moving
            || shaking
            || leaving
            || (self.focus - target).abs() > f32::EPSILON
            || self.phase == Phase::Checking
    }
    fn outro_duration(&self) -> Duration {
        if self.greeter.is_some() {
            Duration::from_millis(180)
        } else {
            OUTRO
        }
    }
    pub(crate) fn outro_done(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Leaving(at) if now.saturating_duration_since(at) >= self.outro_duration())
    }

    /// Whether frames drawn at `now` hide the desktop completely.
    pub(crate) fn covers(&self, now: Instant) -> bool {
        !self.xray || now.saturating_duration_since(self.started) >= INTRO
    }

    pub(crate) fn view<'a>(
        &'a self,
        images: &'a Images,
        primary: bool,
        size: iced::Size,
        now: Instant,
    ) -> Element<'a, Message> {
        i18n::localizer().view(|| self.localized_view(images, primary, size, now))
    }
    fn localized_view<'a>(
        &'a self,
        images: &'a Images,
        primary: bool,
        size: iced::Size,
        now: Instant,
    ) -> Element<'a, Message> {
        // Generic sans-serif at Medium weight otherwise matches an arbitrary face.
        enzui_widgets::sans_serif_fallback::ensure_sans_serif_fallback();
        let intro = ease_out(fraction(self.started, INTRO, now));
        let outro = match self.phase {
            Phase::Leaving(at) => ease_in_out(fraction(at, self.outro_duration(), now)),
            _ => 0.0,
        };
        let focus = ease_in_out(self.focus) * (1.0 - outro);
        let content = intro * (1.0 - outro);
        let pull = ease_in_out(fraction(self.started, FOCUS_PULL, now));
        let softness = (0.60 * focus)
            .max(1.0 - pull)
            .max(outro.min(1.0) * 1.6)
            .min(1.0);

        // How much of the lock covers the desktop: it arrives over the
        // desktop and leaves by softening first, then letting it resolve.
        let cover = (1.0 - intro).max(((outro - 0.35) / 0.65).clamp(0.0, 1.0));
        let presence = if self.xray { 1.0 - cover } else { 1.0 };
        // Sharp under soft at these opacities composites to `presence` of
        // the blend between them.
        let soft_alpha = presence * softness;
        let sharp_alpha = if soft_alpha >= 0.999 {
            0.0
        } else {
            presence * (1.0 - softness) / (1.0 - soft_alpha)
        };
        let mut layers = stack![
            image(images.wallpaper.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Cover)
                .opacity(sharp_alpha)
        ];
        layers = layers.push(
            image(images.soft.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Cover)
                .opacity(soft_alpha),
        );
        // Legibility: a light overall dim, deepening towards the prompt.
        let dim = (0.12 * content + 0.18 * focus) * presence;
        let content = content * if self.xray { presence } else { 1.0 };
        // Stretched textures rather than a gradient quad, which costs several
        // times as much on integrated GPUs and is redrawn on every frame of
        // the rise.
        let (solid, ramp) = shade();
        let bottom = 0.22 * content;
        if dim > 0.001 {
            layers = layers.push(
                image(solid.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Fill)
                    .opacity(dim),
            );
        }
        if bottom > 0.001 {
            // Over the uniform dim, the ramp composes to `dim + bottom * y`.
            layers = layers.push(
                image(ramp.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Fill)
                    .opacity((bottom / (1.0 - dim).max(0.01)).min(1.0)),
            );
        }

        let keyboard = ease_in_out(self.keyboard_reveal);
        let compact = primary && (self.keyboard_reveal > 0.001 || size.height < 420.0);
        if !images.has_clock || !images.has_date {
            let mut clock_prefs = self.clock.clone();
            clock_prefs.show_time &= !images.has_clock;
            clock_prefs.show_date &= !images.has_date;
            let (left, top, width) =
                crate::clock_widgets::geometry(size, &self.clock, focus, keyboard);
            let alpha =
                if size.width < 680.0 && (size.height < 420.0 || self.clock.stacked(size.width)) {
                    content * (1.0 - keyboard)
                } else {
                    content
                };
            let clock = crate::clock_widgets::view(
                &clock_prefs,
                self.clock_24h,
                width,
                focus,
                alpha,
                self.clock.stacked(size.width),
                images.light,
            );
            layers = layers.push(
                container(clock)
                    .padding(
                        iced::Padding::ZERO
                            .left(left)
                            .top(top + 18.0 * (1.0 - intro)),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        }
        if primary {
            let keys = OskSize::fitting(size);
            let bottom = if compact { 72.0 } else { size.height * 0.09 };
            let reveal = ease_out(((self.focus - 0.40) / 0.60).clamp(0.0, 1.0)) * (1.0 - outro);
            let side = size.width >= 680.0 && !compact;
            let center = match self.clock.layout {
                LockLayout::Split if side => size.width * (0.5 - 0.25 * focus),
                LockLayout::Corner if side => size.width * (0.5 + 0.22 * focus),
                _ => size.width * 0.5,
            };
            let prompt = container(self.prompt(reveal, content, now, compact, keys, images.light))
                .center_x(if side { 340.0 } else { size.width });
            let prompt = container(prompt).padding(iced::Padding::ZERO.left(if side {
                (center - 170.0).max(0.0)
            } else {
                0.0
            }));
            let bottom =
                if side && matches!(self.clock.layout, LockLayout::Split | LockLayout::Corner) {
                    bottom + (size.height * 0.26 - bottom) * focus
                } else {
                    bottom
                };
            let ui = column![
                Space::new().height(Length::Fill),
                prompt,
                Space::new().height(bottom + 24.0 * (1.0 - reveal))
            ]
            .height(Length::Fill);
            if reveal > 0.001 {
                layers = layers.push(ui);
            }
            // At rest the lock says how to wake it; the hint gives way to the
            // prompt as it rises.
            let hint = (1.0 - reveal * 2.0).clamp(0.0, 1.0) * content;
            if self.greeter.is_none() && hint > 0.001 {
                layers = layers.push(
                    column![
                        Space::new().height(Length::Fill),
                        container(
                            text(tr("shell-lock-press-any-key"))
                                .size(14)
                                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.8 * hint)),
                        )
                        .center_x(Length::Fill),
                        Space::new().height(bottom + 8.0),
                    ]
                    .height(Length::Fill),
                );
            }
        }
        if primary && let Some(corners) = self.corners(content) {
            layers = layers.push(corners);
        }

        if primary && self.power_reveal > 0.001 {
            layers = layers.push(self.power_popup(content, size));
        }

        // The desktop as it was at lock time: covers everything while the
        // lock arrives and returns while it leaves.
        if let Some(desktop) = images.desktop.clone()
            && cover > 0.001
        {
            layers = layers.push(
                image(desktop)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Fill)
                    .opacity(cover),
            );
        }
        layers.into()
    }

    fn prompt<'a>(
        &'a self,
        focus: f32,
        _content: f32,
        now: Instant,
        compact: bool,
        keys: OskSize,
        light: bool,
    ) -> Element<'a, Message> {
        let shake = if self.reduced_motion {
            0.0
        } else {
            self.error.as_ref().map_or(0.0, |(_, at)| {
                let t = fraction(*at, SHAKE, now);
                (t * 34.0).sin() * 10.0 * (1.0 - t)
            })
        };
        let alpha = focus;
        let white = |a: f32| Color::from_rgba(1.0, 1.0, 1.0, a);
        let plain = |a| {
            if light {
                Color::from_rgba(0.025, 0.035, 0.05, a)
            } else {
                white(a)
            }
        };
        let name = self.display_name();
        let avatar_side = if compact { 40.0 } else { 76.0 };
        let username = self
            .greeter
            .as_ref()
            .and_then(|g| g.users.get(g.user))
            .map(|u| u.name.as_str());
        let avatar = image(crate::avatars::get(username, name))
            .width(avatar_side)
            .height(avatar_side)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(avatar_side * 0.5)
            .opacity(alpha);
        let checking = self.phase == Phase::Checking;
        let (question, secret) = self.question();
        let ring = |target: Target| self.focus_visible && self.target == target;
        let field_text = if self.password.is_empty() {
            text(question).size(15).color(white(0.62 * alpha))
        } else if secret {
            let dots = "●".repeat(self.password.chars().count().min(24));
            text(dots).size(11).color(white(alpha))
        } else {
            text(self.password.clone()).size(15).color(white(alpha))
        };
        let arrow = button(
            container(
                text(if checking { "…" } else { "→" })
                    .size(17)
                    .color(white(alpha)),
            )
            .center_x(30)
            .center_y(30),
        )
        .padding(0)
        .on_press_maybe((!checking && self.answer_ready()).then_some(Message::Submit))
        .style(move |_, status| button::Style {
            background: Some(
                Color::from_rgba(
                    1.0,
                    1.0,
                    1.0,
                    match status {
                        button::Status::Hovered | button::Status::Pressed => 0.34,
                        _ => 0.22,
                    } * alpha,
                )
                .into(),
            ),
            border: Border {
                radius: 15.0.into(),
                ..Default::default()
            },
            text_color: white(alpha),
            ..Default::default()
        });
        let field_ring = ring(Target::Field);
        let pill = frosted(
            self.clock.material(),
            container(
                row![
                    container(field_text)
                        .padding([0, 6])
                        .width(Length::Fill)
                        .center_y(Length::Fill),
                    arrow
                ]
                .align_y(Alignment::Center)
                .spacing(6),
            )
            .padding(iced::Padding::from([5, 5]).left(14.0))
            .width(280)
            .height(42)
            .style(move |_| container::Style {
                background: None,
                border: if field_ring {
                    Border {
                        radius: 21.0.into(),
                        width: 2.0,
                        color: white(0.95 * alpha),
                    }
                } else {
                    Border {
                        radius: 21.0.into(),
                        width: 1.0,
                        color: Color::from_rgba(1.0, 1.0, 1.0, 0.28 * alpha),
                    }
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.18 * alpha),
                    offset: Vector::new(0.0, 6.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            }),
            21.0,
            alpha,
        );
        let status = self.status();
        let many = self.greeter.as_ref().is_some_and(|g| g.users.len() > 1);
        let name_text = text(name.to_owned())
            .size(17)
            .color(plain(alpha))
            .font(iced::Font {
                weight: iced::font::Weight::Semibold,
                ..Default::default()
            });
        let name_row: Element<'a, Message> = if many {
            row![
                chevron(
                    self.clock.material(),
                    "chevron-left",
                    Message::PreviousUser,
                    alpha,
                    ring(Target::PreviousUser)
                ),
                name_text,
                chevron(
                    self.clock.material(),
                    "chevron-right",
                    Message::NextUser,
                    alpha,
                    ring(Target::NextUser)
                )
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        } else {
            name_text.into()
        };
        let rise = 10.0 * (1.0 - focus);
        let prompt = column![Space::new().height(rise)];
        let identity: Element<'a, Message> = if compact {
            row![avatar, name_row]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
        } else {
            column![avatar, Space::new().height(12), name_row]
                .align_x(Alignment::Center)
                .into()
        };
        let mut prompt = prompt
            .push(
                column![
                    identity,
                    Space::new().height(if compact { 8 } else { 14 }),
                    row![
                        Space::new().width(shake.max(0.0)),
                        pill,
                        Space::new().width((-shake).max(0.0))
                    ],
                    Space::new().height(10),
                    text(status).size(13).color(white(0.85 * alpha)),
                ]
                .align_x(Alignment::Center),
            )
            .align_x(Alignment::Center);
        if self.keyboard_reveal > 0.001 {
            let reveal = ease_in_out(self.keyboard_reveal);
            let height = (5.0 * keys.height + 44.0 * keys.width) * reveal;
            prompt = prompt.push(Space::new().height(8.0 * reveal)).push(
                container(self.on_screen_keyboard(alpha * reveal, keys))
                    .height(height)
                    .clip(true),
            );
        }
        prompt.into()
    }

    // The greeter's session choice and power buttons along the bottom edge.
    pub(crate) fn corners<'a>(&'a self, content: f32) -> Option<Element<'a, Message>> {
        let alpha = content;
        let ring = |target: Target| self.focus_visible && self.target == target;
        let keyboard = icon_chip(
            self.clock.material(),
            "keyboard",
            if self.keyboard {
                tr("shell-hide-keyboard")
            } else {
                tr("shell-keyboard")
            },
            Message::ToggleKeyboard,
            alpha,
            ring(Target::Keyboard),
            false,
        );
        let mut left = row![].spacing(8).align_y(Alignment::Center);
        let mut power = row![].spacing(8);
        if let Some(greeter) = self.greeter.as_ref() {
            left = left.push(match greeter.sessions.get(greeter.session) {
                Some(session) if greeter.sessions.len() > 1 => icon_chip(
                    self.clock.material(),
                    "chevron-down",
                    &session.name,
                    Message::NextSession,
                    alpha,
                    ring(Target::Session),
                    true,
                ),
                Some(session) => chip(
                    self.clock.material(),
                    session.name.clone(),
                    None,
                    alpha,
                    false,
                ),
                None => chip(
                    self.clock.material(),
                    tr("shell-no-desktop-session-is-installed").to_owned(),
                    None,
                    alpha,
                    false,
                ),
            });
            if greeter.power {
                power = power.push(icon_button(
                    self.clock.material(),
                    "power",
                    Message::TogglePower,
                    alpha,
                    ring(Target::PowerMenu),
                ));
            }
        }
        left = left.push(keyboard);
        if self.greeter.is_none() && self.can_switch {
            power = power.push(chip(
                self.clock.material(),
                tr("shell-switch-user").to_owned(),
                Some(Message::SwitchUser),
                alpha,
                ring(Target::SwitchUser),
            ));
        }
        Some(
            container(
                row![left, Space::new().width(Length::Fill), power].align_y(Alignment::Center),
            )
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_bottom(Length::Fill)
            .into(),
        )
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
