use super::{Message, Shell};
use enzui_core::calendar_nav::MonthController;
use enzui_core::civil_date::{Date, DateBounds, Weekday};
use enzui_shell::calendar_events::{Client, Event, Failure};
use enzui_shell::i18n::{self, tr};
use enzui_shell::world_clocks::WorldTime;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    calendar::{CalendarEvent, enz_calendar},
    text::{TextSize, enz_text},
};
use iced::widget::{column, row};
use iced::{Element, Length, Task};
use std::sync::Arc;

// The clock popover's calendar, the chosen day's events from the Calendar
// app and the world clocks, at the top of the notification center.
#[derive(Debug)]
pub(crate) struct ClockPanel {
    controller: MonthController,
    selected: Date,
    events: Option<Result<Vec<Event>, Failure>>,
    clocks: Vec<WorldTime>,
    // One MCP Center session for the shell's lifetime (see calendar_events).
    client: Arc<tokio::sync::Mutex<Option<Client>>>,
    fetching: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum ClockMessage {
    Calendar(CalendarEvent),
    Events(Date, Result<Vec<Event>, Failure>),
    Clocks(Vec<WorldTime>),
    OpenCalendar,
}

fn today() -> Date {
    use chrono::Datelike;
    let now = chrono::Local::now().date_naive();
    Date::new(now.year(), now.month(), now.day())
        .unwrap_or(Date::new(2000, 1, 1).expect("a valid date"))
}

impl Default for ClockPanel {
    fn default() -> Self {
        let today = today();
        Self {
            controller: MonthController::new(
                today,
                i18n::current()
                    .locale()
                    .first_weekday()
                    .unwrap_or(Weekday::Monday),
                DateBounds::unbounded(),
            ),
            selected: today,
            events: None,
            clocks: Vec::new(),
            client: Arc::default(),
            fetching: false,
        }
    }
}

/// Calendar is installed as a Flatpak; the clock opens it by its desktop entry.
const CALENDAR: &str = "io.github.enzui.Calendar";

impl Shell {
    // Back to today, with fresh events and world times; on every opening.
    pub(crate) fn clock_refresh(&mut self) -> Task<Message> {
        let today = today();
        self.clock_panel.selected = today;
        self.clock_panel.controller = MonthController::new(
            today,
            i18n::current()
                .locale()
                .first_weekday()
                .unwrap_or(Weekday::Monday),
            DateBounds::unbounded(),
        );
        self.clock_panel.events = None;
        Task::batch([self.clock_fetch(), self.clock_times()])
    }

    // While the popover is open: keep the world times current and retry
    // while the Calendar permission is being decided.
    pub(crate) fn clock_tick(&mut self) -> Task<Message> {
        let waiting = matches!(
            self.clock_panel.events,
            Some(Err(Failure::Permission | Failure::Closed))
        );
        Task::batch([
            self.clock_times(),
            if waiting {
                self.clock_fetch()
            } else {
                Task::none()
            },
        ])
    }

    fn clock_times(&self) -> Task<Message> {
        let zones = self.desired_settings.world_clocks.clone();
        if zones.is_empty() {
            return Task::done(Message::Clock(ClockMessage::Clocks(Vec::new())));
        }
        let h24 = self.desired_settings.session.clock_24h;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || enzui_shell::world_clocks::now(&zones, h24))
                    .await
                    .unwrap_or_default()
            },
            |clocks| Message::Clock(ClockMessage::Clocks(clocks)),
        )
    }

    fn clock_fetch(&mut self) -> Task<Message> {
        if std::mem::replace(&mut self.clock_panel.fetching, true) {
            return Task::none();
        }
        let (client, date) = (
            std::sync::Arc::clone(&self.clock_panel.client),
            self.clock_panel.selected,
        );
        Task::perform(
            async move {
                let mut guard = client.lock().await;
                if !guard.as_mut().is_some_and(Client::alive) {
                    *guard = Some(Client::connect().await?);
                }
                let result = guard
                    .as_mut()
                    .expect("connected")
                    .events(&date.to_string())
                    .await;
                if matches!(result, Err(Failure::Unavailable)) {
                    *guard = None;
                }
                result
            },
            move |result| Message::Clock(ClockMessage::Events(date, result)),
        )
    }

    pub(crate) fn clock_message(&mut self, message: ClockMessage) -> Task<Message> {
        match message {
            ClockMessage::Calendar(event) => {
                let controller = &mut self.clock_panel.controller;
                match event {
                    CalendarEvent::DaySelected(date) => {
                        let _ = controller.focus_date(date);
                        self.clock_panel.selected = date;
                        self.clock_panel.events = None;
                        return self.clock_fetch();
                    }
                    CalendarEvent::Moved(movement) => {
                        let _ = controller.apply(movement);
                    }
                    CalendarEvent::Focused(date) => {
                        let _ = controller.focus_date(date);
                    }
                    CalendarEvent::Paged(months) => {
                        controller.page_months(months);
                    }
                    CalendarEvent::YearChanged(month) | CalendarEvent::Jumped(month) => {
                        controller.jump_to_month(month);
                    }
                    CalendarEvent::HeaderPressed => {}
                }
                Task::none()
            }
            ClockMessage::Events(date, result) => {
                self.clock_panel.fetching = false;
                if date == self.clock_panel.selected {
                    let before = self.clock_panel_height();
                    self.clock_panel.events = Some(result);
                    self.refit_calendar_panel(before)
                } else {
                    self.clock_fetch()
                }
            }
            ClockMessage::Clocks(clocks) => {
                let before = self.clock_panel_height();
                self.clock_panel.clocks = clocks;
                self.refit_calendar_panel(before)
            }
            ClockMessage::OpenCalendar => self.open_desktop_app(CALENDAR),
        }
    }

    /// Opens the installed app whose desktop entry is `id`, as Search does.
    fn open_desktop_app(&mut self, id: &str) -> Task<Message> {
        use enzui_shell::search::{AppTarget, Target, catalogue};
        let Some(path) = catalogue::installed_entry(id) else {
            self.notice = Some(tr("shell-application-is-no-longer-installed").into());
            return Task::none();
        };
        let identity = match catalogue::FileIdentity::capture(&path) {
            Ok(identity) => identity,
            Err(error) => {
                self.notice = Some(error);
                return Task::none();
            }
        };
        if self.launching {
            return Task::none();
        }
        self.launching = true;
        let compositor = self.compositor.clone();
        let root = self.root.clone();
        let serial = self.panel_action_serial;
        let owner = self.launcher;
        self.launch_inflight = Some((serial, owner));
        let target = Target::App(AppTarget::Desktop { path, identity });
        Task::perform(
            async move { compositor.execute_search_target(&root, target).await },
            move |result| Message::Launched(serial, owner, result),
        )
    }

    /// The calendar popup's body height for what it shows: the month, the
    /// day's events (or a note, with an "Open Calendar" button when events
    /// could not be read) and any world clocks. A fixed 440 left a large gap
    /// under a one-line note.
    pub(crate) fn clock_panel_height(&self) -> f32 {
        // Room for a note and its "Open Calendar" button is kept from the
        // start, so the usual answers (none, one or two events, a note) never
        // resize the open popup: a resized popup is re-centred by the
        // compositor and slid up over the clock.
        let events = match &self.clock_panel.events {
            Some(Ok(list)) => 22.0 * list.len().clamp(1, 6).saturating_sub(1) as f32,
            _ => 0.0,
        }
        .max(34.0);
        let clocks = if self.clock_panel.clocks.is_empty() {
            0.0
        } else {
            28.0 + 24.0 * self.clock_panel.clocks.len() as f32
        };
        // The month grid keeps its size; the heading, note and button under
        // it (about 96px of the base height) and the rows grow with the text
        // size, or at the larger sizes the note ran into its button.
        let scale = (f32::from(self.metrics.text) / 13.0).max(1.0);
        (328.0 + 96.0 * (scale - 1.0) + (events + clocks) * scale).min(640.0)
    }

    pub(crate) fn clock_view(&self) -> Element<'_, Message> {
        let palette = self.tokens().palette;
        let calendar = enz_calendar(&self.clock_panel.controller, |event| {
            Message::Clock(ClockMessage::Calendar(event))
        })
        .selected(Some(self.clock_panel.selected))
        .today(Some(today()))
        .flat(true)
        .fill_width(true)
        .round_days(true)
        .cell_size(30.0)
        // Days of the neighbouring months recede: they only fill the grid.
        .tokens_resolved(|theme| {
            let palette = enzui_widgets::theme_lookup::palette(theme);
            let mut tokens = enzui_widgets::calendar::CalendarTokens::from_palette(&palette);
            tokens.adjacent = iced::Color {
                a: tokens.adjacent.a * 0.7,
                ..tokens.adjacent
            };
            tokens
        })
        .id("clock-calendar".into())
        .into_element();
        let heading = if self.clock_panel.selected == today() {
            tr("shell-today").to_owned()
        } else {
            chrono::NaiveDate::from_ymd_opt(
                self.clock_panel.selected.year(),
                self.clock_panel.selected.month(),
                self.clock_panel.selected.day(),
            )
            .map_or_else(
                || self.clock_panel.selected.to_string(),
                |d| i18n::clock_date(d, false),
            )
        };
        let mut events = column![
            enz_text(heading)
                .size(TextSize::Footnote)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element()
        ]
        .spacing(6);
        let note = |text: &'static str| {
            enz_text(text)
                .size(TextSize::Footnote)
                .muted_on(palette.surface)
                .into_element()
        };
        events = match &self.clock_panel.events {
            None => events.push(note(tr("shell-loading-events"))),
            Some(Ok(list)) if list.is_empty() => events.push(note(tr("shell-no-events"))),
            Some(Ok(list)) => list.iter().take(6).fold(events, |events, event| {
                let when = if event.all_day {
                    tr("shell-all-day").to_owned()
                } else {
                    let time = |m: u16| {
                        chrono::NaiveTime::from_hms_opt(
                            u32::from(m / 60 % 24),
                            u32::from(m % 60),
                            0,
                        )
                        .map(|t| i18n::time(&t, self.desired_settings.session.clock_24h))
                        .unwrap_or_default()
                    };
                    format!("{} – {}", time(event.start_minute), time(event.end_minute))
                };
                events.push(
                    row![
                        enz_text(when)
                            .size(TextSize::Footnote)
                            .muted_on(palette.surface)
                            .into_element(),
                        enz_text(event.title.clone())
                            .single_line_elided()
                            .into_element(),
                    ]
                    .spacing(10),
                )
            }),
            Some(Err(Failure::Permission)) => events.push(note(tr(
                "shell-allow-desktop-clock-to-list-calendar-events-in-the-mcp-center-prompt",
            ))),
            // Refused: not retried; Settings › Agents › Permissions can undo it.
            Some(Err(Failure::Denied)) => {
                events.push(note(tr("shell-calendar-access-is-off-for-the-clock")))
            }
            // Whatever kept the events away, Calendar itself still opens.
            // The note may wrap; the action keeps its full name on its own line.
            Some(Err(failure)) => events.push(
                column![
                    note(tr(match failure {
                        Failure::Closed => "shell-calendar-is-not-open",
                        Failure::Unavailable => "shell-events-need-the-enzui-mcp-center",
                        _ => "shell-events-could-not-be-read",
                    })),
                    enz_button(
                        tr("shell-open-calendar"),
                        Message::Clock(ClockMessage::OpenCalendar)
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id("clock-open-calendar".into())
                    .into_element(),
                ]
                .spacing(6),
            ),
        };
        let mut content = column![calendar, events].spacing(12).width(Length::Fill);
        if !self.clock_panel.clocks.is_empty() {
            let mut clocks = column![
                enz_text(tr("shell-world-clocks"))
                    .size(TextSize::Footnote)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element()
            ]
            .spacing(4);
            for clock in &self.clock_panel.clocks {
                clocks = clocks.push(
                    row![
                        enz_text(clock.city.clone()).into_element(),
                        iced::widget::space().width(Length::Fill),
                        enz_text(clock.day.map_or_else(
                            || clock.time.clone(),
                            |day| format!("{day}, {}", clock.time)
                        ))
                        .muted_on(palette.surface)
                        .into_element(),
                    ]
                    .align_y(iced::Alignment::Center),
                );
            }
            content = content.push(clocks);
        }
        content.into()
    }
}
