mod close;

use super::{Message, Shell, window_settings};
use close::{CloseState, caption_line};
use enzui_core::semantics::{Role, Semantics};
use enzui_shell::dock::{Geometry as DockGeometry, Position as DockPosition};
use enzui_shell::i18n::tr;
use enzui_shell::search::compositor::SpreadWindow;
use enzui_shell::spread_layout::{self, Rect};
use enzui_widgets::{
    clickable::enz_clickable,
    focus_button::focus_button,
    icon::enz_icon,
    text::{enz_text, line_box_px},
};
use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{column, container, image, mouse_area, pin, stack};
use iced::{Element, Length, Padding, Subscription, Task, window};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[path = "spread_motion.rs"]
mod motion;

// Floating-desktop window overview using captures while the compositor hides the original
// windows.
const DURATION: f32 = 0.26;
const MARGIN: f32 = 32.0;
const GAP: f32 = 28.0;
const CAPTION_TEXT: f32 = 13.0;
const CAPTION_GAP: f32 = 8.0;
const CAPTION_PADDING_Y: f32 = 5.0;
const RING: f32 = 5.0;
const CLOSE_SIZE: f32 = 32.0;
const WINDOW_KEYS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
// The overview's desktops strip: room between it, Search and the windows.
const STRIP_GAP: f32 = 18.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Loading,
    Opening,
    Open,
    Closing,
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    window: SpreadWindow,
    // Overlay-local: where the window is, and where the spread puts it.
    from: Rect,
    to: Rect,
    picture: Option<image::Handle>,
    shortcut: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(crate) struct Loaded {
    desktop: i64,
    // The compositor's window corner radius, which the pictures take on.
    rounding: f32,
    area: [f32; 4],
    entries: Vec<Entry>,
    insets: Padding,
    strip: Option<Rect>,
}

/// Room the overview keeps on the bar's side for Search's resting card; the
/// desktops strip goes beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverviewBand {
    pub search: f32,
}

#[derive(Clone, Copy, Debug)]
struct MotionFrames {
    first: Instant,
    latest: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct Spread {
    phase: Phase,
    generation: u64,
    load_task: Option<iced::task::Handle>,
    window: Option<window::Id>,
    desktop: i64,
    rounding: f32,
    area: [f32; 4],
    entries: Vec<Entry>,
    selected: Option<u64>,
    hovered: Option<u64>,
    insets: Padding,
    close: CloseState,
    chosen: Option<usize>,
    started: Option<Instant>,
    frames: Cell<Option<MotionFrames>>,
    // The originals are being hidden; the motion starts once they are.
    hiding: bool,
    // How far out the windows were when closing began.
    closing_from: f32,
    duration: f32,
    // The window to focus once the overlay has closed: closing hands focus
    // on, so focusing earlier would be undone.
    focus_after: Option<(window::Id, u64)>,
    allocations: Vec<image::Allocation>,
    ready: bool,
    ready_frames: u8,
    // The overview's desktops strip, overlay-local; Search keeps the keyboard.
    strip: Option<Rect>,
}

#[derive(Clone, Debug)]
pub(crate) enum SpreadMessage {
    Toggle,
    Overview(OverviewBand),
    Desktop(u32),
    Loaded(u64, Result<Loaded, String>),
    Allocated(u64, Vec<image::Allocation>),
    Presented(window::Id),
    Hidden(u64, Result<(), String>),
    MotionFinished(window::Id, Instant),
    Hover(u64),
    Leave(u64),
    Select(u64),
    Pick(u64),
    Close(u64),
    CloseRequested(u64, u64, Result<(), String>),
    LiveWindows(u64, Result<std::collections::HashSet<u64>, String>),
    Dismiss,
    Key(Named),
    Restored(u64, Result<(), String>),
    Finished(u64),
    Focused(Result<(), String>),
}

fn assign_window_keys(entries: &mut [Entry]) {
    entries.iter_mut().for_each(|entry| entry.shortcut = None);
    let mut by_age: Vec<_> = (0..entries.len()).collect();
    by_age.sort_unstable_by_key(|&index| entries[index].window.stable_id);
    for (index, key) in by_age.into_iter().zip(WINDOW_KEYS) {
        entries[index].shortcut = Some(key);
    }
}

fn spread_navigation_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            Named::ArrowLeft | Named::ArrowRight | Named::ArrowUp | Named::ArrowDown | Named::Enter
        )
    )
}

pub(crate) fn activities_key_candidate(key: &Key) -> bool {
    spread_navigation_key(key)
        || matches!(key, Key::Named(Named::Delete))
        || matches!(key, Key::Character(text) if matches!(text.as_bytes(), [b'0'..=b'9']))
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Spread {
    pub(crate) fn owns(&self, id: window::Id) -> bool {
        self.window == Some(id)
    }

    pub(crate) fn closes(&self, id: window::Id) -> bool {
        self.owns(id)
            || self
                .focus_after
                .as_ref()
                .is_some_and(|(closed, _)| *closed == id)
    }

    fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    fn progress(&self, now: Instant) -> f32 {
        let elapsed = self.frames.get().map_or(0.0, |frames| {
            now.saturating_duration_since(frames.first).as_secs_f32()
        });
        let t = if self.duration <= 0.0 {
            1.0
        } else {
            elapsed / self.duration
        };
        match self.phase {
            Phase::Idle | Phase::Loading => 0.0,
            Phase::Opening => {
                if self.started.is_some() {
                    ease(t)
                } else {
                    0.0
                }
            }
            Phase::Open => 1.0,
            // A finished close keeps the windows' places until the overlay goes.
            Phase::Closing => {
                if self.started.is_some() {
                    self.closing_from * (1.0 - ease_in_out(t))
                } else {
                    0.0
                }
            }
        }
    }

    fn sampled_at(&self, now: Instant) -> Instant {
        self.frames.get().map_or(now, |frames| frames.latest)
    }

    fn motion_finished(&self, at: Instant) -> bool {
        self.frames.get().is_some_and(|frames| {
            let duration = if self.phase == Phase::Open {
                DURATION
            } else {
                self.duration
            };
            at >= frames.first && at.duration_since(frames.first).as_secs_f32() >= duration
        })
    }

    fn sample_frame(&self, at: Instant) -> bool {
        if self.started.is_none_or(|armed| at < armed) {
            return false;
        }
        let previous = self.frames.get();
        let finished = previous.is_some_and(|frames| self.motion_finished(frames.latest));
        self.frames.set(Some(MotionFrames {
            first: previous.map_or(at, |frames| frames.first),
            latest: at,
        }));
        !finished && self.motion_finished(at)
    }

    fn animating(&self) -> bool {
        self.started.is_some()
            && !self
                .frames
                .get()
                .is_some_and(|frames| self.motion_finished(frames.latest))
    }
}

// The strip and the bands the windows keep clear, overlay-local; the strip
// sits beside Search on the bar's side.
fn overview_bands(
    area: [f32; 4],
    insets: Padding,
    band: OverviewBand,
    bottom: bool,
) -> (Rect, Padding) {
    // A tenth of the output, between a readable preview and the windows' room.
    let height = (area[3] * 0.1).clamp(56.0, 104.0);
    let y = if bottom {
        area[3] - insets.bottom - band.search - STRIP_GAP - height
    } else {
        insets.top + band.search + STRIP_GAP
    };
    let strip = Rect::new(
        insets.left + MARGIN,
        y,
        (area[2] - insets.x() - 2.0 * MARGIN).max(1.0),
        height,
    );
    let clear = height + band.search + STRIP_GAP;
    let insets = if bottom {
        insets.bottom(insets.bottom + clear)
    } else {
        insets.top(insets.top + clear)
    };
    (strip, insets)
}

fn arrange(windows: &[SpreadWindow], area: [f32; 4], insets: Padding) -> Vec<(Rect, Rect)> {
    let from: Vec<Rect> = windows
        .iter()
        .map(|w| Rect::new(w.at[0] - area[0], w.at[1] - area[1], w.size[0], w.size[1]))
        .collect();
    let to = spread_places(&from, area, insets);
    from.into_iter().zip(to).collect()
}

fn spread_places(from: &[Rect], area: [f32; 4], insets: Padding) -> Vec<Rect> {
    let room = Rect::new(
        insets.left + MARGIN,
        insets.top + MARGIN,
        (area[2] - insets.x() - 2.0 * MARGIN).max(1.0),
        (area[3] - insets.y() - 2.0 * MARGIN).max(1.0),
    );
    spread_layout::layout(from, room, GAP, caption_height() + CAPTION_GAP)
}

fn caption_height() -> f32 {
    let text = enzui_theme::text_scale::effective().apply_px(CAPTION_TEXT);
    (line_box_px(text).max(20.0) + 2.0 * CAPTION_PADDING_Y).max(CLOSE_SIZE)
}

// Capture at the resting tile size to limit uploads.
fn picture_pixels(to: Rect, scale: f32) -> (u32, u32) {
    let scale = if scale.is_finite() {
        scale.clamp(1.0, 4.0)
    } else {
        1.0
    };
    let pixels = |logical: f32| ((logical * scale).ceil() as u32).clamp(1, 8192);
    (pixels(to.width), pixels(to.height))
}

async fn load(
    root: std::path::PathBuf,
    compositor: enzui_shell::compositor::Compositor,
    metrics: enzui_shell::settings::Metrics,
    bar: enzui_shell::settings::BarEdge,
    overview: Option<OverviewBand>,
    dock: Option<(DockPosition, DockGeometry)>,
) -> Result<Loaded, String> {
    let desktop = compositor.spread_windows(overview.is_some()).await?;
    // The overview opens on an empty desktop too, for Search and the desktops.
    if desktop.windows.is_empty() && overview.is_none() {
        return Err(tr("shell-no-windows-to-spread").into());
    }
    let area = desktop.output;
    let band = f32::from(metrics.bar_band());
    if area[3] < band + 1.0 {
        return Err(tr("shell-invalid-spread-geometry").into());
    }
    let (top, bottom) = if bar == enzui_shell::settings::BarEdge::Top {
        (band, 0.0)
    } else {
        (0.0, band)
    };
    let mut insets = Padding::ZERO.top(top).bottom(bottom);
    if let Some((position, dock)) = dock.filter(|(_, dock)| dock.output_id == desktop.output_id) {
        let body = dock.body;
        insets = match position {
            DockPosition::Left => insets.left(body.x + body.width),
            DockPosition::Right => insets.right(area[2] - body.x),
            DockPosition::Top => insets.top(top.max(body.y + body.height)),
            DockPosition::Bottom => insets.bottom(bottom.max(area[3] - body.y)),
            DockPosition::Bar => insets,
        };
    }
    let (strip, insets) = match overview {
        Some(band) => {
            let (strip, bands) = overview_bands(
                area,
                insets,
                band,
                bar == enzui_shell::settings::BarEdge::Bottom,
            );
            (Some(strip), bands)
        }
        None => (None, insets),
    };
    let places = arrange(&desktop.windows, area, insets);
    let capture = iced::futures::future::join_all(desktop.windows.iter().zip(&places).map(
        |(window, (_, to))| {
            let (w, h) = picture_pixels(*to, desktop.scale);
            enzui_shell::previews::capture_window(root.clone(), window.address.clone(), w, h)
        },
    ));
    let (placed, (rounding, _), pictures) = iced::futures::join!(
        compositor.prepare_spread(&desktop, overview.is_some()),
        compositor.window_rounding(),
        capture,
    );
    placed?;
    let mut entries: Vec<_> = desktop
        .windows
        .into_iter()
        .zip(places)
        .zip(pictures)
        .map(|((window, (from, to)), picture)| Entry {
            window,
            from,
            to,
            picture: picture.ok(),
            shortcut: None,
        })
        .collect();
    if overview.is_some() {
        assign_window_keys(&mut entries);
    }
    Ok(Loaded {
        desktop: desktop.desktop,
        rounding,
        area,
        entries,
        insets,
        strip,
    })
}

fn tile_picture<'a>(
    entry: &'a Entry,
    r: Rect,
    radius: f32,
    palette: &enzui_theme::Palette,
) -> Element<'a, Message> {
    let surface = palette.surface;
    let text = palette.text;
    match &entry.picture {
        // On the surface colour: a window whose frame the compositor
        // paints (glass) leaves that frame clear in its picture.
        Some(handle) => container(
            image(handle.clone())
                .width(r.width)
                .height(r.height)
                .content_fit(iced::ContentFit::Fill)
                .border_radius(radius),
        )
        .width(r.width)
        .height(r.height)
        .style(move |_| container::Style {
            background: Some(surface.into()),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into(),
        None => {
            let (name, icon) = enzui_shell::search::catalogue::app_identity(&entry.window.class);
            container(
                column![
                    match icon {
                        Some(path) => enzui_shell::icon_theme::source(&path).into_element(48.0),
                        None => enz_icon("app-window").size(32.0).into_element(),
                    },
                    enz_text(name.unwrap_or_else(|| entry.window.class.clone()))
                        .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                        .single_line_elided()
                        .into_element()
                ]
                .spacing(8)
                .align_x(iced::Alignment::Center),
            )
            .center(Length::Fill)
            .width(r.width)
            .height(r.height)
            .style(move |_| container::Style {
                background: Some(surface.into()),
                border: iced::Border {
                    radius: radius.into(),
                    ..Default::default()
                },
                text_color: Some(text),
                ..Default::default()
            })
            .into()
        }
    }
}

fn tile_label(entry: &Entry) -> String {
    if entry.window.title.is_empty() {
        enzui_shell::search::catalogue::app_identity(&entry.window.class)
            .0
            .unwrap_or_else(|| entry.window.class.clone())
    } else {
        entry.window.title.clone()
    }
}

fn framed_picture<'a>(
    entry: &'a Entry,
    r: Rect,
    rounding: f32,
    selected: bool,
    palette: &enzui_theme::Palette,
) -> Element<'a, Message> {
    let radius = rounding * r.width / entry.from.width.max(1.0);
    let accent = palette.accent;
    container(tile_picture(entry, r, radius, palette))
        .padding(RING)
        .width(r.width + 2.0 * RING)
        .height(r.height + 2.0 * RING)
        .style(move |_| container::Style {
            border: iced::Border {
                radius: (radius + RING).into(),
                width: if selected { 3.0 } else { 0.0 },
                color: accent,
            },
            ..Default::default()
        })
        .into()
}

fn picture_button<'a>(
    picture: Element<'a, Message>,
    entry: &Entry,
    label: &str,
    overview: bool,
) -> Element<'a, Message> {
    let id = entry.window.stable_id;
    let activate = Message::Spread(SpreadMessage::Pick(id));
    if overview {
        let name = entry.shortcut.map_or_else(
            || label.to_owned(),
            |key| {
                enzui_shell::tr!(
                    "shell-overview-window-shortcut",
                    "title" => enzui_shell::i18n::argument(&label),
                    "key" => key
                )
            },
        );
        focus_button(picture, activate)
            .semantics(Semantics::new(Role::Button).label(name))
            .id(format!("overview-window-{id:x}").into())
            .style_focus(|_, _| Default::default())
            .into()
    } else {
        mouse_area(picture)
            .on_press(activate)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }
}

fn tile_caption<'a>(
    entry: &Entry,
    label: String,
    selected: bool,
    overview: bool,
    show_close: bool,
    palette: &enzui_theme::Palette,
) -> Element<'a, Message> {
    let icon = match enzui_shell::search::catalogue::app_identity(&entry.window.class).1 {
        Some(path) => enzui_shell::icon_theme::source(&path).into_element(18.0),
        None => enz_icon("app-window").size(16.0).into_element(),
    };
    let chip_fill = if selected {
        palette.accent_soft
    } else {
        palette.surface_elevated
    };
    let title = enz_text(label.clone())
        .size_px(enzui_theme::text_scale::effective().apply_px(CAPTION_TEXT))
        .weight_rung(if selected {
            enzui_theme::WeightRung::Semibold
        } else {
            enzui_theme::WeightRung::Medium
        })
        .single_line_elided()
        .into_element();
    let mut caption = iced::widget::row![icon, title]
        .spacing(6)
        .align_y(iced::Alignment::Center);
    if let Some(shortcut) = entry.shortcut {
        let accent = palette.accent;
        let badge = container(
            enz_text(shortcut)
                .size_px(12.0)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .color(palette.on_accent())
                .into_element(),
        )
        .center(Length::Fixed(20.0))
        .style(move |_| container::Style {
            background: Some(accent.into()),
            border: iced::Border {
                radius: 10.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        caption = caption.push(badge);
    }
    let text = palette.text;
    let chip = container(caption)
        .padding([CAPTION_PADDING_Y, 12.0])
        .width(Length::Shrink)
        .style(move |_| container::Style {
            background: Some(chip_fill.into()),
            border: iced::Border {
                radius: 999.0.into(),
                ..Default::default()
            },
            text_color: Some(text),
            ..Default::default()
        });
    if !overview {
        return chip.into();
    }

    let id = entry.window.stable_id;
    let mut controls = iced::widget::row![
        mouse_area(chip)
            .on_press(Message::Spread(SpreadMessage::Select(id)))
            .interaction(iced::mouse::Interaction::Pointer)
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center);
    if show_close {
        let name = enzui_shell::tr!(
            "shell-overview-close-window",
            "title" => enzui_shell::i18n::argument(&label)
        );
        controls = controls.push(
            enz_clickable(
                enz_icon("action.close").size(16.0).into_element(),
                Message::Spread(SpreadMessage::Close(id)),
            )
            .aria_label(name)
            .radius(CLOSE_SIZE / 2.0)
            .width(CLOSE_SIZE)
            .height(CLOSE_SIZE)
            .tokens(enzui_widgets::clickable::ClickableTokens {
                background: palette.surface_elevated,
                ..enzui_widgets::clickable::ClickableTokens::from_palette(palette)
            })
            .into_element(),
        );
    }
    controls.into()
}

impl Shell {
    pub(crate) fn spread_message(&mut self, message: SpreadMessage) -> Task<Message> {
        let generation = self.spread.generation;
        match message {
            SpreadMessage::Toggle => match self.spread.phase {
                // Floating and tiled desktops both spread (windows are only
                // made transparent while their pictures stand in); the
                // spread loads the windows itself.
                Phase::Idle => self.load_spread(None),
                Phase::Loading => {
                    self.spread = Spread {
                        generation: generation + 1,
                        ..Spread::default()
                    };
                    Task::none()
                }
                _ => self.close_spread(None),
            },
            SpreadMessage::Overview(band) if self.spread.phase == Phase::Idle => {
                self.load_spread(Some(band))
            }
            SpreadMessage::Overview(_) => Task::none(),
            SpreadMessage::Desktop(id)
                if self.desktops.as_ref().is_some_and(|d| d.active == id) =>
            {
                Task::none()
            }
            // The overview stays open: the spread ends here and reloads once
            // the chosen desktop is up.
            SpreadMessage::Desktop(id) => {
                let spread = self.end_spread();
                Task::batch([spread, self.overview_select_desktop(id)])
            }
            SpreadMessage::Loaded(sent, result)
                if sent == generation && self.spread.phase == Phase::Loading =>
            {
                self.spread.load_task = None;
                match result {
                    Ok(loaded) => {
                        self.spread.desktop = loaded.desktop;
                        self.spread.rounding = loaded.rounding;
                        self.spread.area = loaded.area;
                        self.spread.entries = loaded.entries;
                        self.spread.insets = loaded.insets;
                        self.spread.strip = loaded.strip;
                        self.open_spread_window()
                    }
                    Err(error) => {
                        self.spread = Spread {
                            generation,
                            ..Spread::default()
                        };
                        eprintln!("enzui-shell spread: {error}");
                        // Search, opened with it, stays on its own.
                        self.overview_spread_failed();
                        self.refresh_overview_search()
                    }
                }
            }
            SpreadMessage::Allocated(sent, allocations)
                if sent == generation && self.spread.phase == Phase::Opening =>
            {
                // A picture that cannot be uploaded still loads on its own.
                self.spread.allocations = allocations;
                self.spread.ready = true;
                self.spread_ready_frames()
            }
            SpreadMessage::Allocated(..) => Task::none(),
            SpreadMessage::Loaded(..) => Task::none(),
            SpreadMessage::Presented(id) => self.spread_presented(id),
            SpreadMessage::Hidden(sent, Err(error)) if sent == generation => {
                eprintln!("enzui-shell spread: {error}");
                self.close_spread(None)
            }
            SpreadMessage::Hidden(sent, Ok(())) if sent == generation => {
                // The motion starts once the originals are hidden: the
                // compositor's work to hide them stalls the first frames,
                // and a clock started before would jump.
                if self.spread.phase == Phase::Opening && self.spread.started.is_none() {
                    self.spread.started = Some(Instant::now());
                    self.spread.frames.set(None);
                }
                self.redraw_only = self.spread.window;
                Task::none()
            }
            SpreadMessage::Hidden(..) => Task::none(),
            SpreadMessage::MotionFinished(id, at)
                if self.spread.owns(id) && self.spread.motion_finished(at) =>
            {
                self.redraw_only = Some(id);
                match self.spread.phase {
                    Phase::Opening => {
                        self.spread.started = None;
                        self.spread.phase = Phase::Open;
                        if self.activities {
                            self.redraw_only = None;
                        }
                        Task::batch([
                            self.refresh_overview_search(),
                            self.check_spread_windows_if_dirty(),
                        ])
                    }
                    Phase::Open if self.spread.is_reflowing() => {
                        self.spread.finish_reflow_frame(at);
                        Task::none()
                    }
                    Phase::Closing => {
                        self.spread.started = None;
                        // The pick is raised now, under its picture; it is
                        // focused again once the overlay has gone.
                        let chosen = self
                            .spread
                            .chosen
                            .and_then(|index| self.spread.entries.get(index))
                            .map(|entry| entry.window.stable_id);
                        let compositor = self.compositor.clone();
                        Task::perform(
                            async move { compositor.spread_restore(chosen).await },
                            move |result| {
                                Message::Spread(SpreadMessage::Restored(generation, result))
                            },
                        )
                    }
                    _ => Task::none(),
                }
            }
            SpreadMessage::MotionFinished(..) => {
                self.quiet = true;
                Task::none()
            }
            SpreadMessage::Hover(id)
                if self.spread.phase == Phase::Open && !self.spread.is_reflowing() =>
            {
                self.spread.hovered = Some(id);
                if self.spread.strip.is_none() {
                    self.spread.selected = Some(id);
                }
                self.redraw_only = self.spread.window;
                Task::none()
            }
            SpreadMessage::Leave(id) if self.spread.hovered == Some(id) => {
                self.spread.hovered = None;
                self.redraw_only = self.spread.window;
                Task::none()
            }
            SpreadMessage::Select(id) if self.spread.live_index(id).is_some() => {
                self.spread.selected = Some(id);
                self.redraw_only = self.spread.window;
                Task::none()
            }
            SpreadMessage::Hover(_) | SpreadMessage::Leave(_) | SpreadMessage::Select(_) => {
                self.quiet = true;
                Task::none()
            }
            SpreadMessage::Pick(id) => self
                .spread
                .live_index(id)
                .map_or_else(Task::none, |index| self.close_spread(Some(index))),
            SpreadMessage::Close(id) => self.request_spread_close(id),
            SpreadMessage::CloseRequested(sent, id, result) if sent == generation => {
                self.spread_close_requested(id, result)
            }
            SpreadMessage::LiveWindows(sent, result) if sent == generation => {
                self.spread_windows_read(result)
            }
            SpreadMessage::CloseRequested(..) | SpreadMessage::LiveWindows(..) => Task::none(),
            SpreadMessage::Dismiss => self.close_spread(None),
            SpreadMessage::Key(key) => {
                self.redraw_only = self.spread.window;
                self.spread_key(key)
            }
            SpreadMessage::Restored(sent, result) if sent == generation => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                // The windows fade back in under the pictures before those go.
                Task::perform(
                    async { tokio::time::sleep(Duration::from_millis(140)).await },
                    move |()| Message::Spread(SpreadMessage::Finished(generation)),
                )
            }
            SpreadMessage::Restored(..) => Task::none(),
            SpreadMessage::Finished(sent) if sent == generation => self.end_spread(),
            SpreadMessage::Finished(_) => Task::none(),
            SpreadMessage::Focused(result) => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                Task::none()
            }
        }
    }

    fn load_spread(&mut self, overview: Option<OverviewBand>) -> Task<Message> {
        self.spread.generation += 1;
        self.spread.phase = Phase::Loading;
        let generation = self.spread.generation;
        let (root, compositor, metrics, bar) = (
            self.root.clone(),
            self.compositor.clone(),
            self.metrics,
            self.bar,
        );
        let dock = if overview.is_some() && self.dock_active() && self.dock.window.is_some() {
            self.dock
                .geometry
                .clone()
                .map(|geometry| (self.dock.preferences.position, geometry))
        } else {
            None
        };
        let (load, handle) = Task::perform(
            load(root, compositor, metrics, bar, overview, dock),
            move |result| Message::Spread(SpreadMessage::Loaded(generation, result)),
        )
        .abortable();
        self.spread.load_task = Some(handle.abort_on_drop());
        load
    }

    fn open_spread_window(&mut self) -> Task<Message> {
        let area = self.spread.area;
        let (id, open) = window::open(window_settings("dev.enzui.WindowSpread", area[2], area[3]));
        let reduced = self.window_tokens(None).motion.reduced_motion;
        self.spread.duration = if reduced {
            0.0
        } else {
            DURATION
                * enzui_shell::motion::duration_factor(
                    self.desired_settings.compositor.animation_rate,
                )
        };
        self.spread.window = Some(id);
        self.spread.selected = None;
        self.spread.ready = false;
        self.spread.phase = Phase::Opening;
        open.map(Message::Opened)
    }

    pub(crate) fn spread_opened(&mut self, id: window::Id) -> Task<Message> {
        if !self.spread.owns(id) || self.spread.phase != Phase::Opening {
            return Task::none();
        }
        let generation = self.spread.generation;
        let uploads: Vec<_> = self
            .spread
            .entries
            .iter()
            .filter_map(|entry| entry.picture.clone())
            .map(|picture| image::allocate_in(id, picture))
            .collect();
        // An empty batch never reports: the overview of an empty desktop has
        // nothing to upload.
        if uploads.is_empty() {
            return Task::done(Message::Spread(SpreadMessage::Allocated(
                generation,
                Vec::new(),
            )));
        }
        Task::batch(uploads).collect().map(move |results| {
            Message::Spread(SpreadMessage::Allocated(
                generation,
                results.into_iter().filter_map(Result::ok).collect(),
            ))
        })
    }

    // Leave one frame for the uploaded previews before hiding the originals.
    fn spread_ready_frames(&mut self) -> Task<Message> {
        let Some(id) = self.spread.window else {
            return Task::none();
        };
        self.spread.ready_frames = 2;
        self.redraw_only = Some(id);
        Task::none()
    }

    fn spread_presented(&mut self, id: window::Id) -> Task<Message> {
        if !self.spread.owns(id) || self.spread.ready_frames == 0 {
            self.quiet = true;
            return Task::none();
        }
        self.spread.ready_frames -= 1;
        self.redraw_only = Some(id);
        if self.spread.ready_frames != 0 {
            return Task::none();
        }
        self.begin_spread()
    }

    fn begin_spread(&mut self) -> Task<Message> {
        if self.spread.phase != Phase::Opening
            || self.spread.started.is_some()
            || std::mem::replace(&mut self.spread.hiding, true)
        {
            return Task::none();
        }
        let addresses: Vec<String> = self
            .spread
            .entries
            .iter()
            .map(|entry| entry.window.address.clone())
            .collect();
        let compositor = self.compositor.clone();
        let generation = self.spread.generation;
        Task::perform(
            async move { compositor.spread_hide(&addresses).await },
            move |result| Message::Spread(SpreadMessage::Hidden(generation, result)),
        )
    }

    pub(crate) fn close_spread(&mut self, chosen: Option<usize>) -> Task<Message> {
        let with_search = self.activities;
        let search = self.overview_spread_closing();
        let spread = match self.spread.phase {
            Phase::Opening if !self.spread.hiding => {
                self.spread.chosen = chosen.filter(|index| *index < self.spread.entries.len());
                self.end_spread()
            }
            Phase::Opening | Phase::Open => {
                let now = Instant::now();
                let sample = self.spread.sampled_at(now);
                self.spread.closing_from = self.spread.progress(sample);
                self.spread.freeze_reflow(sample);
                self.spread.phase = Phase::Closing;
                self.spread.chosen = chosen.filter(|index| *index < self.spread.entries.len());
                self.spread.started = Some(now);
                self.spread.frames.set(None);
                if !with_search {
                    self.redraw_only = self.spread.window;
                }
                Task::none()
            }
            Phase::Loading => {
                self.spread = Spread {
                    generation: self.spread.generation + 1,
                    ..Spread::default()
                };
                Task::none()
            }
            Phase::Idle | Phase::Closing => Task::none(),
        };
        Task::batch([search, spread])
    }

    fn end_spread(&mut self) -> Task<Message> {
        let window = self.spread.window.take();
        let chosen = self
            .spread
            .chosen
            .and_then(|index| self.spread.entries.get(index))
            .map(|entry| entry.window.stable_id);
        let focus_after = window.zip(chosen);
        self.spread = Spread {
            generation: self.spread.generation + 1,
            focus_after,
            ..Spread::default()
        };
        window.map_or_else(Task::none, window::close)
    }

    pub(crate) fn spread_closed(&mut self, id: window::Id) -> Task<Message> {
        if self.spread.owns(id) {
            return self.abort_spread();
        }
        let Some((_, stable_id)) = self.spread.focus_after.take_if(|(closed, _)| *closed == id)
        else {
            return Task::none();
        };
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.spread_restore(Some(stable_id)).await },
            |result| Message::Spread(SpreadMessage::Focused(result)),
        )
    }

    pub(crate) fn spread_desktop(&self) -> Option<i64> {
        self.spread.window.is_some().then_some(self.spread.desktop)
    }

    pub(crate) fn abort_spread(&mut self) -> Task<Message> {
        let search = self.overview_spread_closing();
        let spread = if self.spread.active() {
            self.end_spread()
        } else {
            Task::none()
        };
        Task::batch([search, spread])
    }

    pub(crate) fn spread_active(&self) -> bool {
        self.spread.active()
    }

    pub(crate) fn activities_window_key(
        &self,
        key: &Key,
        modifiers: Modifiers,
    ) -> Option<SpreadMessage> {
        if !self.activities
            || self.spread.phase != Phase::Open
            || self.spread.entries.is_empty()
            || modifiers.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::LOGO)
            || !self.search.accepts_overview_keys()
        {
            return None;
        }
        match key {
            Key::Character(digit) => self
                .spread
                .entries
                .iter()
                .find(|entry| {
                    entry.shortcut == Some(digit.as_str()) && !self.spread.entry_gone(entry)
                })
                .map(|entry| SpreadMessage::Pick(entry.window.stable_id)),
            Key::Named(Named::Delete) if modifiers.is_empty() => {
                self.spread.current_tile().map(SpreadMessage::Close)
            }
            Key::Named(named) if modifiers.is_empty() && spread_navigation_key(key) => {
                Some(SpreadMessage::Key(*named))
            }
            _ => None,
        }
    }

    fn spread_key(&mut self, key: Named) -> Task<Message> {
        if self.spread.phase != Phase::Open {
            return Task::none();
        }
        // The window that had focus is stacked last: the arrows start there,
        // and Enter with nothing chosen picks it.
        let Some(selected) = self
            .spread
            .current_tile()
            .and_then(|id| self.spread.live_index(id))
        else {
            return Task::none();
        };
        if matches!(key, Named::Enter | Named::Space) {
            return self.close_spread(Some(selected));
        }
        if key == Named::Delete {
            return self.request_spread_close(self.spread.entries[selected].window.stable_id);
        }
        let current = &self.spread.entries[selected];
        let center = |r: Rect| (r.x + r.width / 2.0, r.y + r.height / 2.0);
        let (cx, cy) = center(current.to);
        let best = self
            .spread
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !self.spread.entry_gone(entry))
            .filter_map(|(index, entry)| {
                let (x, y) = center(entry.to);
                let (dx, dy) = (x - cx, y - cy);
                let ahead = match key {
                    Named::ArrowLeft => -dx,
                    Named::ArrowRight => dx,
                    Named::ArrowUp => -dy,
                    Named::ArrowDown => dy,
                    _ => return None,
                };
                let aside = if matches!(key, Named::ArrowLeft | Named::ArrowRight) {
                    dy.abs()
                } else {
                    dx.abs()
                };
                (ahead > 1.0).then_some((index, ahead + 2.0 * aside))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        self.spread.selected = Some(
            self.spread.entries[best.map_or(selected, |(index, _)| index)]
                .window
                .stable_id,
        );
        Task::none()
    }

    pub(crate) fn spread_subscription(&self) -> Subscription<Message> {
        let Some(id) = self.spread.window else {
            return Subscription::none();
        };
        let keys = iced::event::listen_with_interest(
            iced::event::Interest::KEYBOARD,
            |event, status, window| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: ref key @ Key::Named(named),
                    modifiers,
                    repeat,
                    ..
                }) if status == iced::event::Status::Ignored
                    && (spread_navigation_key(key)
                        || named == Named::Delete && modifiers.is_empty() && !repeat) =>
                {
                    Some((window, named))
                }
                _ => None,
            },
        )
        .with(id)
        .filter_map(|(owner, (window, key))| {
            (owner == window).then_some(Message::Spread(SpreadMessage::Key(key)))
        });
        let presented = if self.spread.ready_frames > 0 {
            iced::event::listen_raw(|event, _, window| match event {
                iced::Event::Window(window::Event::RedrawRequested(_)) => Some(window),
                _ => None,
            })
            .with(id)
            .filter_map(|(owner, window)| {
                (owner == window).then_some(Message::Spread(SpreadMessage::Presented(window)))
            })
        } else {
            Subscription::none()
        };
        Subscription::batch([keys, presented])
    }

    // The desktops beside Search: each one's preview, the current one ringed,
    // and the empty one at the end as a small add button.
    fn overview_strip(&self, strip: Rect, palette: &enzui_theme::Palette) -> Element<'_, Message> {
        let p = palette.clone();
        let aspect = (self.spread.area[2] / self.spread.area[3].max(1.0)).clamp(0.5, 4.0);
        let height = (strip.height - 6.0).max(1.0);
        let width = height * aspect;
        let mut cards = iced::widget::row![]
            .spacing(14)
            .align_y(iced::Alignment::Center);
        if let Some(desktops) = &self.desktops {
            let count = desktops.entries.len();
            for (index, desktop) in desktops.entries.iter().enumerate() {
                let active = desktop.id == desktops.active;
                let spare = !active && desktop.windows == 0 && index + 1 == count && index > 0;
                let preview = self
                    .previews
                    .iter()
                    .find(|preview| preview.desktop == desktop.id);
                let size = if spare {
                    iced::Size::new((width * 0.45).max(44.0), height)
                } else {
                    iced::Size::new(width, height)
                };
                let picture: Element<'_, Message> = if spare {
                    container(
                        enz_icon("plus")
                            .size(20.0)
                            .tint(p.text_muted)
                            .into_element(),
                    )
                    .center_x(size.width)
                    .center_y(size.height)
                    .style(move |_| container::Style {
                        background: Some(iced::Color { a: 0.08, ..p.text }.into()),
                        border: iced::Border {
                            radius: 8.0.into(),
                            width: 1.0,
                            color: iced::Color { a: 0.22, ..p.text },
                        },
                        ..Default::default()
                    })
                    .into()
                } else if let Some(preview) = preview {
                    enzui_widgets::image::enz_image_handle(preview.image.clone())
                        .fit(enzui_widgets::image::EnzImageFit::Cover)
                        .width(Length::Fixed(size.width))
                        .height(Length::Fixed(size.height))
                        .radius(8.0)
                        .into_element()
                } else {
                    container(enz_icon("monitor").size(22.0).into_element())
                        .center_x(size.width)
                        .center_y(size.height)
                        .style(move |_| container::Style {
                            background: Some(p.surface_alt.into()),
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .into()
                };
                let mut tokens = enzui_widgets::clickable::ClickableTokens::from_palette(&p);
                tokens.background = iced::Color::TRANSPARENT;
                tokens.border = if active {
                    p.accent
                } else {
                    iced::Color::TRANSPARENT
                };
                cards = cards.push(
                    enzui_widgets::clickable::enz_clickable(
                        picture,
                        Message::Spread(SpreadMessage::Desktop(desktop.id)),
                    )
                    .aria_label(enzui_shell::tr!("workspace-caption", "number" => index + 1, "windows" => desktop.windows, "current" => if active { "yes" } else { "no" }))
                    .description(tr("shell-select-desktop"))
                    .id(format!("overview-desktop-{}", desktop.id).into())
                    .tokens(tokens)
                    .width(size.width + 6.0)
                    .height(size.height + 6.0)
                    .padding(3)
                    .radius(11.0)
                    .border_width(2.0)
                    .into_element(),
                );
            }
        }
        let from = if self.bar == enzui_shell::settings::BarEdge::Bottom {
            12.0
        } else {
            -12.0
        };
        let shown = enzui_widgets::motion::animated(
            container(cards).center_x(strip.width).height(strip.height),
        )
        .enters_from(iced::Vector::new(0.0, from), 0.0)
        .opacity(if self.spread.phase == Phase::Closing {
            0.0
        } else {
            1.0
        });
        pin(shown).x(strip.x).y(strip.y).into()
    }

    pub(crate) fn spread_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if !self.spread.owns(id) {
            return None;
        }
        let tokens = self.window_tokens(Some(id));
        let palette = tokens.palette;
        let scrim = mouse_area(motion::backdrop(&self.spread))
            .on_press(Message::Spread(SpreadMessage::Dismiss));
        let mut layers: Vec<Element<'_, Message>> = vec![scrim.into()];
        if self.spread.ready {
            if let Some(strip) = self.spread.strip {
                layers.push(self.overview_strip(strip, &palette));
            }
            if self.spread.phase == Phase::Open && !self.spread.is_reflowing() {
                self.resting_tiles(&palette, &mut layers);
            } else {
                layers.push(motion::tiles(&self.spread, id, &palette));
            }
        }
        Some(
            stack(layers)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        )
    }

    fn resting_tiles<'a>(
        &'a self,
        palette: &enzui_theme::Palette,
        layers: &mut Vec<Element<'a, Message>>,
    ) {
        let overview = self.spread.strip.is_some();
        let close_tile = self.spread.current_tile();
        let footer = caption_height();
        for entry in &self.spread.entries {
            let r = entry.to;
            let id = entry.window.stable_id;
            let selected = self.spread.selected == Some(id) || self.spread.hovered == Some(id);
            let picture = framed_picture(entry, r, self.spread.rounding, selected, palette);
            let label = tile_label(entry);
            let pick = picture_button(picture, entry, &label, overview);
            let chip = tile_caption(
                entry,
                label,
                selected,
                overview,
                selected || close_tile == Some(id),
                palette,
            );
            let hover = Message::Spread(SpreadMessage::Hover(id));
            let caption_y =
                caption_line(r, self.spread.entries.iter().map(|other| other.to)) + CAPTION_GAP;
            if overview {
                let width = r.width + 2.0 * RING;
                let height = caption_y - r.y + RING + footer;
                let tile = stack![
                    iced::widget::space().width(width).height(height),
                    pick,
                    pin(container(chip).center_x(width)).y(caption_y - r.y + RING),
                ]
                .width(width)
                .height(height);
                layers.push(
                    pin(mouse_area(tile)
                        .on_enter(hover)
                        .on_exit(Message::Spread(SpreadMessage::Leave(id))))
                    .x(r.x - RING)
                    .y(r.y - RING)
                    .into(),
                );
            } else {
                layers.push(
                    pin(mouse_area(pick).on_enter(hover))
                        .x(r.x - RING)
                        .y(r.y - RING)
                        .into(),
                );
                layers.push(
                    pin(container(chip).center_x(r.width.max(120.0)))
                        .x(r.x + r.width / 2.0 - r.width.max(120.0) / 2.0)
                        .y(caption_y)
                        .into(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_keys_follow_creation_order_and_leave_an_active_query_to_search() {
        use crate::quick_search::Message as SearchMessage;

        let compositor = enzui_shell::compositor::Compositor::new(
            std::path::PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, std::path::PathBuf::from("/")).0;
        let search = window::Id::unique();
        shell.activities = true;
        shell.search.showing = true;
        shell.search.overview = true;
        shell.search.started = true;
        shell.search.window = Some(search);
        shell.spread.phase = Phase::Open;
        shell.spread.entries = (1..=12)
            .rev()
            .map(|stable_id| {
                let rect = Rect::new(stable_id as f32 * 50.0, 0.0, 40.0, 40.0);
                Entry {
                    window: SpreadWindow {
                        address: format!("0x{stable_id:x}"),
                        stable_id,
                        pid: 2,
                        title: format!("Window {stable_id}"),
                        class: "fixture".into(),
                        at: [rect.x, rect.y],
                        size: [rect.width, rect.height],
                    },
                    from: rect,
                    to: rect,
                    picture: None,
                    shortcut: None,
                }
            })
            .collect();
        assign_window_keys(&mut shell.spread.entries);
        assert!(
            shell.spread.entries[..2]
                .iter()
                .all(|entry| entry.shortcut.is_none())
        );
        let digit = |shell: &Shell| {
            SearchMessage::Key(
                search,
                shell.search.model.generation,
                Key::Character("0".into()),
                Modifiers::empty(),
                false,
            )
        };
        shell.search.model.set_query("build".into());
        let _ = shell.search_message(digit(&shell));
        assert_eq!(shell.spread.phase, Phase::Open);

        shell.search.model.set_query(String::new());
        let _ = shell.search_message(digit(&shell));
        let picked = shell
            .spread
            .chosen
            .map(|index| shell.spread.entries[index].window.stable_id);
        assert_eq!(picked, Some(10));
        assert_eq!(shell.spread.phase, Phase::Closing);
        assert!(!shell.activities);
        assert!(
            shell.search.overview,
            "Search keeps its overview layout until hidden"
        );
    }

    #[test]
    fn the_originals_are_hidden_after_the_spread_draws_its_pictures() {
        let compositor = enzui_shell::compositor::Compositor::new(
            std::path::PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, std::path::PathBuf::from("/")).0;
        assert!(shell.snapshot.is_none());
        let _ = shell.spread_message(SpreadMessage::Toggle);
        assert!(shell.spread.phase == Phase::Loading);
        let generation = shell.spread.generation;
        let window = SpreadWindow {
            address: "0x1".into(),
            stable_id: 1,
            pid: 2,
            title: "Files".into(),
            class: "files".into(),
            at: [0.0, 0.0],
            size: [800.0, 600.0],
        };
        let place = Rect::new(0.0, 0.0, 800.0, 600.0);
        let _ = shell.spread_message(SpreadMessage::Loaded(
            generation,
            Ok(Loaded {
                desktop: 1,
                rounding: 8.0,
                area: [0.0, 0.0, 1280.0, 800.0],
                entries: vec![Entry {
                    window,
                    from: place,
                    to: place,
                    picture: Some(image::Handle::from_rgba(1, 1, vec![0; 4])),
                    shortcut: None,
                }],
                insets: Padding::ZERO,
                strip: None,
            }),
        ));
        let id = shell
            .spread
            .window
            .expect("the spread window opens with its places");
        assert!(shell.spread.phase == Phase::Opening);
        assert!(!shell.spread.ready);

        let _ = shell.spread_message(SpreadMessage::Presented(id));
        assert!(!shell.spread.hiding);
        let _ = shell.spread_message(SpreadMessage::Allocated(generation, Vec::new()));
        assert!(shell.spread.ready);
        let _ = shell.spread_message(SpreadMessage::Presented(id));
        let _ = shell.spread_message(SpreadMessage::Presented(window::Id::unique()));
        assert!(!shell.spread.hiding);
        let _ = shell.spread_message(SpreadMessage::Presented(id));
        assert!(shell.spread.hiding);

        shell.redraw_only = None;
        let _ = shell.spread_message(SpreadMessage::Hidden(generation, Ok(())));
        assert_eq!(shell.redraw_only, Some(id));
        shell.quiet = false;
        let _ = shell.spread_message(SpreadMessage::Hover(0));
        assert!(shell.quiet);
        shell.quiet = false;
        shell.redraw_only = None;
        let _ = shell.spread_message(SpreadMessage::MotionFinished(
            window::Id::unique(),
            Instant::now(),
        ));
        assert!(shell.quiet);
        assert_eq!(shell.redraw_only, None);
        let _ = shell.spread_message(SpreadMessage::Dismiss);
        assert!(shell.spread.phase == Phase::Closing);
        assert_eq!(shell.redraw_only, Some(id));
    }

    #[test]
    fn the_travel_follows_the_spread_frame_clock() {
        let compositor = enzui_shell::compositor::Compositor::new(
            std::path::PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, std::path::PathBuf::from("/")).0;
        let id = window::Id::unique();
        let started = Instant::now();
        shell.spread.window = Some(id);
        shell.spread.phase = Phase::Opening;
        shell.spread.hiding = true;
        shell.spread.duration = 10.0;
        shell.spread.started = Some(started);

        let first_frame = started + Duration::from_millis(100);
        assert!(!shell.spread.sample_frame(first_frame));
        assert_eq!(shell.spread.progress(first_frame), 0.0);
        let half = first_frame + Duration::from_secs(5);
        assert!(!shell.spread.sample_frame(half));
        assert!(shell.spread.phase == Phase::Opening);
        let shown = shell
            .spread
            .progress(shell.spread.sampled_at(Instant::now()));
        assert!((shown - ease(0.5)).abs() < 1e-4);

        let _ = shell.spread_message(SpreadMessage::Dismiss);
        assert!(shell.spread.phase == Phase::Closing);
        assert!((shell.spread.closing_from - shown).abs() < 1e-4);
        let restart = shell.spread.started.unwrap();
        assert!(
            (shell.spread.progress(restart) - shown).abs() < 1e-4,
            "the close begins where the opening was shown"
        );

        let first_frame = restart + Duration::from_millis(100);
        assert!(!shell.spread.sample_frame(first_frame));
        assert!((shell.spread.progress(first_frame) - shown).abs() < 1e-4);
        let end = first_frame + Duration::from_secs(11);
        assert!(shell.spread.sample_frame(end));
        assert!(!shell.spread.sample_frame(end));
        let _ = shell.spread_message(SpreadMessage::MotionFinished(id, end));
        assert!(
            shell.spread.started.is_none(),
            "a finished close restores the windows"
        );
    }
}
