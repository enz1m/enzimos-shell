use super::{Message, Shell, window_settings};
use enzui_shell::i18n::tr;
use enzui_shell::search::compositor::SpreadWindow;
use enzui_shell::spread_layout::{self, Rect};
use enzui_widgets::{icon::enz_icon, text::enz_text};
use iced::widget::{column, container, image, mouse_area, pin, stack};
use iced::{Element, Length, Subscription, Task, window};
use std::time::{Duration, Instant};

// Floating-desktop window overview using captures while the compositor hides the original
// windows.
const DURATION: f32 = 0.26;
const MARGIN: f32 = 32.0;
const GAP: f32 = 28.0;
const LABEL: f32 = 34.0;
const RING: f32 = 5.0;

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
}

#[derive(Clone, Debug)]
pub(crate) struct Loaded {
    desktop: i64,
    // The compositor's window corner radius, which the pictures take on.
    rounding: f32,
    area: [f32; 4],
    entries: Vec<Entry>,
}

#[derive(Debug, Default)]
pub(crate) struct Spread {
    phase: Phase,
    generation: u64,
    window: Option<window::Id>,
    desktop: i64,
    rounding: f32,
    area: [f32; 4],
    entries: Vec<Entry>,
    selected: Option<usize>,
    chosen: Option<usize>,
    started: Option<Instant>,
    // The originals are being hidden; the motion starts once they are.
    hiding: bool,
    // How far out the windows were when closing began.
    closing_from: f32,
    duration: f32,
    // The window to focus once the overlay has closed: closing hands focus
    // on, so focusing earlier would be undone.
    focus_after: Option<(window::Id, String)>,
    // The pictures' uploads, held while the spread is up so its first frame
    // already shows them.
    allocations: Vec<image::Allocation>,
}

#[derive(Clone, Debug)]
pub(crate) enum SpreadMessage {
    Toggle,
    Loaded(u64, Result<Loaded, String>),
    // The pictures are uploaded: map the overlay.
    Allocated(u64, Result<Vec<image::Allocation>, image::Error>),
    // The pictures have had their first frames: start.
    Begin(u64),
    Hidden(u64, Result<(), String>),
    Frame,
    Hover(usize),
    Pick(usize),
    Dismiss,
    Key(iced::keyboard::key::Named),
    Restored(u64, Result<(), String>),
    Finished(u64),
    Focused(Result<(), String>),
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
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
        let elapsed = self
            .started
            .map_or(0.0, |at| now.duration_since(at).as_secs_f32());
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
                    self.closing_from * (1.0 - ease(t))
                } else {
                    0.0
                }
            }
        }
    }

    fn animating(&self) -> bool {
        matches!(self.phase, Phase::Opening | Phase::Closing) && self.started.is_some()
    }
}

fn arrange(windows: &[SpreadWindow], area: [f32; 4], bands: (f32, f32)) -> Vec<(Rect, Rect)> {
    let from: Vec<Rect> = windows
        .iter()
        .map(|w| Rect::new(w.at[0] - area[0], w.at[1] - area[1], w.size[0], w.size[1]))
        .collect();
    // Room under each window for its title.
    let room = Rect::new(
        MARGIN,
        bands.0 + MARGIN,
        (area[2] - 2.0 * MARGIN).max(1.0),
        (area[3] - bands.0 - bands.1 - 2.0 * MARGIN).max(1.0),
    );
    let padded: Vec<Rect> = from
        .iter()
        .map(|r| Rect::new(r.x, r.y, r.width, r.height + LABEL))
        .collect();
    let to = spread_layout::layout(&padded, room, GAP);
    from.into_iter()
        .zip(to)
        .zip(padded)
        .map(|((from, to), padded)| {
            let scale = to.width / padded.width;
            (
                from,
                Rect::new(to.x, to.y, from.width * scale, from.height * scale),
            )
        })
        .collect()
}

async fn load(
    root: std::path::PathBuf,
    compositor: enzui_shell::compositor::Compositor,
    metrics: enzui_shell::settings::Metrics,
    bar: enzui_shell::settings::BarEdge,
) -> Result<Loaded, String> {
    let desktop = compositor.spread_windows().await?;
    if desktop.windows.is_empty() {
        return Err(tr("shell-no-windows-to-spread").into());
    }
    let (area, top, bottom) = compositor.prepare_spread(metrics, bar).await?;
    let (rounding, _) = compositor.window_rounding().await;
    let places = arrange(&desktop.windows, area, (top, bottom));
    // Pictures at up to twice the spread size, for a crisp scaled output.
    let pictures = iced::futures::future::join_all(desktop.windows.iter().zip(&places).map(
        |(window, (_, to))| {
            let (w, h) = (
                (to.width * 2.0).ceil() as u32,
                (to.height * 2.0).ceil() as u32,
            );
            enzui_shell::previews::capture_window(
                root.clone(),
                window.address.clone(),
                w.clamp(1, 8192),
                h.clamp(1, 8192),
            )
        },
    ))
    .await;
    Ok(Loaded {
        desktop: desktop.desktop,
        rounding,
        area,
        entries: desktop
            .windows
            .into_iter()
            .zip(places)
            .zip(pictures)
            .map(|((window, (from, to)), picture)| Entry {
                window,
                from,
                to,
                picture: picture.ok(),
            })
            .collect(),
    })
}

impl Shell {
    pub(crate) fn spread_message(&mut self, message: SpreadMessage) -> Task<Message> {
        let generation = self.spread.generation;
        match message {
            SpreadMessage::Toggle => match self.spread.phase {
                Phase::Idle => {
                    // Floating and tiled desktops both spread (windows are only
                    // made transparent while their pictures stand in); the
                    // spread loads the windows itself.
                    self.spread.generation += 1;
                    self.spread.phase = Phase::Loading;
                    let generation = self.spread.generation;
                    let (root, compositor, metrics, bar) = (
                        self.root.clone(),
                        self.compositor.clone(),
                        self.metrics,
                        self.bar,
                    );
                    Task::perform(load(root, compositor, metrics, bar), move |result| {
                        Message::Spread(SpreadMessage::Loaded(generation, result))
                    })
                }
                Phase::Loading => {
                    self.spread = Spread {
                        generation: generation + 1,
                        ..Spread::default()
                    };
                    Task::none()
                }
                _ => self.close_spread(None),
            },
            SpreadMessage::Loaded(sent, result)
                if sent == generation && self.spread.phase == Phase::Loading =>
            {
                match result {
                    Ok(loaded) => {
                        // Upload the pictures before mapping the overlay: the
                        // renderer loads images asynchronously, and the
                        // originals are hidden right after it maps, which
                        // left blank window silhouettes for a moment.
                        let uploads: Vec<_> = loaded
                            .entries
                            .iter()
                            .filter_map(|entry| entry.picture.clone())
                            .map(image::allocate)
                            .collect();
                        self.spread.desktop = loaded.desktop;
                        self.spread.rounding = loaded.rounding;
                        self.spread.area = loaded.area;
                        self.spread.entries = loaded.entries;
                        Task::batch(uploads).collect().map(move |results| {
                            Message::Spread(SpreadMessage::Allocated(
                                generation,
                                results.into_iter().collect(),
                            ))
                        })
                    }
                    Err(error) => {
                        self.spread = Spread {
                            generation,
                            ..Spread::default()
                        };
                        eprintln!("enzui-shell spread: {error}");
                        Task::none()
                    }
                }
            }
            SpreadMessage::Allocated(sent, result)
                if sent == generation && self.spread.phase == Phase::Loading =>
            {
                // A picture that cannot be uploaded still loads on its own.
                self.spread.allocations = result.unwrap_or_default();
                {
                    {
                        let area = self.spread.area;
                        let (id, open) = window::open(window_settings(
                            "dev.enzui.WindowSpread",
                            area[2],
                            area[3],
                        ));
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
                        // Nothing is chosen until the pointer or an arrow does.
                        self.spread.selected = None;
                        self.spread.phase = Phase::Opening;
                        open.map(Message::Opened)
                    }
                }
            }
            SpreadMessage::Allocated(..) => Task::none(),
            SpreadMessage::Loaded(..) => Task::none(),
            SpreadMessage::Begin(sent) if sent == generation => self.begin_spread(),
            SpreadMessage::Begin(_) => Task::none(),
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
                }
                Task::none()
            }
            SpreadMessage::Hidden(..) => Task::none(),
            SpreadMessage::Frame => {
                self.redraw_only = self.spread.window;
                let now = Instant::now();
                let done = self
                    .spread
                    .started
                    .is_some_and(|at| now.duration_since(at).as_secs_f32() >= self.spread.duration);
                match self.spread.phase {
                    Phase::Opening if done => {
                        self.spread.phase = Phase::Open;
                        Task::none()
                    }
                    Phase::Closing if done && self.spread.started.take().is_some() => {
                        // The pick is raised now, under its picture; it is
                        // focused again once the overlay has gone.
                        let chosen = self
                            .spread
                            .chosen
                            .and_then(|index| self.spread.entries.get(index))
                            .map(|entry| entry.window.address.clone());
                        let compositor = self.compositor.clone();
                        Task::perform(
                            async move { compositor.spread_restore(chosen.as_deref()).await },
                            move |result| {
                                Message::Spread(SpreadMessage::Restored(generation, result))
                            },
                        )
                    }
                    _ => Task::none(),
                }
            }
            SpreadMessage::Hover(index) if self.spread.phase == Phase::Open => {
                self.spread.selected = Some(index);
                Task::none()
            }
            SpreadMessage::Hover(_) => Task::none(),
            SpreadMessage::Pick(index) => self.close_spread(Some(index)),
            SpreadMessage::Dismiss => self.close_spread(None),
            SpreadMessage::Key(key) => self.spread_key(key),
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

    pub(crate) fn spread_opened(&mut self, id: window::Id) -> Task<Message> {
        if !self.spread.owns(id) {
            return Task::none();
        }
        // Present captured windows before hiding originals to avoid a dimmed startup frame.
        let generation = self.spread.generation;
        Task::perform(
            async { tokio::time::sleep(Duration::from_millis(90)).await },
            move |()| Message::Spread(SpreadMessage::Begin(generation)),
        )
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
        match self.spread.phase {
            Phase::Opening | Phase::Open => {
                let now = Instant::now();
                self.spread.closing_from = self.spread.progress(now);
                self.spread.phase = Phase::Closing;
                self.spread.chosen = chosen.filter(|index| *index < self.spread.entries.len());
                self.spread.started = Some(now);
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
        }
    }

    fn end_spread(&mut self) -> Task<Message> {
        let window = self.spread.window.take();
        let chosen = self
            .spread
            .chosen
            .and_then(|index| self.spread.entries.get(index))
            .map(|entry| entry.window.address.clone());
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
        let Some((_, address)) = self.spread.focus_after.take() else {
            return Task::none();
        };
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.spread_restore(Some(&address)).await },
            |result| Message::Spread(SpreadMessage::Focused(result)),
        )
    }

    pub(crate) fn spread_desktop(&self) -> Option<i64> {
        self.spread.window.is_some().then_some(self.spread.desktop)
    }

    pub(crate) fn abort_spread(&mut self) -> Task<Message> {
        if self.spread.active() {
            self.end_spread()
        } else {
            Task::none()
        }
    }

    fn spread_key(&mut self, key: iced::keyboard::key::Named) -> Task<Message> {
        use iced::keyboard::key::Named;
        if self.spread.phase != Phase::Open {
            return Task::none();
        }
        // The window that had focus is stacked last: the arrows start there,
        // and Enter with nothing chosen picks it.
        let focused = self.spread.entries.len().checked_sub(1);
        if matches!(key, Named::Enter | Named::Space) {
            return self.close_spread(self.spread.selected.or(focused));
        }
        let Some(current) = self
            .spread
            .selected
            .and_then(|index| self.spread.entries.get(index))
        else {
            self.spread.selected = focused;
            return Task::none();
        };
        let center = |r: Rect| (r.x + r.width / 2.0, r.y + r.height / 2.0);
        let (cx, cy) = center(current.to);
        let best = self
            .spread
            .entries
            .iter()
            .enumerate()
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
        if let Some((index, _)) = best {
            self.spread.selected = Some(index);
        }
        Task::none()
    }

    pub(crate) fn spread_subscription(&self) -> Subscription<Message> {
        let Some(id) = self.spread.window else {
            return Subscription::none();
        };
        let keys = iced::event::listen_with(|event, _, window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(key),
                ..
            }) if matches!(
                key,
                iced::keyboard::key::Named::ArrowLeft
                    | iced::keyboard::key::Named::ArrowRight
                    | iced::keyboard::key::Named::ArrowUp
                    | iced::keyboard::key::Named::ArrowDown
                    | iced::keyboard::key::Named::Enter
            ) =>
            {
                Some((window, key))
            }
            _ => None,
        })
        .with(id)
        .filter_map(|(owner, (window, key))| {
            (owner == window).then_some(Message::Spread(SpreadMessage::Key(key)))
        });
        // The motion follows the display: each frame of the spread asks for
        // the next, at the refresh rate, and redraws only the spread.
        let frames = if self.spread.animating() {
            window::frames().map(|_| Message::Spread(SpreadMessage::Frame))
        } else {
            Subscription::none()
        };
        Subscription::batch([keys, frames])
    }

    pub(crate) fn spread_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if !self.spread.owns(id) {
            return None;
        }
        let tokens = self.window_tokens(Some(id));
        let palette = tokens.palette;
        let p = self.spread.progress(Instant::now());
        let open = self.spread.phase == Phase::Open;
        let scrim = iced::Color {
            a: 0.32 * p,
            ..iced::Color::BLACK
        };
        let mut layers: Vec<Element<'_, Message>> = vec![
            mouse_area(
                container(iced::widget::space())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(scrim.into()),
                        ..Default::default()
                    }),
            )
            .on_press(Message::Spread(SpreadMessage::Dismiss))
            .into(),
        ];
        // Captions of similar-height windows share one line per row; a much
        // shorter window keeps its caption under itself.
        let caption_line = |index: usize| -> f32 {
            let Some(own) = self.spread.entries.get(index).map(|entry| entry.to) else {
                return 0.0;
            };
            let bottom = own.y + own.height;
            let line = self
                .spread
                .entries
                .iter()
                .map(|entry| entry.to)
                .filter(|other| other.y < own.y + own.height && own.y < other.y + other.height)
                .map(|other| other.y + other.height)
                .fold(bottom, f32::max);
            // Far below its window a caption reads as a separate control: only
            // near bottoms share a line.
            if line - bottom > 56.0 { bottom } else { line }
        };
        // Bottom-most first; a window being picked flies back on top, where
        // it will be.
        let mut order: Vec<usize> = (0..self.spread.entries.len()).collect();
        if let Some(chosen) = self
            .spread
            .chosen
            .filter(|_| self.spread.phase == Phase::Closing)
        {
            order.retain(|index| *index != chosen);
            order.push(chosen);
        }
        for (index, entry) in order
            .into_iter()
            .filter_map(|index| Some((index, self.spread.entries.get(index)?)))
        {
            let r = entry.from.lerp(entry.to, p);
            // The window's own rounded corners, at the picture's scale.
            let radius = self.spread.rounding * r.width / entry.from.width.max(1.0);
            let selected = open && self.spread.selected == Some(index);
            let picture: Element<'_, Message> = match &entry.picture {
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
                    background: Some(palette.surface.into()),
                    border: iced::Border {
                        radius: radius.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
                // No picture yet: the app's own icon and name, never its raw ID.
                None => container(
                    column![
                        match enzui_shell::search::catalogue::app_identity(&entry.window.class).1 {
                            Some(path) => enzui_shell::icon_theme::source(&path).into_element(48.0),
                            None => enz_icon("app-window").size(32.0).into_element(),
                        },
                        enz_text(
                            enzui_shell::search::catalogue::app_identity(&entry.window.class)
                                .0
                                .unwrap_or_else(|| entry.window.class.clone())
                        )
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
                    background: Some(palette.surface.into()),
                    border: iced::Border {
                        radius: radius.into(),
                        ..Default::default()
                    },
                    text_color: Some(palette.text),
                    ..Default::default()
                })
                .into(),
            };
            let accent = palette.accent;
            // The ring sits outside the picture, which would cover it.
            let framed = container(picture)
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
                });
            let label = if entry.window.title.is_empty() {
                enzui_shell::search::catalogue::app_identity(&entry.window.class)
                    .0
                    .unwrap_or_else(|| entry.window.class.clone())
            } else {
                entry.window.title.clone()
            };
            // The pointer and the arrows choose; the ring shows the choice.
            let pick = mouse_area(framed)
                .on_press(Message::Spread(SpreadMessage::Pick(index)))
                .on_enter(Message::Spread(SpreadMessage::Hover(index)))
                .interaction(iced::mouse::Interaction::Pointer);
            layers.push(pin(pick).x(r.x - RING).y(r.y - RING).into());
            // Every window is named below its picture (app icon and title),
            // so it is recognised without reading its scaled-down chrome; the
            // chosen one's caption takes the accent.
            if open {
                let icon: Element<'_, Message> =
                    match enzui_shell::search::catalogue::app_identity(&entry.window.class).1 {
                        Some(path) => enzui_shell::icon_theme::source(&path).into_element(18.0),
                        None => enz_icon("app-window").size(16.0).into_element(),
                    };
                let chip_fill = if selected {
                    palette.accent_soft
                } else {
                    palette.surface_elevated
                };
                let chip = container(
                    iced::widget::row![
                        icon,
                        enz_text(label)
                            .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                            .weight_rung(if selected {
                                enzui_theme::WeightRung::Semibold
                            } else {
                                enzui_theme::WeightRung::Medium
                            })
                            .single_line_elided()
                            .into_element(),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                )
                .padding([5, 12])
                .width(Length::Shrink)
                .style(move |_| container::Style {
                    background: Some(chip_fill.into()),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..Default::default()
                    },
                    text_color: Some(palette.text),
                    ..Default::default()
                });
                layers.push(
                    pin(container(chip).center_x(r.width.max(120.0)))
                        .x(r.x + r.width / 2.0 - r.width.max(120.0) / 2.0)
                        .y(caption_line(index) + 8.0)
                        .into(),
                );
            }
        }
        Some(
            stack(layers)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spread asked for before the shell's first compositor snapshot
    /// opens instead of being ignored.
    ///
    /// Sabotage: return early again while `snapshot` is `None`.
    #[test]
    fn a_spread_opens_before_the_first_snapshot() {
        let compositor = enzui_shell::compositor::Compositor::new(
            std::path::PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, std::path::PathBuf::from("/")).0;
        assert!(shell.snapshot.is_none());
        let _ = shell.spread_message(SpreadMessage::Toggle);
        assert!(shell.spread.phase == Phase::Loading);
    }
}
