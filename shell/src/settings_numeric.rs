use super::{Message as Parent, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    chrome_style::Change as ChromeChange,
    dock::Change as DockChange,
    notifications::preferences::Change as NotificationChange,
    search::preferences::Change as SearchChange,
    settings::{Change, Settings},
};
use iced::{Task, window};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug)]
pub(super) struct Spec {
    pub key: String,
    pub epoch: u64,
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub unit: String,
    pub template: Change,
}
impl Spec {
    pub(super) fn input_id(&self) -> String {
        format!("numeric-input-{}-{}", self.epoch, self.key)
    }
    fn field_width(&self) -> f32 {
        let digits = self
            .min
            .to_string()
            .len()
            .max(self.max.to_string().len())
            .max(2);
        // The number and its unit sit together: a wide field left the unit
        // stranded at its far edge. The floor holds three digits and "px",
        // so the sliders of neighbouring rows start and end together.
        (30.0 + digits as f32 * 7.5 + self.unit.trim().chars().count() as f32 * 6.5).max(66.0)
    }
    pub(super) fn current(&self, settings: &Settings) -> i32 {
        match self.template {
            Change::Dock(DockChange::IconSize(_)) => settings.dock.icon_size.into(),
            Change::Dock(DockChange::Spacing(_)) => settings.dock.spacing.into(),
            Change::Dock(DockChange::Radius(_)) => settings.dock.radius.into(),
            Change::Dock(DockChange::HighlightRadius(_)) => settings.dock.highlight_radius.into(),
            Change::Dock(DockChange::Margin(_)) => settings.dock.margin.into(),
            Change::Metric(key, _) => settings.metrics.get(key).into(),
            Change::Island(key, _) => settings.islands.get(key).into(),
            Change::FullscreenPadding(_) => settings.sidebar_behavior.fullscreen_padding.into(),
            Change::Compositor(key, _) => settings.compositor.value(key),
            Change::Search(SearchChange::Rows(_)) => settings.quick_search.rows.into(),
            Change::Search(SearchChange::ClipboardMinutes(_)) => {
                settings.quick_search.clipboard_minutes.into()
            }
            Change::Notification(NotificationChange::Duration(_)) => {
                (settings.notifications.policy.toast_ms / 1000) as i32
            }
            Change::Notification(NotificationChange::Width(_)) => {
                settings.notifications.width.into()
            }
            Change::Notification(NotificationChange::MaximumToasts(_)) => {
                settings.notifications.maximum_toasts.into()
            }
            Change::Notification(NotificationChange::HistoryMinutes(_)) => {
                settings.notifications.policy.history_minutes as i32
            }
            Change::Notification(NotificationChange::HistoryMiB(_)) => {
                settings.notifications.policy.history_mib as i32
            }
            Change::NightLight(enzui_shell::night_light::Change::Temperature(_)) => {
                settings.night_light.temperature.into()
            }
            Change::Capture(enzui_shell::capture::Change::JpegQuality(_)) => {
                settings.capture.jpeg_quality.into()
            }
            Change::Chrome(ChromeChange::Opacity(_)) => settings.chrome.opacity.into(),
            Change::Chrome(ChromeChange::GlassTint(_)) => {
                settings.chrome.glass_tint.map_or(self.value, i32::from)
            }
            Change::Chrome(ChromeChange::GlassBlur(_)) => {
                settings.chrome.glass_blur.map_or(self.value, i32::from)
            }
            Change::Chrome(ChromeChange::GlassBending(_)) => {
                settings.chrome.glass_bending.map_or(self.value, i32::from)
            }
            Change::Chrome(ChromeChange::GlassHighlight(_)) => settings
                .chrome
                .glass_highlight
                .map_or(self.value, i32::from),
            Change::Capture(enzui_shell::capture::Change::PngCompression(_)) => {
                settings.capture.png_compression.into()
            }
            _ => unreachable!("Registered numeric setting"),
        }
    }
    pub(super) fn change(&self, value: i32) -> Result<Change, String> {
        if !(self.min..=self.max).contains(&value) {
            return Err(
                tr!("shell-use-a-whole-number-from-to-2", "arg0" => format!("{:}", self.min), "arg1" => format!("{:}", self.max), "arg2" => self.unit.clone()),
            );
        }
        Ok(match self.template {
            Change::Dock(DockChange::IconSize(_)) => {
                Change::Dock(DockChange::IconSize(value as u16))
            }
            Change::Dock(DockChange::Spacing(_)) => Change::Dock(DockChange::Spacing(value as u16)),
            Change::Dock(DockChange::Radius(_)) => Change::Dock(DockChange::Radius(value as u16)),
            Change::Dock(DockChange::HighlightRadius(_)) => {
                Change::Dock(DockChange::HighlightRadius(value as u16))
            }
            Change::Dock(DockChange::Margin(_)) => Change::Dock(DockChange::Margin(value as u16)),
            Change::Metric(key, _) => Change::Metric(key, value as u16),
            Change::Island(key, _) => Change::Island(key, value as u16),
            Change::FullscreenPadding(_) => Change::FullscreenPadding(value as u16),
            Change::Compositor(key, _) => Change::Compositor(key, value),
            Change::Search(SearchChange::Rows(_)) => {
                Change::Search(SearchChange::Rows(value as u8))
            }
            Change::Search(SearchChange::ClipboardMinutes(_)) => {
                Change::Search(SearchChange::ClipboardMinutes(value as u16))
            }
            Change::Notification(NotificationChange::Duration(_)) => {
                Change::Notification(NotificationChange::Duration(value as u32 * 1000))
            }
            Change::Notification(NotificationChange::Width(_)) => {
                Change::Notification(NotificationChange::Width(value as u16))
            }
            Change::Notification(NotificationChange::MaximumToasts(_)) => {
                Change::Notification(NotificationChange::MaximumToasts(value as u8))
            }
            Change::Notification(NotificationChange::HistoryMinutes(_)) => {
                Change::Notification(NotificationChange::HistoryMinutes(value as u32))
            }
            Change::Notification(NotificationChange::HistoryMiB(_)) => {
                Change::Notification(NotificationChange::HistoryMiB(value as u32))
            }
            Change::NightLight(enzui_shell::night_light::Change::Temperature(_)) => {
                Change::NightLight(enzui_shell::night_light::Change::Temperature(value as u16))
            }
            Change::Chrome(ChromeChange::Opacity(_)) => {
                Change::Chrome(ChromeChange::Opacity(value as u8))
            }
            Change::Chrome(ChromeChange::GlassTint(_)) => {
                Change::Chrome(ChromeChange::GlassTint(Some(value as u8)))
            }
            Change::Chrome(ChromeChange::GlassBlur(_)) => {
                Change::Chrome(ChromeChange::GlassBlur(Some(value as u8)))
            }
            Change::Chrome(ChromeChange::GlassBending(_)) => {
                Change::Chrome(ChromeChange::GlassBending(Some(value as u8)))
            }
            Change::Chrome(ChromeChange::GlassHighlight(_)) => {
                Change::Chrome(ChromeChange::GlassHighlight(Some(value as u8)))
            }
            Change::Capture(enzui_shell::capture::Change::JpegQuality(_)) => {
                Change::Capture(enzui_shell::capture::Change::JpegQuality(value as u8))
            }
            Change::Capture(enzui_shell::capture::Change::PngCompression(_)) => {
                Change::Capture(enzui_shell::capture::Change::PngCompression(value as u8))
            }
            _ => unreachable!("Registered numeric setting"),
        })
    }
    pub(super) fn has_slider(&self) -> bool {
        !matches!(
            self.template,
            Change::Search(SearchChange::ClipboardMinutes(_))
                | Change::Notification(NotificationChange::HistoryMinutes(_))
        )
    }
}

#[derive(Clone, Debug)]
pub(super) enum Pointer {
    Mouse(bool),
    Touch(u64, bool),
    Lost,
    Escape,
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    Input(Spec, Arc<AtomicU64>, String),
    Submit(Spec, Arc<AtomicU64>),
    Edit(Spec),
    StartText(Spec, String),
    Text(u64, String),
    Slide(Spec, i32),
    Live(u64),
    Apply(u64, u64, i32),
    Cancel(u64),
    Settle(u64),
    Pointer(window::Id, Pointer),
}
#[derive(Clone, Debug)]
pub(super) struct Editor {
    pub spec: Spec,
    pub session: u64,
    pub revision: u64,
    pub text: String,
    pub error: Option<String>,
}
struct Pending {
    spec: Spec,
    value: i32,
    ticket: u64,
}
#[derive(Default)]
pub(super) struct State {
    pub editor: Option<Editor>,
    pending: Option<Pending>,
    generation: u64,
    pub epoch: u64,
    blocked: bool,
    press_settings: Option<Settings>,
    // The setting a held drag has already applied, and the value it wrote.
    live: Option<(String, i32)>,
    live_armed: bool,
    mouse: bool,
    touches: BTreeSet<u64>,
    pub focus_pending: Option<u64>,
    pub reveal_pending: Option<String>,
}
impl State {
    fn field_id(&self, spec: &Spec) -> String {
        self.editor
            .as_ref()
            .filter(|editor| editor.spec.key == spec.key)
            .map_or_else(|| spec.input_id(), |editor| editor.spec.input_id())
    }
    fn next(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.generation
    }
    pub(super) fn cancel(&mut self) {
        self.editor = None;
        self.cancel_gesture();
    }
    pub(super) fn close(&mut self) {
        self.mouse = false;
        self.touches.clear();
        self.press_settings = None;
        self.cancel();
    }
    fn cancel_gesture(&mut self) {
        self.pending = None;
        self.live = None;
        self.focus_pending = None;
        self.reveal_pending = None;
        self.epoch = self.epoch.wrapping_add(1);
        self.blocked = self.held();
        self.next();
    }
    fn control_enabled(&self, spec: &Spec) -> bool {
        !self.blocked
            && self
                .press_settings
                .as_ref()
                .is_none_or(|settings| spec.current(settings) == spec.value)
    }
    fn held(&self) -> bool {
        self.mouse || !self.touches.is_empty()
    }
    pub(super) fn display(&self, spec: &Spec) -> i32 {
        self.pending
            .as_ref()
            .filter(|p| p.spec.key == spec.key && p.spec.value == spec.value)
            .map_or(spec.value, |p| p.value)
    }
    // Whether `spec` still describes the setting: unchanged since it was
    // drawn, or changed only by the drag that holds it.
    fn current(&self, spec: &Spec, settings: &Settings) -> bool {
        let now = spec.current(settings);
        now == spec.value
            || self
                .live
                .as_ref()
                .is_some_and(|(key, value)| *key == spec.key && *value == now)
    }
    pub(super) fn flush(&mut self, settings: &Settings) -> Option<Change> {
        if self.held() {
            return None;
        }
        let pending = self.pending.take()?;
        let current = self.current(&pending.spec, settings);
        self.live = None;
        (current && pending.spec.current(settings) != pending.value)
            .then(|| pending.spec.change(pending.value).ok())
            .flatten()
    }
    // The value a held drag has reached, applied while the pointer is still down.
    fn live_change(&mut self, settings: &Settings) -> Option<Change> {
        let pending = self.pending.as_ref().filter(|_| self.held())?;
        if !self.current(&pending.spec, settings) || pending.spec.current(settings) == pending.value
        {
            return None;
        }
        let change = pending.spec.change(pending.value).ok()?;
        self.live = Some((pending.spec.key.clone(), pending.value));
        Some(change)
    }
}

pub(super) fn subscription() -> iced::Subscription<Parent> {
    // Slider presses are captured; ignored-only events would commit a draft
    // while its pointer is still held.
    iced::event::listen_with(|event, _, id| {
        let event = match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
                Pointer::Mouse(true)
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Pointer::Mouse(false)
            }
            iced::Event::Touch(iced::touch::Event::FingerPressed { id, .. }) => {
                Pointer::Touch(id.0, true)
            }
            iced::Event::Touch(iced::touch::Event::FingerLifted { id, .. }) => {
                Pointer::Touch(id.0, false)
            }
            iced::Event::Touch(iced::touch::Event::FingerLost { .. })
            | iced::Event::Window(window::Event::Unfocused | window::Event::Resized(_)) => {
                Pointer::Lost
            }
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) => Pointer::Escape,
            _ => return None,
        };
        Some(Parent::Numeric(Message::Pointer(id, event)))
    })
}

impl SettingsApp {
    pub(super) fn numeric_view<'a>(
        &'a self,
        mut spec: Spec,
        title: &'static str,
    ) -> iced::Element<'a, Parent> {
        // A held drag applies its value as it moves; the control keeps the
        // identity it was pressed with, so the drag is not reset under it.
        if let Some(pressed) = &self.numeric.press_settings {
            spec.value = spec.current(pressed);
        }
        use enzui_widgets::{
            button::{ButtonKind, ButtonSize, enz_button},
            enz_text,
            slider::enz_slider,
            text_field::{TextFieldSize, enz_text_field_owned as enz_text_field},
            tokens::FieldVariant,
        };
        use iced::{
            Element, Length,
            widget::{column, container, row},
        };
        let palette = self.tokens().palette;
        let backdrop = if self.highlighted == Some(spec.key.as_str()) {
            palette.accent_soft
        } else {
            palette.background
        };
        let displayed = self.numeric.display(&spec);
        let editor = self
            .numeric
            .editor
            .as_ref()
            .filter(|e| e.spec.key == spec.key);
        let actions: Option<Element<'a, Parent>> = editor.map(|editor| {
            let session = editor.session;
            row![
                enz_text(tr("shell-enter-to-apply-esc-to-cancel"))
                    .size_px(self.text_px(11.0))
                    .muted_on(backdrop)
                    .width(Length::Fill)
                    .into_element(),
                enz_button(
                    tr("shell-cancel"),
                    Parent::Numeric(Message::Cancel(session))
                )
                .kind(ButtonKind::Quiet)
                .size(ButtonSize::Sm)
                .aria_label(enzui_shell::tr!("shell-a11y-cancel-edit", "arg0" => enzui_shell::i18n::argument(&(title))))
                .id(format!("numeric-cancel-{session}").into())
                .enabled(self.controls_live())
                .into_element(),
                enz_button(
                    enzui_shell::i18n::tr("shell-apply"),
                    Parent::Numeric(Message::Apply(session, editor.revision, spec.value))
                )
                .size(ButtonSize::Sm)
                .icon("check")
                .aria_label(enzui_shell::tr!("shell-a11y-apply", "arg0" => enzui_shell::i18n::argument(&(title))))
                .id(format!("numeric-apply-{session}-{}-{}", editor.revision, spec.value).into())
                .enabled(self.controls_live())
                .into_element(),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into()
        });
        let text = editor.map_or_else(|| displayed.to_string(), |editor| editor.text.clone());
        let source = Arc::new(AtomicU64::new(editor.map_or(0, |editor| editor.session)));
        let input_source = std::sync::Arc::clone(&source);
        let edit_spec = spec.clone();
        let submit = Parent::Numeric(Message::Submit(spec.clone(), source));
        let value = container(
            enz_text_field("", "0", text, move |text| {
                Parent::Numeric(Message::Input(
                    edit_spec.clone(),
                    std::sync::Arc::clone(&input_source),
                    text,
                ))
            })
            .aria_label(
                tr!("shell-exact-value", "arg0" => title.to_owned(), "arg1" => spec.unit.clone()),
            )
            .id(self.numeric.field_id(&spec).into())
            .quiet(true)
            .variant(FieldVariant::Filled)
            .size(TextFieldSize::Sm)
            .adjustable_value(enzui_core::semantics::NumericValue {
                value: editor
                    .and_then(|editor| editor.text.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .unwrap_or_else(|| f64::from(displayed))
                    .clamp(f64::from(spec.min), f64::from(spec.max)),
                min: f64::from(spec.min),
                max: f64::from(spec.max),
                step: Some(1.0),
            })
            .on_submit(submit)
            .trailing(
                enz_text(spec.unit.trim().to_owned())
                    .size_px(self.text_px(11.0))
                    .muted_on(palette.surface_alt)
                    .into_element(),
            )
            .enabled(self.controls_live())
            .into_element(),
        )
        .width(spec.field_width());
        let control: Element<'a, Parent> = if spec.has_slider() {
            let slider_spec = spec.clone();
            let slider = enz_slider(
                spec.min as f32..=spec.max as f32,
                displayed as f32,
                move |value| {
                    Parent::Numeric(Message::Slide(slider_spec.clone(), value.round() as i32))
                },
            )
            .step(1.0)
            .step_marks(false)
            .show_value(false)
            // The row names it, like every other preferences row.
            .aria_label(title)
            .typography(self.tokens().typography)
            .id(format!("numeric-slider-{}-{}", spec.key, spec.value).into())
            .enabled(self.controls_live() && self.numeric.control_enabled(&spec))
            .into_element();
            // A slider ID does not reset its retained drag state.
            enzui_widgets::keyed_column::keyed_column(vec![(
                self.numeric.epoch.to_string(),
                slider,
            )])
            .into()
        } else {
            iced::widget::space().into()
        };
        // The same row every other setting uses, so name, icon and height
        // match; the slider and the exact value are its control.
        let trailing: Element<'a, Parent> = if spec.has_slider() {
            // The value keeps its own width inside one slot, so a wider
            // field (four digits) does not push its slider out of line.
            let slot = container(value)
                .width(spec.field_width().max(74.0))
                .align_x(iced::alignment::Horizontal::Right);
            row![container(control).width(240.0), slot]
                .spacing(12)
                .align_y(iced::Alignment::Center)
                .into()
        } else {
            value.into()
        };
        let line = crate::settings_app::settings_row(title)
            .icon(enzui_shell::settings_catalog::icon(&spec.key).unwrap_or(""))
            .control(trailing)
            .into_element();
        let mut extra = column![].spacing(6);
        let mut detailed = false;
        if let Some(actions) = actions {
            extra = extra.push(actions);
            detailed = true;
        }
        if let Some(error) = editor.and_then(|e| e.error.as_ref()) {
            extra = extra.push(
                enz_text(error)
                    .size_px(self.text_px(12.0))
                    .tone_on(enzui_widgets::text::TextTone::Danger, backdrop)
                    .into_element(),
            );
            detailed = true;
        }
        if detailed {
            column![
                line,
                container(extra).padding(iced::Padding {
                    top: 0.0,
                    right: 12.0,
                    bottom: 6.0,
                    left: 46.0,
                })
            ]
            .width(Length::Fill)
            .into()
        } else {
            line
        }
    }

    pub(super) fn numeric_message(&mut self, message: Message) -> Task<Parent> {
        if self.closing {
            return Task::none();
        }
        match message {
            Message::Input(spec, source, text) => {
                let session = source.load(Ordering::Relaxed);
                if session != 0 {
                    if self.numeric.editor.as_ref().is_some_and(|editor| {
                        editor.session == session && editor.spec.key == spec.key
                    }) {
                        return self.numeric_message(Message::Text(session, text));
                    }
                    return Task::none();
                }
                if self
                    .numeric
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.spec.key == spec.key)
                {
                    return Task::none();
                }
                let key = spec.key.clone();
                let task = self.numeric_message(Message::StartText(spec, text));
                if let Some(editor) = &self.numeric.editor
                    && editor.spec.key == key
                {
                    source.store(editor.session, Ordering::Relaxed);
                }
                return task;
            }
            Message::Submit(spec, source) => {
                let session = source.load(Ordering::Relaxed);
                if session == 0
                    && self
                        .numeric
                        .editor
                        .as_ref()
                        .is_none_or(|editor| editor.spec.key != spec.key)
                {
                    let key = spec.key.clone();
                    let task = self.numeric_message(Message::Edit(spec));
                    if let Some(editor) = &self.numeric.editor
                        && editor.spec.key == key
                    {
                        source.store(editor.session, Ordering::Relaxed);
                    }
                    return task;
                }
                if let Some(editor) = &self.numeric.editor
                    && editor.session == session
                    && editor.spec.key == spec.key
                {
                    return self.numeric_message(Message::Apply(
                        session,
                        editor.revision,
                        spec.value,
                    ));
                }
                return Task::none();
            }
            Message::Edit(spec)
                if spec.epoch == self.numeric.epoch
                    && spec.current(&self.settings) == spec.value =>
            {
                self.numeric.pending = None;
                self.numeric.reveal_pending = None;
                let session = self.numeric.next();
                self.numeric.editor = Some(Editor {
                    text: spec.value.to_string(),
                    spec,
                    session,
                    revision: 0,
                    error: None,
                });
                self.numeric.focus_pending = Some(session);
            }
            Message::StartText(spec, value)
                if spec.epoch == self.numeric.epoch
                    && spec.current(&self.settings) == spec.value
                    && self
                        .numeric
                        .editor
                        .as_ref()
                        .is_none_or(|e| e.spec.key != spec.key) =>
            {
                self.numeric.pending = None;
                self.numeric.focus_pending = None;
                self.numeric.reveal_pending = None;
                let session = self.numeric.next();
                self.numeric.editor = Some(Editor {
                    text: value.chars().take(16).collect(),
                    spec,
                    session,
                    revision: 1,
                    error: None,
                });
            }
            Message::Text(session, value) => {
                if let Some(editor) = &mut self.numeric.editor
                    && editor.session == session
                {
                    editor.text = value.chars().take(16).collect();
                    editor.revision = editor.revision.wrapping_add(1);
                    editor.error = None;
                }
            }
            Message::Slide(spec, value)
                if spec.epoch == self.numeric.epoch
                    && self.numeric.control_enabled(&spec)
                    && self.numeric.current(&spec, &self.settings) =>
            {
                if spec.change(value).is_err() {
                    return Task::none();
                }
                let ticket = self.numeric.next();
                self.numeric.editor = None;
                self.numeric.focus_pending = None;
                self.numeric.reveal_pending = None;
                self.numeric.pending = Some(Pending {
                    spec,
                    value,
                    ticket,
                });
                let settle = Task::perform(
                    async { tokio::time::sleep(std::time::Duration::from_millis(80)).await },
                    move |()| Parent::Numeric(Message::Settle(ticket)),
                );
                // A held drag shows its effect as it moves, a few times a second.
                if self.numeric.held() && !self.numeric.live_armed {
                    self.numeric.live_armed = true;
                    let epoch = self.numeric.epoch;
                    return Task::batch([
                        settle,
                        Task::perform(
                            async {
                                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                            },
                            move |()| Parent::Numeric(Message::Live(epoch)),
                        ),
                    ]);
                }
                return settle;
            }
            Message::Live(epoch) => {
                self.numeric.live_armed = false;
                if epoch == self.numeric.epoch
                    && let Some(change) = self.numeric.live_change(&self.settings)
                {
                    return self.update(Parent::Change(change));
                }
            }
            Message::Apply(session, revision, expected) => {
                if let Some(editor) = &mut self.numeric.editor
                    && editor.session == session
                    && editor.revision == revision
                {
                    let change = if editor.spec.current(&self.settings) != expected {
                        Err(
                            tr("shell-this-value-changed-elsewhere-review-it-and-apply-again")
                                .into(),
                        )
                    } else {
                        editor
                            .text
                            .trim()
                            .parse::<i32>()
                            .map_err(|_| {
                                tr!("shell-use-a-whole-number-from-to-2", "arg0" => format!("{:}", editor.spec.min), "arg1" => format!("{:}", editor.spec.max), "arg2" => editor.spec.unit.clone())
                            })
                            .and_then(|value| editor.spec.change(value))
                    };
                    match change {
                        Ok(change) => {
                            self.numeric.cancel();
                            return self.update(Parent::Change(change));
                        }
                        Err(error) => {
                            editor.error = Some(error);
                            self.numeric.reveal_pending = Some(editor.spec.key.clone());
                        }
                    }
                }
            }
            Message::Cancel(session) => {
                if self
                    .numeric
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.session == session)
                {
                    self.numeric.cancel();
                }
            }
            Message::Settle(ticket) => {
                if self
                    .numeric
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.ticket == ticket)
                    && let Some(change) = self.numeric.flush(&self.settings)
                {
                    return self.update(Parent::Change(change));
                }
            }
            Message::Pointer(id, pointer) if self.window == Some(id) => {
                let held = self.numeric.held();
                match pointer {
                    Pointer::Mouse(value) => self.numeric.mouse = value,
                    Pointer::Touch(id, true) => {
                        if self.numeric.touches.len() < 16 {
                            self.numeric.touches.insert(id);
                        } else {
                            self.numeric.cancel_gesture();
                        }
                    }
                    Pointer::Touch(id, false) => {
                        self.numeric.touches.remove(&id);
                    }
                    Pointer::Lost => {
                        self.numeric.mouse = false;
                        self.numeric.touches.clear();
                        self.numeric.press_settings = None;
                        self.numeric.cancel_gesture();
                        return Task::none();
                    }
                    Pointer::Escape => {
                        // Escape gives up the drag, and what it had applied so far.
                        let revert = self.numeric.pending.as_ref().and_then(|pending| {
                            self.numeric
                                .live
                                .as_ref()
                                .filter(|(key, value)| {
                                    *key == pending.spec.key
                                        && pending.spec.current(&self.settings) == *value
                                })
                                .and_then(|_| pending.spec.change(pending.spec.value).ok())
                        });
                        self.numeric.cancel();
                        return revert
                            .map_or_else(Task::none, |change| self.update(Parent::Change(change)));
                    }
                }
                if !held && self.numeric.held() {
                    self.numeric.press_settings = Some(self.settings.clone());
                }
                if !self.numeric.held() {
                    self.numeric.press_settings = None;
                    self.numeric.blocked = false;
                }
                if let Some(change) = self.numeric.flush(&self.settings) {
                    return self.update(Parent::Change(change));
                }
            }
            _ => {}
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzui_shell::settings::{Metric, Store};

    fn spec() -> Spec {
        Spec {
            key: "sizing/text_size".into(),
            epoch: 0,
            value: 13,
            min: 10,
            max: 20,
            unit: " px".into(),
            template: Change::Metric(Metric::Text, 13),
        }
    }
    fn app() -> (tempfile::TempDir, SettingsApp, window::Id) {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = SettingsApp::new(Store::at(root.path().join("settings.json")).unwrap());
        let window = window::Id::unique();
        app.window = Some(window);
        (root, app, window)
    }
    #[test]
    fn settings_numeric_behaviour() {
        // Stale settles and peer changes cannot commit an old drag.
        {
            let (_root, mut app, id) = app();
            let _ = app.numeric_message(Message::Pointer(id, Pointer::Mouse(true)));
            let _ = app.numeric_message(Message::Slide(spec(), 14));
            let old = app.numeric.pending.as_ref().unwrap().ticket;
            let _ = app.numeric_message(Message::Slide(spec(), 15));
            let _ = app.numeric_message(Message::Settle(old));
            assert_eq!(app.numeric.display(&spec()), 15);
            app.settings
                .preview(Change::Metric(Metric::Text, 18))
                .unwrap();
            let _ = app.numeric_message(Message::Pointer(id, Pointer::Mouse(false)));
            assert_eq!(app.settings.metrics.text, 18);
            assert!(!app.busy);
        }
        // Numeric text is retained until valid explicit apply.
        {
            let (_root, mut app, _) = app();
            let _ = app.numeric_message(Message::Edit(spec()));
            let session = app.numeric.editor.as_ref().unwrap().session;
            let _ = app.numeric_message(Message::Text(session, "invalid".into()));
            let revision = app.numeric.editor.as_ref().unwrap().revision;
            let _ = app.numeric_message(Message::Apply(session, revision, 13));
            assert_eq!(app.settings.metrics.text, 13);
            assert_eq!(app.numeric.editor.as_ref().unwrap().text, "invalid");
            assert!(app.numeric.editor.as_ref().unwrap().error.is_some());
            assert_eq!(
                app.numeric.reveal_pending.as_deref(),
                Some("sizing/text_size")
            );
            let _ = app.numeric_message(Message::Text(session, "1".into()));
            let _ = app.numeric_message(Message::Text(session, "15".into()));
            let _ = app.numeric_message(Message::Apply(session, revision, 13));
            assert_eq!(app.settings.metrics.text, 13);
            let revision = app.numeric.editor.as_ref().unwrap().revision;
            let _ = app.numeric_message(Message::Apply(session, revision, 13));
            assert_eq!(app.settings.metrics.text, 15);
            assert!(app.numeric.editor.is_none());
        }
        // Close flushes settling keyboard edits but not a held drag.
        {
            let (_root, mut keyboard, _) = app();
            let _ = keyboard.numeric_message(Message::Slide(spec(), 14));
            let _ = keyboard.update(Parent::Close);
            assert_eq!(keyboard.settings.metrics.text, 14);
            assert!(keyboard.closing && keyboard.busy);
            let (_root, mut pointer, id) = app();
            let _ = pointer.numeric_message(Message::Pointer(id, Pointer::Mouse(true)));
            let _ = pointer.numeric_message(Message::Slide(spec(), 14));
            let _ = pointer.update(Parent::Close);
            assert_eq!(pointer.settings.metrics.text, 13);
            assert!(!pointer.busy);
            assert!(pointer.numeric.pending.is_none());
        }
    }
}
