//! First-login setup and a guide to the desktop’s keyboard shortcuts.

#[path = "settings_preview_scene.rs"]
mod scene;
#[path = "theme_preview.rs"]
mod theme_preview;

use enzui_core::semantics::{Role, Semantics};
use enzui_core::theme_file::integration::{ThemeEvent, ThemeIntegration};
use enzui_core::wizard::Wizard;
use enzui_shell::i18n::tr;
use enzui_shell::local_ai::{self, Online, ProviderEdit};
use enzui_shell::settings::{BarEdge, Change, Metric, Settings, Store, TextStep};
use enzui_shell::shell_style::ShellStyle;
use enzui_shell::shortcut_map;
use enzui_shell::system_settings::display::{self, Displays, SCALES, supports_scale};
use enzui_shell::theme_palette::{self, PRESETS, Scheme, Seed, Style};
use enzui_shell::theme_presets::Choices;
use enzui_shell::tr;
use enzui_shell::welcome::{self, Ai, Completion, Launch, ModelKind, ModelOffer};
use enzui_theme::EnzThemeMode;
use enzui_widgets::{
    app_shell::{ShellLayout, enz_app_shell},
    banner::{BannerTone, enz_banner},
    button::{ButtonKind, enz_button},
    color_swatch::enz_color_swatch,
    key_badge::enz_key_badge,
    motion::fade_on_change,
    preferences_row::enz_preferences_row,
    pressable::{PressableStyle, enz_pressable},
    scroll_view::enz_scroll_view,
    secure_field::enz_secure_field,
    semantics_container::SemanticsContainer,
    slider::enz_slider,
    text::{TextAlign, TextSize, TextTone, enz_text},
    title_bar::enz_title_bar,
    view_switcher::{SwitcherPage, enz_view_switcher},
    wizard::WizardEvent,
};
use iced::widget::{Space, column, container, responsive, row, svg};
use iced::{Alignment, Element, Length, Subscription, Task};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const DESKTOP: usize = 0;
const APPEARANCE: usize = 1;
const WINDOWS: usize = 2;
const AI: usize = 3;
const STEPS: usize = 5;
// Matches the Settings display confirmation.
const SCALE_CONFIRM: Duration = Duration::from_secs(15);
const PROVIDERS: [(&str, &str); 3] = [
    ("openrouter", "OpenRouter"),
    ("openai", "OpenAI"),
    ("groq", "Groq"),
];

fn css(color: iced::Color) -> String {
    theme_palette::hex(color)
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Theme(Box<ThemeEvent>),
    KeyLabels(shortcut_map::KeyLabels),
    Wizard(WizardEvent),
    Loaded(Result<Settings, String>),
    ThemeRead(Choices),
    Change(Change),
    Saved(Result<Settings, String>),
    Dark(bool),
    Choose(Choices),
    ThemeSaved(Result<Choices, String>),
    DisplaysRead(Displays),
    ScaleMenu(bool),
    Scale(f64),
    ScaleApplied(Result<Vec<display::Change>, String>),
    KeepScale,
    RevertScale,
    Reverted(Result<(), String>),
    Tick(Instant),
    AiEnabled(bool),
    Provider(usize),
    Key(Secret),
    Reveal,
    AddProvider,
    ProviderAdded(Result<String, String>),
    CatalogRead(Vec<ModelOffer>),
    TextModel(Option<String>),
    SpeechModel(Option<String>),
    Finished(Result<(), Failure>),
    CloseAnyway,
    CloseRequested,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Queue(String),
    Ai(String),
    Record(String),
}

struct PendingScale {
    previous: Vec<display::Change>,
    deadline: Instant,
}

pub(crate) struct Welcome {
    themes: ThemeIntegration,
    wizard: Wizard,
    store: Store,
    settings: Settings,
    key_labels: shortcut_map::KeyLabels,
    loaded: bool,
    writes: VecDeque<Change>,
    writing: bool,
    choices: Choices,
    displays: Option<Displays>,
    scale_menu_open: bool,
    scale: Option<PendingScale>,
    now: Instant,
    ai: bool,
    provider: usize,
    key: Secret,
    reveal: bool,
    adding: bool,
    added: Vec<String>,
    catalog: Option<Vec<ModelOffer>>,
    text_model: Option<String>,
    speech_model: Option<String>,
    finishing: bool,
    finish_after_write: Option<bool>,
    error: Option<String>,
    record_failed: bool,
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, fallback: T) -> T {
    tokio::task::spawn_blocking(work).await.unwrap_or(fallback)
}

impl Welcome {
    pub(crate) fn new(store: Store) -> (Self, Task<Message>) {
        let (themes, boot) = ThemeIntegration::boot_for(
            enzui_core::app_data::AppId::new(welcome::APP_ID).expect("a valid app id"),
        );
        let app = Self {
            themes,
            wizard: Wizard::new(STEPS),
            store: store.clone(),
            settings: Settings::default(),
            key_labels: shortcut_map::KeyLabels::default(),
            loaded: false,
            writes: VecDeque::new(),
            writing: false,
            choices: Choices::default(),
            displays: None,
            scale_menu_open: false,
            scale: None,
            now: Instant::now(),
            ai: welcome::record_path().is_some_and(|path| welcome::completed(&path))
                && local_ai::enabled(),
            provider: 0,
            key: Secret::default(),
            reveal: false,
            adding: false,
            added: Vec::new(),
            catalog: None,
            text_model: None,
            speech_model: None,
            finishing: false,
            finish_after_write: None,
            error: None,
            record_failed: false,
        };
        let load = Task::perform(
            blocking(
                move || store.load(),
                Err(tr("welcome-could-not-read-settings").to_owned()),
            ),
            Message::Loaded,
        );
        let theme = Task::perform(
            blocking(
                || {
                    enzui_shell::theme_store::Store::read(
                        &enzui_core::theme_file::ThemePaths::from_env(),
                    )
                    .choices()
                },
                Choices::default(),
            ),
            Message::ThemeRead,
        );
        (
            app,
            Task::batch([
                boot.map(|event| Message::Theme(Box::new(event))),
                load,
                theme,
                read_displays(),
                Task::perform(
                    blocking(welcome::read_catalog, Vec::new()),
                    Message::CatalogRead,
                ),
            ]),
        )
    }

    pub(crate) fn title(&self) -> String {
        tr("welcome-window-title").to_owned()
    }

    pub(crate) fn theme(&self) -> iced::Theme {
        self.themes.iced_theme()
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.themes
                .subscription()
                .map(|event| Message::Theme(Box::new(event))),
            shortcut_map::key_labels_subscription().map(Message::KeyLabels),
            iced::window::close_requests().map(|_| Message::CloseRequested),
            if self.scale.is_some() {
                iced::time::every(Duration::from_millis(250)).map(Message::Tick)
            } else {
                Subscription::none()
            },
        ])
    }

    fn mode(&self) -> EnzThemeMode {
        self.themes.theme().mode()
    }

    fn live(&self) -> bool {
        self.loaded && !self.finishing
    }

    fn current_choices(&self) -> Choices {
        let mut choices = self.choices;
        if let Some(scheme) = self
            .settings
            .theme_scheme
            .as_deref()
            .and_then(Scheme::from_id_str)
        {
            choices.scheme = scheme;
        }
        choices
    }

    fn change(&mut self, change: Change) -> Task<Message> {
        if let Err(error) = self.settings.preview(change.clone()) {
            self.error = Some(error);
            return Task::none();
        }
        if let Some(last) = self.writes.back()
            && last.same_key(&change)
        {
            self.writes.pop_back();
        }
        self.writes.push_back(change);
        self.write_next()
    }

    fn write_next(&mut self) -> Task<Message> {
        if self.writing {
            return Task::none();
        }
        let Some(change) = self.writes.pop_front() else {
            return Task::none();
        };
        self.writing = true;
        let store = self.store.clone();
        let fade = matches!(change, Change::Light(_) | Change::ThemeScheme(_))
            && !self.themes.theme().motion.reduced_motion;
        let compositor = fade
            .then(enzui_shell::compositor::Compositor::from_environment)
            .and_then(Result::ok);
        Task::perform(
            async move {
                if let Some(compositor) = &compositor {
                    compositor.appearance_transition(false).await;
                }
                let saved = blocking(
                    move || store.update(change),
                    Err(tr("welcome-could-not-save").to_owned()),
                )
                .await;
                if let Some(compositor) = &compositor {
                    compositor.appearance_transition(true).await;
                }
                saved
            },
            Message::Saved,
        )
    }

    fn selected_models(&self) -> Vec<String> {
        if !self.ai {
            return Vec::new();
        }
        self.text_model
            .iter()
            .chain(self.speech_model.iter())
            .cloned()
            .collect()
    }

    fn finish(&mut self, skip: bool) -> Task<Message> {
        if self.finishing {
            return Task::none();
        }
        self.finishing = true;
        if self.writing {
            self.finish_after_write = Some(skip);
            return Task::none();
        }
        self.error = None;
        let revert = self.scale.take().map(|pending| pending.previous);
        let queued = if skip {
            Vec::new()
        } else {
            self.selected_models()
        };
        let local_enabled = self.ai;
        let session = welcome::Session::from_environment();
        let ai = if !skip && (self.ai || !self.added.is_empty()) {
            Ai::Configured
        } else {
            Ai::Skipped
        };
        Task::perform(
            blocking(
                move || {
                    if let Some(previous) = revert {
                        let _ = display::apply(&previous);
                    }
                    if !skip && session.installed && !session.remote {
                        local_ai::set_enabled(local_enabled).map_err(Failure::Ai)?;
                    }
                    if !queued.is_empty() {
                        welcome::queue_models(&welcome::settings_program(), &queued)
                            .map_err(Failure::Queue)?;
                    }
                    let path = welcome::record_path()
                        .ok_or_else(|| Failure::Record(tr("welcome-no-home").to_owned()))?;
                    if skip && welcome::completed(&path) {
                        return Ok(());
                    }
                    welcome::record(&path, &Completion { ai, queued })
                        .map_err(|error| Failure::Record(error.to_string()))
                },
                Err(Failure::Record(tr("welcome-could-not-save").to_owned())),
            ),
            Message::Finished,
        )
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Theme(event) => self
                .themes
                .update(*event)
                .map(|event| Message::Theme(Box::new(event))),
            Message::KeyLabels(keys) => {
                self.key_labels = keys;
                Task::none()
            }
            Message::Wizard(WizardEvent::Finish) => self.finish(false),
            Message::Wizard(WizardEvent::Cancel) | Message::CloseRequested => self.finish(true),
            Message::Wizard(event) => {
                enzui_widgets::wizard::apply(&mut self.wizard, event);
                Task::none()
            }
            Message::Loaded(Ok(settings)) => {
                self.settings = settings;
                self.loaded = true;
                Task::none()
            }
            Message::Loaded(Err(error)) | Message::Saved(Err(error)) => {
                self.writing = false;
                self.writes.clear();
                if self.finish_after_write.take().is_some() {
                    self.finishing = false;
                }
                self.error = Some(tr!("welcome-save-failed", "reason" => error));
                let store = self.store.clone();
                Task::perform(
                    blocking(
                        move || store.load(),
                        Err(tr("welcome-could-not-read-settings").to_owned()),
                    ),
                    |loaded| match loaded {
                        Ok(settings) => Message::Loaded(Ok(settings)),
                        Err(_) => Message::Loaded(Ok(Settings::default())),
                    },
                )
            }
            Message::Saved(Ok(settings)) => {
                self.writing = false;
                if self.writes.is_empty() {
                    self.settings = settings;
                    if let Some(skip) = self.finish_after_write.take() {
                        self.finishing = false;
                        return self.finish(skip);
                    }
                }
                self.write_next()
            }
            Message::ThemeRead(choices) => {
                self.choices = choices;
                Task::none()
            }
            Message::Change(change) => self.change(change),
            Message::Dark(dark) => {
                self.themes.set_mode_override(Some(if dark {
                    EnzThemeMode::Dark
                } else {
                    EnzThemeMode::Light
                }));
                self.change(Change::Light(!dark))
            }
            Message::Choose(after) => {
                let before = self.current_choices();
                if before == after {
                    return Task::none();
                }
                Task::perform(
                    blocking(
                        move || {
                            let mut store = enzui_shell::theme_store::Store::read(
                                &enzui_core::theme_file::ThemePaths::from_env(),
                            );
                            store.stage_from(before, after);
                            store.save().map(|()| after)
                        },
                        Err(tr("welcome-could-not-save").to_owned()),
                    ),
                    Message::ThemeSaved,
                )
            }
            Message::ThemeSaved(Ok(choices)) => {
                self.choices = choices;
                self.change(Change::ThemeScheme(choices.scheme.to_id_string()))
            }
            Message::ThemeSaved(Err(error)) => {
                self.error = Some(tr!("welcome-save-failed", "reason" => error));
                Task::none()
            }
            Message::DisplaysRead(displays) => {
                self.displays = Some(displays);
                Task::none()
            }
            Message::ScaleMenu(open) => {
                self.scale_menu_open = open;
                Task::none()
            }
            Message::Scale(scale) => {
                self.scale_menu_open = false;
                let Some(displays) = self.displays.clone() else {
                    return Task::none();
                };
                let previous = display::layout(&displays);
                let mut layout = previous.clone();
                let Some(primary) = layout.iter_mut().find(|change| change.enabled) else {
                    return Task::none();
                };
                if (primary.scale - scale).abs() < 1e-6 {
                    return Task::none();
                }
                primary.scale = scale;
                // A second change before confirming still returns to the first layout.
                let previous = self
                    .scale
                    .take()
                    .map_or(previous, |pending| pending.previous);
                Task::perform(
                    blocking(
                        move || display::apply(&layout).map(|()| previous),
                        Err(tr("welcome-could-not-save").to_owned()),
                    ),
                    Message::ScaleApplied,
                )
            }
            Message::ScaleApplied(Ok(previous)) => {
                self.now = Instant::now();
                self.scale = Some(PendingScale {
                    previous,
                    deadline: self.now + SCALE_CONFIRM,
                });
                read_displays()
            }
            Message::ScaleApplied(Err(error)) => {
                self.error = Some(tr!("welcome-save-failed", "reason" => error));
                read_displays()
            }
            Message::Tick(now) => {
                self.now = now;
                match &self.scale {
                    Some(pending) if now >= pending.deadline => self.update(Message::RevertScale),
                    _ => Task::none(),
                }
            }
            Message::KeepScale => {
                self.scale = None;
                Task::perform(blocking(display::read, Displays::default()), |displays| {
                    let saved = display::saved(&displays, &display::layout(&displays));
                    Message::Change(Change::Displays(saved))
                })
            }
            Message::RevertScale => {
                let Some(pending) = self.scale.take() else {
                    return Task::none();
                };
                Task::perform(
                    blocking(
                        move || display::apply(&pending.previous),
                        Err(tr("welcome-could-not-save").to_owned()),
                    ),
                    Message::Reverted,
                )
            }
            Message::Reverted(result) => {
                if let Err(error) = result {
                    self.error = Some(tr!("welcome-save-failed", "reason" => error));
                }
                read_displays()
            }
            Message::AiEnabled(enabled) => {
                self.ai = enabled;
                Task::none()
            }
            Message::Provider(index) => {
                self.provider = index.min(PROVIDERS.len() - 1);
                Task::none()
            }
            Message::Key(key) => {
                self.key = key;
                Task::none()
            }
            Message::Reveal => {
                self.reveal = !self.reveal;
                Task::none()
            }
            Message::AddProvider => {
                if self.adding || self.key.0.trim().is_empty() {
                    return Task::none();
                }
                let (name, _) = PROVIDERS[self.provider];
                let Some((_, base_url)) = local_ai::PRESETS.iter().find(|(id, _)| *id == name)
                else {
                    return Task::none();
                };
                self.adding = true;
                let edit = ProviderEdit::AddOnline(Online {
                    name: name.to_owned(),
                    base_url: (*base_url).to_owned(),
                    key: Some(std::mem::take(&mut self.key).0),
                });
                Task::perform(
                    blocking(
                        move || local_ai::change_providers(&edit).map(|()| name.to_owned()),
                        Err(tr("welcome-could-not-save").to_owned()),
                    ),
                    Message::ProviderAdded,
                )
            }
            Message::ProviderAdded(result) => {
                self.adding = false;
                match result {
                    Ok(name) if !self.added.contains(&name) => self.added.push(name),
                    Ok(_) => {}
                    Err(error) => {
                        self.error = Some(tr!("welcome-save-failed", "reason" => error));
                    }
                }
                Task::none()
            }
            Message::CatalogRead(offers) => {
                if offers
                    .iter()
                    .any(|offer| offer.id == welcome::DEFAULT_TEXT_MODEL)
                {
                    self.text_model = Some(welcome::DEFAULT_TEXT_MODEL.into());
                }
                self.catalog = Some(offers);
                Task::none()
            }
            Message::TextModel(id) => {
                self.text_model = id;
                Task::none()
            }
            Message::SpeechModel(id) => {
                self.speech_model = id;
                Task::none()
            }
            Message::Finished(Ok(())) => iced::exit(),
            Message::Finished(Err(failure)) => {
                self.finishing = false;
                match failure {
                    Failure::Ai(reason) => self.error = Some(reason),
                    Failure::Queue(reason) => {
                        self.error = Some(tr!("welcome-downloads-failed", "reason" => reason));
                    }
                    Failure::Record(reason) => {
                        self.record_failed = true;
                        self.error = Some(tr!("welcome-record-failed", "reason" => reason));
                    }
                }
                read_displays()
            }
            Message::CloseAnyway => iced::exit(),
        }
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        enz_app_shell(move || self.body())
            .layout(ShellLayout::TitlebarContent)
            .titlebar(move || {
                enz_title_bar()
                    .center(self.progress_indicator())
                    .show_close(false)
                    .id("welcome-titlebar")
                    .into_element()
            })
            .into_element()
    }

    fn progress_indicator(&self) -> Element<'_, Message> {
        let steps = [
            ("desktop", tr("welcome-step-desktop")),
            ("appearance", tr("welcome-step-appearance")),
            ("windows", tr("welcome-step-windows")),
            ("ai", tr("welcome-step-ai")),
            ("shortcuts", tr("welcome-step-shortcuts")),
        ];
        let current = self.wizard.current();
        let pages = steps
            .into_iter()
            .enumerate()
            .map(|(index, (id, label))| {
                SwitcherPage::new(id, label).with_enabled(self.wizard.is_reached(index))
            })
            .collect();
        enz_view_switcher(pages, current, |step| {
            Message::Wizard(WizardEvent::GoTo(step))
        })
        .aria_label(enzui_core::localized!(
            "enzui-wizard-step",
            "current" => (current + 1).to_string(),
            "total" => self.wizard.steps().to_string()
        ))
        .scrollable(true)
        .tab_id_prefix("welcome-progress")
        .into_element()
    }

    fn body(&self) -> Element<'_, Message> {
        let spacing = self.themes.theme().spacing;
        let current = self.wizard.current();
        let page = column![
            self.heading(),
            match current {
                DESKTOP => self.desktop_page(),
                APPEARANCE => self.appearance_page(),
                WINDOWS => self.windows_page(),
                AI => self.ai_page(),
                _ => self.shortcuts_page(),
            }
        ]
        .spacing(spacing.space5);
        let scrolled = enz_scroll_view(
            container(
                container(page).width(Length::Fill.max(enzui_theme::text_scale::ui_px(720.0))),
            )
            .center_x(Length::Fill),
        )
        .height(Length::Fill)
        .into_element();
        // Notices stay above the footer so a scale countdown is never scrolled away.
        let mut content = column![scrolled].spacing(spacing.space4);
        if let Some(pending) = &self.scale {
            let left = pending
                .deadline
                .saturating_duration_since(self.now)
                .as_secs()
                + 1;
            content = content.push(
                enz_banner(tr("welcome-scale-confirm"))
                    .body(tr!("welcome-scale-reverting", "seconds" => left.to_string()))
                    .tone(BannerTone::Warning)
                    .action_text(tr("welcome-keep"))
                    .on_action(Message::KeepScale)
                    .secondary_action(tr("welcome-revert"), Message::RevertScale),
            );
        }
        if let Some(error) = &self.error {
            let mut banner = enz_banner(error.clone()).tone(BannerTone::Danger);
            if self.record_failed {
                banner = banner
                    .action_text(tr("welcome-close-anyway"))
                    .on_action(Message::CloseAnyway);
            }
            content = content.push(banner);
        }
        let (label, event) = if self.wizard.is_last() {
            (
                enzui_core::localized!("enzui-wizard-finish"),
                WizardEvent::Finish,
            )
        } else {
            (
                enzui_core::localized!("enzui-wizard-next"),
                WizardEvent::Next,
            )
        };
        let footer = row![
            enz_button(
                enzui_core::localized!("enzui-wizard-cancel"),
                Message::Wizard(WizardEvent::Cancel),
            )
            .kind(ButtonKind::Quiet)
            .id("welcome-cancel".into()),
            Space::new().width(Length::Fill),
            enz_button(
                enzui_core::localized!("enzui-wizard-back"),
                Message::Wizard(WizardEvent::Back),
            )
            .kind(ButtonKind::Secondary)
            .id("welcome-back".into())
            .enabled(self.wizard.can_go_back()),
            enz_button(label, Message::Wizard(event))
                .kind(ButtonKind::Accent)
                .id("welcome-forward".into())
                .enabled(self.live() && self.scale.is_none() && !self.adding),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let wizard = column![
            fade_on_change(current, content.height(Length::Fill)),
            footer,
        ]
        .spacing(16);
        container(container(wizard).width(Length::Fill.max(enzui_theme::text_scale::ui_px(880.0))))
            .center_x(Length::Fill)
            .padding(spacing.space6)
            .height(Length::Fill)
            .into()
    }

    fn heading(&self) -> Element<'_, Message> {
        let (title, subtitle) = match self.wizard.current() {
            DESKTOP => ("welcome-title", "welcome-subtitle"),
            APPEARANCE => ("welcome-appearance-title", "welcome-appearance-subtitle"),
            WINDOWS => ("welcome-windows-title", "welcome-windows-subtitle"),
            AI => ("welcome-ai-title", "welcome-ai-subtitle"),
            _ => ("welcome-shortcuts-title", "welcome-shortcuts-subtitle"),
        };
        row![
            enzui_shell::brand::mark(40.0).into_element(),
            column![
                enz_text(tr(title)).size(TextSize::Title2).into_element(),
                enz_text(tr(subtitle))
                    .tone(TextTone::Muted)
                    .wrap(true)
                    .into_element(),
            ]
            .spacing(4),
        ]
        .spacing(16)
        .align_y(Alignment::Center)
        .into()
    }

    fn section<'a>(
        &self,
        title: &'static str,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        column![
            enz_text(tr(title)).size(TextSize::Headline).into_element(),
            content.into(),
        ]
        .spacing(10)
        .into()
    }

    fn card<'a>(
        &self,
        id: String,
        label: String,
        selected: bool,
        content: Element<'a, Message>,
        on_press: Message,
    ) -> Element<'a, Message> {
        enz_pressable(content, on_press)
            .style(PressableStyle::Card)
            .selected(selected)
            .selectable(true)
            .aria_label(label)
            .id(id.into())
            .padding(10)
            .radius(enzui_theme::text_defaults::shape_radius(|shape| shape.card))
            .border_width(1.0)
            .width(Length::Fill)
            .enabled(self.live())
            .into_element()
    }

    fn picture(svg_text: String, height: f32) -> Element<'static, Message> {
        svg(svg::Handle::from_memory(svg_text.into_bytes()))
            .width(Length::Fill)
            .height(height)
            .into()
    }

    fn caption(label: &str, detail: &str) -> Element<'static, Message> {
        column![
            enz_text(label.to_owned())
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .full_width()
                .align(TextAlign::Center)
                .into_element(),
            enz_text(detail.to_owned())
                .size(TextSize::Footnote)
                .tone(TextTone::Muted)
                .wrap(true)
                .full_width()
                .align(TextAlign::Center)
                .into_element(),
        ]
        .spacing(2)
        .into()
    }

    fn segments<'a, T: Copy + PartialEq + 'a>(
        &self,
        id: &'static str,
        options: impl IntoIterator<Item = (T, String)>,
        selected: Option<T>,
        on_press: impl Fn(T) -> Message,
        enabled: bool,
    ) -> Element<'a, Message> {
        let buttons = options
            .into_iter()
            .enumerate()
            .map(|(index, (value, label))| {
                let chosen = selected == Some(value);
                enz_button(label, on_press(value))
                    .kind(if chosen {
                        ButtonKind::Accent
                    } else {
                        ButtonKind::Secondary
                    })
                    .selected(chosen)
                    .role(enzui_core::semantics::Role::RadioButton)
                    .id(format!("{id}-{index}").into())
                    .enabled(enabled && self.live())
                    .into_element()
            });
        row(buttons).spacing(8).wrap().into()
    }

    fn desktop_page(&self) -> Element<'_, Message> {
        let palette = self.themes.theme().palette.clone();
        let current = self.settings.shell_style;
        let settings = &self.settings;
        let styles = responsive(move |size| -> Element<'_, Message> {
            let cards = [ShellStyle::Frame, ShellStyle::Islands].map(|style| {
                let description = match style {
                    ShellStyle::Frame => tr("shell-a-continuous-edge-with-joined-panels"),
                    ShellStyle::Islands => tr("shell-floating-pills-separate-or-connected"),
                };
                self.card(
                    format!("welcome-style-{}", style.key()),
                    style.label().to_owned(),
                    style == current,
                    column![
                        Self::picture(
                            scene::desktop(settings, &palette, style, false, false),
                            150.0,
                        ),
                        Self::caption(style.label(), description),
                    ]
                    .spacing(8)
                    .into(),
                    Message::Change(Change::ShellStyle(style)),
                )
            });
            if size.width >= 480.0 {
                row(cards).spacing(12).into()
            } else {
                column(cards).spacing(12).into()
            }
        });
        let mut body = column![self.section("welcome-layout", styles)].spacing(24);
        if current == ShellStyle::Islands {
            body = body.push(
                enz_preferences_row(tr("welcome-connected-islands"))
                    .subtitle(tr("welcome-connected-islands-description"))
                    .icon("link")
                    .switch(self.settings.islands.joined_bar, |joined| {
                        Message::Change(Change::IslandsJoinedBar(joined))
                    })
                    .enabled(self.live())
                    .id_prefix("welcome-joined-bar")
                    .into_element(),
            );
        }
        let bar_position = tr("welcome-bar-position");
        let bar_switcher = enz_view_switcher(
            vec![
                SwitcherPage::new("top", BarEdge::Top.label()),
                SwitcherPage::new("bottom", BarEdge::Bottom.label()),
            ],
            usize::from(self.settings.bar == BarEdge::Bottom),
            |index| Message::Change(Change::Bar(BarEdge::ALL[index])),
        )
        .aria_label(bar_position)
        .tab_id_prefix("welcome-bar")
        .enabled(self.live());
        body.push(
            enz_preferences_row(bar_position)
                .control(bar_switcher)
                .enabled(self.live())
                .into_element(),
        )
        .into()
    }

    fn appearance_page(&self) -> Element<'_, Message> {
        let choices = self.current_choices();
        let dark = self.mode() == EnzThemeMode::Dark;
        let modes = [false, true].map(|is_dark| {
            let mode = if is_dark {
                EnzThemeMode::Dark
            } else {
                EnzThemeMode::Light
            };
            let label = tr(if is_dark {
                "welcome-dark"
            } else {
                "welcome-light"
            });
            self.card(
                format!("welcome-mode-{}", if is_dark { "dark" } else { "light" }),
                label.to_owned(),
                dark == is_dark,
                column![
                    Self::picture(theme_preview::card(choices, mode, mode), 120.0),
                    enz_text(label)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                ]
                .spacing(8)
                .align_x(Alignment::Center)
                .into(),
                Message::Dark(is_dark),
            )
        });
        let mut swatches = row![].spacing(10).align_y(Alignment::Center);
        for (name, hex) in PRESETS {
            let color = theme_palette::rgb(hex);
            let seed = if hex == 0x0a84ff {
                Seed::Default
            } else {
                Seed::Color(color)
            };
            let selected = choices.scheme.seed == seed
                || (hex == 0x0a84ff && choices.scheme.seed == Seed::Color(color));
            swatches = swatches.push(
                enz_color_swatch(
                    color,
                    Message::Choose(Choices {
                        scheme: Scheme {
                            seed,
                            ..choices.scheme
                        },
                        ..choices
                    }),
                )
                .size(30.0)
                .radius(15.0)
                .selected(selected)
                .aria_label(name)
                .id_prefix(format!("welcome-accent-{}", name.to_lowercase()))
                .enabled(self.live())
                .into_element(),
            );
        }
        let mode = self.mode();
        let styles = Style::ALL.map(|style| {
            let scheme = Scheme {
                style,
                ..choices.scheme
            };
            self.card(
                format!("welcome-tone-{}", style.id_str()),
                tr!("shell-style-2", "arg0" => style.label().to_owned()),
                choices.scheme.style == style,
                column![
                    Self::picture(
                        theme_preview::tile(scheme, mode, choices.corners.rungs()[3]),
                        theme_preview::TILE_HEIGHT * 0.75,
                    ),
                    enz_text(style.label())
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                ]
                .spacing(6)
                .align_x(Alignment::Center)
                .into(),
                Message::Choose(Choices { scheme, ..choices }),
            )
        });
        let style_description = match choices.scheme.style {
            Style::Neutral => {
                tr("shell-quiet-grey-surfaces-the-colour-marks-buttons-selection-and-focus")
            }
            Style::Tinted => tr("shell-surfaces-and-lines-take-a-soft-wash-of-the-colour"),
            Style::Vibrant => tr("shell-rich-colourful-surfaces-and-a-saturated-accent"),
            Style::Contrast => {
                tr("shell-deeper-surfaces-brighter-text-and-stronger-lines-in-the-colour")
            }
        };
        column![
            self.section("welcome-mode", row(modes).spacing(12)),
            self.scale_setting(),
            self.section("welcome-accent", swatches.wrap()),
            self.section(
                "welcome-tone",
                column![
                    row(styles).spacing(8),
                    enz_text(style_description)
                        .tone(TextTone::Muted)
                        .wrap(true)
                        .into_element(),
                ]
                .spacing(8),
            ),
            self.section("welcome-text-size", self.text_size_setting()),
        ]
        .spacing(24)
        .into()
    }

    fn text_size_setting(&self) -> Element<'_, Message> {
        let selected = TextStep::nearest(self.settings.metrics.text);
        let index = TextStep::ALL
            .iter()
            .position(|step| *step == selected)
            .expect("the nearest text size is one of the steps");
        let thumb_width = 28.0;
        let mut samples = row![].align_y(Alignment::End);
        for (index, step) in TextStep::ALL.into_iter().enumerate() {
            if index > 0 {
                samples = samples.push(Space::new().width(Length::Fill));
            }
            samples = samples.push(
                enz_text("A")
                    .size_px(f32::from(step.px()))
                    .width(thumb_width)
                    .align(TextAlign::Center)
                    .tone(if selected == step {
                        TextTone::Default
                    } else {
                        TextTone::Muted
                    }),
            );
        }
        let slider = enz_slider(
            0.0..=(TextStep::ALL.len() - 1) as f32,
            index as f32,
            |value| {
                Message::Change(Change::Metric(
                    Metric::Text,
                    TextStep::ALL[value as usize].px(),
                ))
            },
        )
        .step(1.0)
        .thumb_width(thumb_width)
        .aria_label(tr("welcome-text-size"))
        .value_format(|value| TextStep::ALL[value as usize].description().to_owned())
        .id_prefix("welcome-text")
        .enabled(self.live());
        column![samples, slider].spacing(4).into()
    }

    fn scale_setting(&self) -> Element<'_, Message> {
        let row = enz_preferences_row(tr("welcome-scale")).icon("zoom-in");
        let primary = self
            .displays
            .as_ref()
            .filter(|displays| displays.supported)
            .and_then(|displays| displays.outputs.iter().find(|output| output.enabled))
            .and_then(|output| Some((output.current_mode()?, output.scale)));
        let Some((mode, current)) = primary else {
            return row.subtitle(tr("welcome-scale-unavailable")).into_element();
        };
        let options = SCALES
            .into_iter()
            .filter(|&scale| {
                (scale - current).abs() < 1e-6 || (scale >= 1.0 && supports_scale(mode, scale))
            })
            .map(|scale| (format!("{:.0}%", scale * 100.0), Message::Scale(scale)))
            .collect();
        row.id_prefix("welcome-scale")
            .value_menu(
                format!("{:.0}%", current * 100.0),
                self.scale_menu_open,
                Message::ScaleMenu(true),
                Message::ScaleMenu(false),
                options,
            )
            .enabled(self.live() && self.scale.is_none())
            .into_element()
    }

    fn shortcut(&self, id: &str) -> Option<&str> {
        let action = shortcut_map::action(id)?;
        let chord = shortcut_map::chord(action, &self.settings.compositor.shortcuts);
        (!chord.is_empty()).then_some(chord)
    }

    fn windows_page(&self) -> Element<'_, Message> {
        let current = self.settings.tiling.unwrap_or(false);
        let palette = &self.themes.theme().palette;
        let settings = &self.settings;
        let cards = responsive(move |size| -> Element<'_, Message> {
            let cards = [true, false].map(|tiled| {
                let (id, title, detail) = if tiled {
                    ("tiled", tr("welcome-tiled"), tr("welcome-tiled-detail"))
                } else {
                    (
                        "floating",
                        tr("welcome-floating"),
                        tr("welcome-floating-detail"),
                    )
                };
                self.card(
                    format!("welcome-windows-{id}"),
                    format!("{title}. {detail}"),
                    tiled == current,
                    column![
                        Self::picture(
                            scene::desktop(settings, palette, settings.shell_style, tiled, false),
                            126.0,
                        ),
                        Self::caption(title, detail),
                    ]
                    .spacing(8)
                    .into(),
                    Message::Change(Change::Tiling(tiled)),
                )
            });
            if size.width >= enzui_theme::text_scale::ui_px(448.0) {
                row(cards).spacing(12).into()
            } else {
                column(cards).spacing(12).into()
            }
        });
        let help = self.shortcut("desktop-layout").map_or_else(
            || tr("welcome-windows-change").to_owned(),
            |chord| tr!("welcome-windows-change-key", "shortcut" => self.key_labels.badge(chord)),
        );
        column![
            cards,
            enz_text(help)
                .tone(TextTone::Muted)
                .wrap(true)
                .into_element(),
        ]
        .spacing(18)
        .into()
    }

    fn shortcuts_page(&self) -> Element<'_, Message> {
        let bindings = shortcut_map::important(&self.settings.compositor);
        let rows = column(bindings.into_iter().map(|binding| {
            let caps = self.key_labels.caps(&binding.chord);
            let badge = caps.join(" + ");
            let keys = row(caps
                .into_iter()
                .map(|key| enz_key_badge(key).into_element()))
            .spacing(4)
            .wrap();
            row![
                enz_text(binding.name)
                    .wrap(true)
                    .full_width()
                    .into_element(),
                SemanticsContainer::leaf(keys, Semantics::new(Role::Label).label(badge)),
            ]
            .spacing(16)
            .align_y(Alignment::Center)
            .padding([4, 0])
            .into()
        }))
        .spacing(4);
        let note = self.shortcut("shortcuts").map_or_else(
            || tr("welcome-shortcuts-settings").to_owned(),
            |chord| tr!("welcome-shortcuts-overlay", "shortcut" => self.key_labels.badge(chord)),
        );
        column![
            rows,
            enz_text(note)
                .tone(TextTone::Muted)
                .wrap(true)
                .into_element(),
        ]
        .spacing(18)
        .into()
    }

    fn ai_page(&self) -> Element<'_, Message> {
        let enable = enz_preferences_row(tr("welcome-ai-enable"))
            .subtitle(tr("welcome-ai-enable-description"))
            .icon("ai.assistant")
            .switch(self.ai, Message::AiEnabled)
            .enabled(!self.finishing)
            .id_prefix("welcome-ai")
            .into_element();
        let mut content = column![enable].spacing(24);
        if self.ai {
            content = content
                .push(self.models(ModelKind::Text))
                .push(self.models(ModelKind::Speech))
                .push(
                    enz_text(tr("welcome-downloads-note"))
                        .tone(TextTone::Muted)
                        .wrap(true)
                        .into_element(),
                );
        }
        content
            .push(self.section("welcome-provider", self.provider_setting()))
            .into()
    }

    fn provider_setting(&self) -> Element<'_, Message> {
        let presets = self.segments(
            "welcome-provider",
            PROVIDERS
                .iter()
                .enumerate()
                .map(|(index, (_, label))| (index, (*label).to_owned())),
            Some(self.provider),
            Message::Provider,
            !self.adding,
        );
        let key = enz_secure_field(
            &self.key.0,
            |text| Message::Key(Secret(text)),
            self.reveal,
            Message::Reveal,
        )
        .label(tr("welcome-provider-key"))
        .aria_label(
            tr!("welcome-provider-key-for", "name" => PROVIDERS[self.provider].1.to_owned()),
        )
        .id_prefix("welcome-provider-key")
        .enabled(!self.adding && !self.finishing)
        .on_submit(Message::AddProvider)
        .into_element();
        let add = enz_button(
            tr(if self.adding {
                "welcome-provider-adding"
            } else {
                "welcome-provider-add"
            }),
            Message::AddProvider,
        )
        .icon("plus")
        .enabled(!self.adding && !self.finishing && !self.key.0.trim().is_empty())
        .id("welcome-provider-add".into())
        .into_element();
        let mut body = column![
            enz_text(tr("welcome-provider-description"))
                .tone(TextTone::Muted)
                .wrap(true)
                .into_element(),
            presets,
            row![container(key).width(Length::Fill), add]
                .spacing(8)
                .align_y(Alignment::End),
        ]
        .spacing(10);
        for name in &self.added {
            let label = PROVIDERS
                .iter()
                .find(|(id, _)| id == name)
                .map_or(name.as_str(), |(_, label)| label);
            body = body.push(
                row![
                    enzui_widgets::enz_icon("check").size(14.0).into_element(),
                    enz_text(tr!("welcome-provider-added", "name" => label.to_owned()))
                        .into_element(),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        body.into()
    }

    fn models(&self, kind: ModelKind) -> Element<'_, Message> {
        let (title, selected, on_choose): (_, _, fn(Option<String>) -> Message) = match kind {
            ModelKind::Text => ("welcome-text-model", &self.text_model, Message::TextModel),
            ModelKind::Speech => (
                "welcome-voice-model",
                &self.speech_model,
                Message::SpeechModel,
            ),
        };
        let mut rows = column![].spacing(8);
        if kind == ModelKind::Speech && local_ai::streaming_speech_installed() {
            rows = rows.push(
                enz_text(tr("welcome-streaming-english-included"))
                    .tone(TextTone::Muted)
                    .wrap(true)
                    .into_element(),
            );
        }
        let offers: Vec<&ModelOffer> = self
            .catalog
            .iter()
            .flatten()
            .filter(|offer| offer.kind == kind)
            .collect();
        if offers.is_empty() {
            let note = if self.catalog.is_none() {
                "welcome-models-reading"
            } else {
                "welcome-models-unavailable"
            };
            return self.section(
                title,
                rows.push(
                    enz_text(tr(note))
                        .tone(TextTone::Muted)
                        .wrap(true)
                        .into_element(),
                ),
            );
        }
        let prefix = match kind {
            ModelKind::Text => "welcome-text-model",
            ModelKind::Speech => "welcome-voice-model",
        };
        rows = rows.push(self.card(
            format!("{prefix}-none"),
            tr("welcome-model-none").to_owned(),
            selected.is_none(),
            Self::model_caption(tr("welcome-model-none"), String::new()),
            on_choose(None),
        ));
        for offer in offers {
            let detail = if offer.added {
                tr!("welcome-model-installed", "size" => offer.size.clone())
            } else {
                tr!("welcome-model-download", "size" => offer.size.clone())
            };
            rows = rows.push(self.card(
                format!("{prefix}-{}", offer.id),
                format!("{}, {detail}", offer.title),
                selected.as_deref() == Some(offer.id.as_str()),
                Self::model_caption(&offer.title, detail),
                on_choose(Some(offer.id.clone())),
            ));
        }
        self.section(title, rows)
    }

    fn model_caption(title: &str, detail: String) -> Element<'static, Message> {
        row![
            enz_text(title.to_owned())
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            iced::widget::space().width(Length::Fill),
            enz_text(detail).tone(TextTone::Muted).into_element(),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }
}

fn read_displays() -> Task<Message> {
    Task::perform(
        blocking(display::read, Displays::default()),
        Message::DisplaysRead,
    )
}

pub(crate) fn main() -> std::process::ExitCode {
    let launch = Launch::from_args(std::env::args().skip(1));
    let completed = welcome::record_path().is_some_and(|path| welcome::completed(&path));
    if !welcome::opens(launch, welcome::Session::from_environment(), completed) {
        return std::process::ExitCode::SUCCESS;
    }
    let Some(store) = Store::from_environment() else {
        eprintln!("enzui-welcome: no shell settings directory is configured");
        return std::process::ExitCode::from(2);
    };
    use enzui_widgets::app_shell::DesktopApplicationIdentity;
    let identity = DesktopApplicationIdentity::new(welcome::APP_ID).expect("a valid app id");
    let window = identity.apply_to_window_settings(iced::window::Settings {
        size: iced::Size::new(760.0, 680.0),
        min_size: Some(iced::Size::new(480.0, 520.0)),
        exit_on_close_request: false,
        ..identity.window_settings(ShellLayout::TitlebarContent)
    });
    let boot = move || Welcome::new(store.clone());
    match enzui_widgets::app_setup::run_with_services(
        iced::application(boot, Welcome::update, Welcome::view)
            .title(Welcome::title)
            .window(window)
            .theme(Welcome::theme)
            .subscription(Welcome::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("enzui-welcome: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
