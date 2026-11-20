use crate::{accounts, background, handle, ui};
use enzui_core::app_runtime::{self, AccessibilityWake};
use enzui_shell::i18n::{self, tr};
use enzui_widgets::a11y_actions::{apply_accessibility_wake, apply_pending_accessibility_actions};
use iced::{
    Element, Subscription, Task,
    keyboard::{self, key::Named},
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
enum Message {
    Control(ui::Message),
    Input(iced::Event),
    Tick(Instant),
    Screenshot(iced::window::Screenshot),
    Accessibility(AccessibilityWake),
    Backdrop(u64, ui::Images),
}

struct Preview {
    ui: ui::Ui,
    images: ui::Images,
    fallback_images: Option<ui::Images>,
    backdrop_epoch: u64,
    last_tick: Instant,
    animating: bool,
    checking: Option<Instant>,
    accepted: bool,
    capture_pending: bool,
    preferences_read: Instant,
    wallpaper: Option<enzui_shell::desktop_preferences::Wallpaper>,
    live: Option<crate::live_background::Live>,
    live_policy: (bool, bool),
    theme: iced::Theme,
    report_at: Instant,
    live_updates: u64,
    dimensions: (u32, u32),
    live_size: (u32, u32),
    lens_probe: bool,
    test_backdrop: bool,
    test_images: Option<ui::Images>,
    flat_backdrop: usize,
    flat_images: Option<[ui::Images; 2]>,
}

impl Preview {
    fn new() -> Self {
        let backdrop = background::load(&Default::default(), 1366, 768);
        Self {
            ui: {
                let mut ui = fixture();
                match std::env::var("ENZUI_GREETER_PREVIEW_STATE").as_deref() {
                    Ok("prompt") => ui.last_input = Some(Instant::now()),
                    Ok("keyboard") => {
                        ui.last_input = Some(Instant::now());
                        ui.keyboard = true;
                    }
                    Ok("power") => {
                        ui.local(ui::Message::TogglePower);
                    }
                    _ => {}
                }
                ui
            },
            images: ui::Images {
                desktop: None,
                wallpaper: handle(backdrop.sharp),
                soft: handle(backdrop.soft),
                has_clock: false,
                has_date: false,
                light: backdrop.light,
            },
            fallback_images: None,
            backdrop_epoch: 0,
            last_tick: Instant::now(),
            animating: true,
            checking: None,
            accepted: false,
            preferences_read: Instant::now(),
            wallpaper: None,
            live: None,
            live_policy: (true, true),
            theme: crate::appearance::theme().0,
            report_at: Instant::now(),
            live_updates: 0,
            dimensions: (1100, 700),
            live_size: (0, 0),
            // The lens probe is a greeter-preview tool; a lock-mode capture shows
            // the lock screen as it is.
            lens_probe: std::env::var("ENZUI_GREETER_PREVIEW_MODE").as_deref() != Ok("lock"),
            test_backdrop: false,
            test_images: None,
            flat_backdrop: 0,
            flat_images: None,
            capture_pending: std::env::var("ENZUI_GREETER_PREVIEW_CAPTURE").as_deref() == Ok("1"),
        }
    }

    fn control(&mut self, action: ui::Message) {
        let Some(action) = self.ui.local(action) else {
            return;
        };
        let now = Instant::now();
        match action {
            ui::Message::Submit if self.ui.phase == ui::Phase::Idle => {
                self.accepted = self.ui.password == "preview";
                self.ui.password.clear();
                self.ui.phase = ui::Phase::Checking;
                self.checking = Some(now);
            }
            ui::Message::PreviousUser | ui::Message::NextUser => {
                let g = self
                    .ui
                    .greeter
                    .as_mut()
                    .expect("the greeter preview owns its initialized state");
                let step = if action == ui::Message::NextUser {
                    1
                } else {
                    g.users.len() - 1
                };
                g.user = (g.user + step) % g.users.len();
                g.notice = None;
                self.ui.password.clear();
                self.ui.error = None;
            }
            ui::Message::NextSession => {
                let g = self
                    .ui
                    .greeter
                    .as_mut()
                    .expect("the greeter preview owns its initialized state");
                g.session = (g.session + 1) % g.sessions.len();
            }
            ui::Message::PowerOff | ui::Message::Restart | ui::Message::Sleep => {
                self.ui
                    .greeter
                    .as_mut()
                    .expect("the greeter preview owns its initialized state")
                    .notice = Some(tr("shell-preview-only-no-power-action-was-performed").into());
            }
            _ => {}
        }
        self.ui.last_input = Some(now);
    }

    fn input(&mut self, event: iced::Event) {
        let iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            text,
            modifiers,
            ..
        }) = event
        else {
            if matches!(
                event,
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
            ) {
                self.ui.last_input = Some(Instant::now());
            }
            return;
        };
        if key == keyboard::Key::Named(Named::F1) {
            self.checking = None;
            self.ui.phase = ui::Phase::Idle;
            self.ui.password.clear();
            self.ui.error = None;
            self.ui.keyboard = false;
            self.ui.power_menu = false;
            self.ui.last_input = None;
            self.ui.focus_visible = false;
            self.ui.target = ui::Target::Field;
            return;
        }
        if key == keyboard::Key::Named(Named::F5) {
            self.ui = fixture();
            self.checking = None;
            return;
        }
        if key == keyboard::Key::Named(Named::F2) {
            self.ui.last_input = Some(Instant::now());
            return;
        }
        if key == keyboard::Key::Named(Named::F8) {
            self.lens_probe = !self.lens_probe;
            return;
        }
        if key == keyboard::Key::Named(Named::F9) {
            self.test_images.get_or_insert_with(test_backdrop);
            self.flat_backdrop = 0;
            self.test_backdrop = !self.test_backdrop;
            return;
        }
        if key == keyboard::Key::Named(Named::F10) {
            self.flat_images.get_or_insert_with(|| {
                [[20, 24, 38], [218, 224, 234]].map(|rgb| {
                    let handle = handle(background::Picture::solid(8, 8, rgb));
                    ui::Images {
                        desktop: None,
                        wallpaper: handle.clone(),
                        soft: handle,
                        has_clock: false,
                        has_date: false,
                        light: rgb[0] > 128,
                    }
                })
            });
            self.flat_backdrop = (self.flat_backdrop + 1) % 3;
            return;
        }
        if key == keyboard::Key::Named(Named::F7) {
            use enzui_shell::{
                session_preferences::{LockMaterial, SessionChange},
                settings::{Change, Store},
            };
            let index = LockMaterial::ALL
                .iter()
                .position(|m| *m == self.ui.clock.material())
                .unwrap_or(0);
            let next = LockMaterial::ALL[(index + 1) % LockMaterial::ALL.len()];
            // Only the launcher's explicitly supplied review profile may be written.
            if let Some(store) = std::env::var_os("ENZUI_SHELL_SETTINGS")
                .and_then(|path| Store::at(path.into()).ok())
            {
                match store.update(Change::Session(SessionChange::LockMaterial(next))) {
                    Ok(settings) => self.ui.clock = settings.session.lock_clock,
                    Err(error) => eprintln!("Preview clock selection: {error}"),
                }
            }
            return;
        }
        if key == keyboard::Key::Named(Named::F3) {
            self.ui = fixture();
            self.checking = None;
            self.ui.last_input = Some(Instant::now());
            self.ui.error = Some((tr("shell-authentication-failed").into(), Instant::now()));
            return;
        }
        if key == keyboard::Key::Named(Named::F4) {
            self.checking = None;
            self.ui.phase = ui::Phase::Leaving(Instant::now());
            return;
        }
        if self.ui.phase != ui::Phase::Idle {
            return;
        }
        self.ui.last_input = Some(Instant::now());
        match key {
            keyboard::Key::Named(Named::Tab) => self.ui.move_focus(!modifiers.shift()),
            keyboard::Key::Named(Named::Enter) => self.control(self.ui.target.message()),
            keyboard::Key::Named(Named::ArrowUp | Named::ArrowLeft) if self.ui.power_menu => {
                self.ui.move_focus(false);
            }
            keyboard::Key::Named(Named::ArrowDown | Named::ArrowRight) if self.ui.power_menu => {
                self.ui.move_focus(true);
            }
            keyboard::Key::Named(Named::Backspace) if self.ui.power_menu => {}
            keyboard::Key::Named(Named::Backspace) if modifiers.control() => {
                self.ui.password.clear();
            }
            keyboard::Key::Named(Named::Backspace) => {
                self.ui.password.pop();
            }
            keyboard::Key::Named(Named::Escape) if self.ui.power_menu => {
                self.ui.power_menu = false;
                self.ui.target = ui::Target::PowerMenu;
            }
            keyboard::Key::Named(Named::Escape) => {
                self.ui.password.clear();
                self.ui.power_menu = false;
                self.ui.error = None;
                self.ui.last_input = None;
                self.ui.target = ui::Target::Field;
                self.ui.keyboard = false;
            }
            keyboard::Key::Named(Named::ArrowUp) if self.ui.password.is_empty() => {
                self.control(ui::Message::PreviousUser);
            }
            keyboard::Key::Named(Named::ArrowDown) if self.ui.password.is_empty() => {
                self.control(ui::Message::NextUser);
            }
            keyboard::Key::Named(Named::CapsLock) => self.ui.caps_lock = !self.ui.caps_lock,
            _ if !modifiers.control() && !modifiers.alt() && !modifiers.logo() => {
                if let Some(text) = text {
                    if text.as_str() == " " && self.ui.target != ui::Target::Field {
                        self.control(self.ui.target.message());
                    } else if !self.ui.power_menu && self.ui.password.chars().count() < 256 {
                        self.ui.target = ui::Target::Field;
                        self.ui.password.extend(
                            text.chars()
                                .filter(|c| !c.is_control())
                                .take(256 - self.ui.password.chars().count()),
                        );
                        self.ui.error = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let actions = apply_pending_accessibility_actions();
        let mut deferred = Task::none();
        match message {
            Message::Control(action) => {
                self.control(action);
                self.animating = true;
            }
            Message::Input(iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::F6),
                ..
            })) => {
                return Task::batch([actions, screenshot()]);
            }
            Message::Input(iced::Event::Window(iced::window::Event::Resized(size))) => {
                self.dimensions = (size.width.max(16.0) as u32, size.height.max(16.0) as u32);
                self.animating = true;
            }
            Message::Input(event) => {
                self.input(event);
                self.animating = true;
            }
            Message::Screenshot(image) => {
                let root = std::env::var_os("ENZUI_DESKTOP_ROOT")
                    .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
                let path = root.join(".build/greeter-preview/preview.png");
                let result = std::fs::create_dir_all(
                    path.parent()
                        .expect("the greeter preview owns its initialized state"),
                )
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    raster::save_buffer(
                        &path,
                        &image.rgba,
                        image.size.width,
                        image.size.height,
                        raster::ColorType::Rgba8,
                    )
                    .map_err(|e| e.to_string())
                });
                if let Err(error) = result {
                    eprintln!("Preview screenshot: {error}");
                } else {
                    eprintln!("Preview screenshot saved: {}", path.display());
                }
            }
            Message::Backdrop(epoch, images) => {
                if epoch == self.backdrop_epoch {
                    self.fallback_images = Some(images.clone());
                    self.images = images;
                    self.animating = true;
                }
            }
            Message::Accessibility(wake) => {
                return Task::batch([actions, apply_accessibility_wake(wake)]);
            }
            Message::Tick(now) => {
                if now.saturating_duration_since(self.preferences_read) >= Duration::from_secs(1) {
                    if let Some(settings) =
                        enzui_shell::settings::Store::from_environment().and_then(|s| s.load().ok())
                    {
                        let wallpaper =
                            background::selected(&settings.wallpaper, &settings.session.lock_clock);
                        let (theme, reduced) = crate::appearance::theme();
                        self.theme = theme;
                        self.ui.reduced_motion = reduced || !settings.compositor.animations;
                        let policy = (
                            settings.session.lock_clock.hide_wallpaper_clock,
                            settings.session.lock_clock.hide_wallpaper_date,
                        );
                        let language_changed = i18n::select(settings.language);
                        let changed = language_changed
                            || self.wallpaper.as_ref() != Some(&wallpaper)
                            || policy != self.live_policy
                            || self.live_size != self.dimensions;
                        if changed {
                            self.live = None;
                            self.live_policy = policy;
                            self.live_size = self.dimensions;
                            self.live = crate::live_background::Live::start(
                                &settings.wallpaper,
                                &settings.session.lock_clock,
                                self.dimensions,
                                self.ui.reduced_motion,
                            );
                        }
                        self.ui.clock = settings.session.lock_clock;
                        i18n::select(settings.language);
                        self.ui.clock_24h = settings.session.clock_24h;
                        if changed {
                            self.wallpaper = Some(wallpaper.clone());
                            self.preferences_read = now;
                            self.backdrop_epoch = self.backdrop_epoch.wrapping_add(1);
                            let epoch = self.backdrop_epoch;
                            deferred = Task::perform(
                                async move { tokio_backdrop(wallpaper).await },
                                move |images| Message::Backdrop(epoch, images),
                            );
                        }
                    }
                    self.preferences_read = now;
                }
                if let Some(live) = &mut self.live {
                    if let Some(frame) = live.frame() {
                        self.images = frame;
                        self.live_updates += 1;
                    }
                    if !live.healthy() {
                        drop(self.live.take());
                        if let Some(images) = &self.fallback_images {
                            self.images = images.clone();
                        }
                        self.animating = true;
                    }
                }
                if self
                    .checking
                    .is_some_and(|at| now.duration_since(at) >= Duration::from_millis(650))
                {
                    self.checking = None;
                    self.ui.phase = if self.accepted {
                        ui::Phase::Leaving(now)
                    } else {
                        ui::Phase::Idle
                    };
                    if !self.accepted {
                        self.ui.error = Some((tr("shell-authentication-failed").into(), now));
                    }
                }
                if matches!(self.ui.phase, ui::Phase::Leaving(at) if now.duration_since(at) > Duration::from_millis(1500))
                {
                    self.ui = fixture();
                    self.ui
                        .greeter
                        .as_mut()
                        .expect("the greeter preview owns its initialized state")
                        .notice =
                        Some(tr("shell-preview-sign-in-complete-no-session-was-launched").into());
                    self.ui.last_input = Some(now);
                }
                self.animating = self.ui.tick(
                    now,
                    now.duration_since(self.last_tick).as_secs_f32().min(0.1),
                );
                self.last_tick = now;
                if now.saturating_duration_since(self.report_at) >= Duration::from_millis(100) {
                    self.report_at = now;
                    if let Some(path) = std::env::var_os("ENZUI_GREETER_PREVIEW_REPORT") {
                        let report = serde_json::json!({"material":self.ui.clock.material(),"layout":self.ui.clock.layout,"focus":self.ui.preview_focus(),"keyboard":self.ui.keyboard,"power":self.ui.power_menu,"live":self.live.is_some(),"live_updates":self.live_updates,"wallpaper_clock":self.images.has_clock,"wallpaper_date":self.images.has_date,"light":self.images.light});
                        let path = std::path::PathBuf::from(path);
                        let tmp = path.with_extension("tmp");
                        if std::fs::write(&tmp, report.to_string()).is_ok() {
                            let _ = std::fs::rename(tmp, path);
                        }
                    }
                }
                if self.capture_pending
                    && now.duration_since(self.ui.started) >= Duration::from_secs(2)
                {
                    self.capture_pending = false;
                    return Task::batch([actions, screenshot()]);
                }
            }
        }
        Task::batch([actions, deferred])
    }

    fn view(&self) -> Element<'_, Message> {
        use enzui_shell::session_preferences::ClockStyle;
        use iced::{
            Alignment, Length,
            widget::{column, container, stack, text},
        };
        iced::widget::responsive(move |size| {
            let images = if self.flat_backdrop > 0 {
                &self
                    .flat_images
                    .as_ref()
                    .expect("the greeter preview owns its initialized state")
                    [self.flat_backdrop - 1]
            } else if self.test_backdrop {
                self.test_images.as_ref().unwrap_or(&self.images)
            } else {
                &self.images
            };
            let base = self
                .ui
                .view(images, true, size, Instant::now())
                .map(Message::Control);
            if !self.lens_probe
                || self.ui.last_input.is_some()
                || size.width < 600.0
                || size.height < 700.0
                || !matches!(
                    self.ui.clock.material().style(),
                    ClockStyle::Glass | ClockStyle::FrostedGlass
                )
            {
                return base;
            }
            let pill = crate::clock_widgets::probe(self.ui.clock.material().style(), 360.0)
                .map(Message::Control);
            let caption = text(tr(
                "shell-clock-lens-f8-hide-pill-f9-pattern-f10-solid-backdrops",
            ))
            .size(11)
            .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.75));
            let probe = container(column![pill, caption].align_x(Alignment::Center).spacing(8))
                .padding(iced::Padding::ZERO.top(size.height - 180.0))
                .center_x(Length::Fill)
                .height(Length::Fill);
            stack![base, probe].into()
        })
        .into()
    }

    fn theme(&self) -> iced::Theme {
        self.theme.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Window(iced::window::Event::Resized(_))
                | iced::Event::Keyboard(keyboard::Event::KeyPressed { .. })
                | iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_)) => {
                    Some(Message::Input(event))
                }
                _ => None,
            }),
            iced::time::every(Duration::from_millis(if self.animating {
                16
            } else if self.live.is_some() {
                66
            } else {
                1000
            }))
            .map(Message::Tick),
            app_runtime::accessibility_wake_subscription(Message::Accessibility),
        ])
    }
}

fn test_backdrop() -> ui::Images {
    use std::fmt::Write;
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1100 700'><defs><linearGradient id='bg' x2='1' y2='1'><stop stop-color='#12495b'/><stop offset='.5' stop-color='#bd7a37'/><stop offset='1' stop-color='#5c3265'/></linearGradient></defs><path fill='url(#bg)' d='M0 0H1100V700H0Z'/>",
    );
    let colors = [
        "#2679ee", "#ed3b74", "#ffa700", "#8050ee", "#08b99a", "#ef543b",
    ];
    for row in 0..6 {
        for col in 0..9 {
            let (x, y) = (col * 132 - 36, row * 140 - 24);
            let color = colors[(row + col) as usize % colors.len()];
            let _ = write!(
                svg,
                "<g transform='translate({x} {y})'><rect width='108' height='108' rx='24' fill='#f4f0ea'/><rect x='18' y='14' width='72' height='58' rx='16' fill='{color}'/><circle cx='54' cy='43' r='17' fill='white'/><path d='M23 86H85' stroke='{color}' stroke-width='7' stroke-linecap='round'/><text x='54' y='102' text-anchor='middle' font-size='11' fill='#182636'>{row}{col}</text></g>"
            );
        }
    }
    svg.push_str("</svg>");
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .expect("comparison backdrop");
    let mut pixels = resvg::tiny_skia::Pixmap::new(1100, 700).expect("comparison pixels");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    let handle = iced::widget::image::Handle::from_rgba(1100, 700, pixels.take());
    ui::Images {
        desktop: None,
        wallpaper: handle.clone(),
        soft: handle,
        has_clock: false,
        has_date: false,
        light: false,
    }
}

fn screenshot() -> Task<Message> {
    iced::window::latest()
        .and_then(iced::window::screenshot)
        .map(Message::Screenshot)
}

fn fixture() -> ui::Ui {
    let settings = enzui_shell::settings::Store::from_environment()
        .and_then(|s| s.load().ok())
        .unwrap_or_default();
    let mut ui = ui::Ui::new(tr("shell-alex-morgan").into(), settings.session.clock_24h);
    ui.clock = settings.session.lock_clock;
    ui.greeter = Some(ui::Greeter {
        users: vec![
            accounts::User {
                name: "alex".into(),
                display: tr("shell-alex-morgan").into(),
            },
            accounts::User {
                name: "sam".into(),
                display: tr("shell-sam-rivera").into(),
            },
        ],
        user: 0,
        sessions: ["enzUI", tr("shell-another-desktop")]
            .into_iter()
            .map(|name| accounts::Session {
                id: name.into(),
                name: name.into(),
                exec: String::new(),
                desktop_names: String::new(),
                icon: None,
                path: Default::default(),
            })
            .collect(),
        session: 0,
        prompt: None,
        notice: None,
        power: true,
        existing: None,
    });
    // The lock screen is the same view without the greeter's user and
    // session choice.
    if std::env::var("ENZUI_GREETER_PREVIEW_MODE").as_deref() == Ok("lock") {
        ui.greeter = None;
    }
    ui
}

/// Opens the production greeter view in an ordinary window with inert mock actions.
///
/// # Errors
/// Returns the native application runtime error if the window cannot run.
pub fn run() -> iced::Result {
    let size = std::env::var("ENZUI_GREETER_PREVIEW_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
            ((320..=1920).contains(&w) && (320..=1080).contains(&h)).then_some((w as f32, h as f32))
        })
        .unwrap_or((1100.0, 700.0));
    enzui_widgets::app_setup::run_with_traversal(
        iced::application(Preview::new, Preview::update, Preview::view)
            .title(tr(
                "shell-enzui-login-preview-password-preview-f1-idle-f2-prompt-f7-material",
            ))
            .window_size(size)
            .font(iced::Font {
                family: iced::font::Family::Name(enzui_shell::font_settings::default_sans()),
                ..Default::default()
            })
            .subscription(Preview::subscription)
            .theme(Preview::theme),
        enzui_widgets::app_setup::TraversalBinding::Disabled,
    )
}

async fn tokio_backdrop(wallpaper: enzui_shell::desktop_preferences::Wallpaper) -> ui::Images {
    // Preview rendering uses the same bounded loader off the event loop.
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let b = background::load(&wallpaper, 1366, 768);
        let _ = sender.send(ui::Images {
            desktop: None,
            wallpaper: handle(b.sharp),
            soft: handle(b.soft),
            has_clock: b.has_clock,
            has_date: false,
            light: b.light,
        });
    });
    receiver.await.expect("preview backdrop worker")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mock_sign_in_and_power_never_reach_system_backends() {
        let mut preview = Preview::new();
        assert!(
            preview
                .ui
                .greeter
                .as_ref()
                .unwrap()
                .sessions
                .iter()
                .all(|s| s.exec.is_empty())
        );
        preview.control(ui::Message::Restart);
        assert!(preview.ui.status().contains("no power action"));
        preview.control(ui::Message::PowerOff);
        preview.ui.password = "wrong".into();
        preview.control(ui::Message::Submit);
        let _ = preview.update(Message::Tick(Instant::now() + Duration::from_secs(1)));
        assert!(preview.ui.error.is_some());
        preview.ui.password = "preview".into();
        preview.control(ui::Message::Submit);
        let _ = preview.update(Message::Tick(Instant::now() + Duration::from_secs(2)));
        assert!(matches!(preview.ui.phase, ui::Phase::Leaving(_)));
    }
}
