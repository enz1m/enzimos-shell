use super::{Message, Shell};
use enzui_shell::activity::{self, Event, Snapshot, Windows};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{icon::enz_icon, progress_bar::enz_progress_bar, text::enz_text};
use iced::advanced::{Layout, Widget, layout, renderer, widget::Tree};
use iced::widget::{container, row, stack};
use iced::{Element, Length, Rectangle, Size, Subscription, Task, mouse};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

// Background work in the bar: the latest snapshot, the compositor's
// windows, and each app's icon.
#[derive(Clone, Debug, Default)]
pub(crate) struct Activity {
    snapshot: Option<Snapshot>,
    windows: Windows,
    icons: HashMap<String, Option<PathBuf>>,
    // The (pid, token) pairs the windows were read for.
    read_for: Vec<(u32, Option<String>)>,
    reading: bool,
    generation: u64,
    windows_current: bool,
}

impl Activity {
    fn invalidate_windows(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.windows_current = false;
    }

    fn visible<'a>(&self, snapshot: &'a Snapshot) -> Vec<&'a activity::Job> {
        activity::visible(snapshot, &self.windows)
            .into_iter()
            .filter(|job| {
                self.windows_current || job.work.visibility == activity::Visibility::Always
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ActivityMessage {
    Service(Event),
    Focus,
    Windows(u64, Result<Windows, String>),
    Tick,
}

impl Shell {
    fn activity_enabled(&self) -> bool {
        self.desired_settings.activity_status
    }

    pub(crate) fn activity_subscription(&self) -> Subscription<Message> {
        let Some(socket) = activity::socket().filter(|_| self.activity_enabled()) else {
            return Subscription::none();
        };
        let counting = self.activity_panel_window().is_some()
            && self
                .activity
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| activity::counting(&self.activity.visible(snapshot)));
        let needs_focus = self.activity.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .jobs
                .iter()
                .any(|job| job.work.visibility == activity::Visibility::WhenUnfocused)
        });
        Subscription::batch([
            activity::subscription(socket)
                .map(|event| Message::Activity(ActivityMessage::Service(event))),
            if needs_focus {
                self.compositor
                    .focus_changes()
                    .map(|()| Message::Activity(ActivityMessage::Focus))
            } else {
                Subscription::none()
            },
            if counting {
                iced::time::every(std::time::Duration::from_secs(1))
                    .map(|_| Message::Activity(ActivityMessage::Tick))
            } else {
                Subscription::none()
            },
        ])
    }

    pub(crate) fn activity_message(&mut self, message: ActivityMessage) -> Task<Message> {
        match message {
            ActivityMessage::Service(Event::Snapshot(snapshot)) => {
                if !self.activity_enabled() {
                    return Task::none();
                }
                self.activity
                    .icons
                    .retain(|id, _| snapshot.jobs.iter().any(|job| &job.app_id == id));
                for job in &snapshot.jobs {
                    self.activity
                        .icons
                        .entry(job.app_id.clone())
                        .or_insert_with(|| {
                            enzui_shell::search::catalogue::icon_for_app_id(&job.app_id)
                        });
                }
                let owners: Vec<(u32, Option<String>)> = snapshot
                    .jobs
                    .iter()
                    .filter(|job| job.work.visibility == activity::Visibility::WhenUnfocused)
                    .map(|job| (job.peer_pid, job.work.window_token.clone()))
                    .collect();
                self.activity.snapshot = Some(*snapshot);
                // New publishers or tokens: learn which windows they own.
                if !owners.is_empty()
                    && (!self.activity.windows_current
                        || owners
                            .iter()
                            .any(|owner| !self.activity.read_for.contains(owner)))
                {
                    self.activity.read_for = owners;
                    self.activity.invalidate_windows();
                    return self.read_activity_windows();
                }
                self.sync_activity_panel()
            }
            ActivityMessage::Service(Event::Cleared) => {
                self.quiet = self.activity.snapshot.is_none();
                self.activity.snapshot = None;
                self.activity.read_for.clear();
                self.activity.invalidate_windows();
                self.sync_activity_panel()
            }
            ActivityMessage::Focus => {
                // Only starts a read; the windows it finds may change the bar.
                self.quiet = false;
                self.activity.invalidate_windows();
                self.read_activity_windows()
            }
            ActivityMessage::Windows(generation, result) => {
                self.activity.reading = false;
                if generation != self.activity.generation {
                    // A focus change during the query needs a fresh read, not a stale reply.
                    return self.read_activity_windows();
                }
                self.quiet = false;
                self.activity.windows_current = result.is_ok();
                if let Ok(windows) = result {
                    self.activity.windows = windows;
                }
                self.sync_activity_panel()
            }
            ActivityMessage::Tick => {
                self.redraw_only = self.activity_panel_window();
                self.quiet = self.redraw_only.is_none();
                Task::none()
            }
        }
    }

    fn read_activity_windows(&mut self) -> Task<Message> {
        if !self.activity_enabled() || std::mem::replace(&mut self.activity.reading, true) {
            return Task::none();
        }
        let compositor = self.compositor.clone();
        let generation = self.activity.generation;
        Task::perform(
            async move { compositor.activity_windows().await },
            move |result| Message::Activity(ActivityMessage::Windows(generation, result)),
        )
    }

    // Forgets everything when the presentation is switched off.
    pub(crate) fn activity_settings_changed(&mut self) {
        if !self.activity_enabled() {
            self.activity.snapshot = None;
            self.activity.read_for.clear();
            self.activity.icons.clear();
            self.activity.invalidate_windows();
        }
    }

    fn visible_activity(&self) -> Vec<&activity::Job> {
        self.activity
            .snapshot
            .as_ref()
            .filter(|_| self.activity_enabled())
            .map(|snapshot| self.activity.visible(snapshot))
            .unwrap_or_default()
    }

    pub(crate) fn has_visible_activity(&self) -> bool {
        self.activity.snapshot.as_ref().is_some_and(|snapshot| {
            self.activity_enabled()
                && snapshot.jobs.iter().any(|job| {
                    job.work.visibility == activity::Visibility::Always
                        || (self.activity.windows_current && !self.activity.windows.is_origin(job))
                })
        })
    }

    pub(crate) fn activity_panel_size(&self) -> iced::Size {
        let rows = self.visible_activity().len().clamp(1, 4);
        let text = f32::from(self.metrics.text);
        iced::Size::new(350.0, 56.0 + text * 2.0 + rows as f32 * (28.0 + text * 2.7))
    }

    fn activity_icon(&self, job: &activity::Job, size: f32) -> Element<'_, Message> {
        match self
            .activity
            .icons
            .get(&job.app_id)
            .and_then(Option::as_ref)
        {
            Some(path) if path.extension().is_some_and(|ext| ext == "svg") => {
                iced::widget::svg(iced::widget::svg::Handle::from_path(path))
                    .width(size)
                    .height(size)
                    .into()
            }
            Some(path) => iced::widget::image(path.clone())
                .width(size)
                .height(size)
                .into(),
            None => enz_icon("activity").size(size).into_element(),
        }
    }

    pub(crate) fn activity_view(&self) -> Element<'_, Message> {
        use enzui_widgets::scroll_view::enz_scroll_view;
        use iced::{Length, widget::column};

        let text = f32::from(self.metrics.text);
        let surface = self.tokens().palette.surface;
        let header = enz_text(tr("shell-live-activity"))
            .size_px(enzui_shell::panel_style::title_px(text))
            .weight_rung(enzui_theme::WeightRung::Semibold)
            .into_element();
        let jobs = self.visible_activity();
        let mut rows = column![].spacing(16).width(Length::Fill);
        let received = self
            .activity
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.received);
        let now = Instant::now();
        if jobs.is_empty() {
            rows = rows.push(
                enz_text(tr("shell-no-live-activity"))
                    .muted_on(surface)
                    .into_element(),
            );
        }
        for job in jobs {
            let fraction = activity::fraction(job).filter(|_| job.outcome.is_none());
            let state = received.and_then(|received| activity::status(job, received, now));
            let status = match (fraction, state) {
                (Some(fraction), Some(state)) => format!("{} · {state}", percent(fraction)),
                (Some(fraction), None) => percent(fraction),
                (None, state) => state.unwrap_or_else(|| tr("shell-in-progress").into()),
            };
            let mut copy = column![
                row![
                    enz_text(&job.work.label)
                        .size_px(text)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .single_line_elided()
                        .width(Length::Fill)
                        .into_element(),
                    enz_text(status)
                        .size_px(text)
                        .muted_on(surface)
                        .into_element(),
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center)
            ]
            .spacing(6)
            .width(Length::Fill);
            if !job.work.detail.is_empty() {
                copy = copy.push(
                    enz_text(&job.work.detail)
                        .size_px(text)
                        .single_line_elided()
                        .muted_on(surface)
                        .into_element(),
                );
            }
            if let Some(fraction) = fraction {
                copy = copy.push(enz_progress_bar(fraction).bar_height(4.0).into_element());
            }
            rows = rows.push(
                row![self.activity_icon(job, text + 10.0), copy]
                    .spacing(12)
                    .align_y(iced::Alignment::Center)
                    .width(Length::Fill),
            );
        }
        column![
            header,
            enz_scroll_view(rows).height(Length::Fill).into_element()
        ]
        .spacing(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    pub(crate) fn activity_item(&self) -> Option<Element<'_, Message>> {
        use crate::notification_panel::Message as Panel;
        use iced::widget::mouse_area;

        let jobs = self.visible_activity();
        let job = *jobs.first()?;
        let snapshot = self.activity.snapshot.as_ref()?;
        let size = f32::from(self.metrics.text);
        let fraction = activity::fraction(job).filter(|_| job.outcome.is_none());
        let icon = match job.outcome {
            Some(activity::Outcome::Succeeded) => Some("check"),
            Some(activity::Outcome::Failed) => Some("circle-alert"),
            Some(activity::Outcome::Cancelled) => Some("x"),
            None if job.work.state == activity::State::Paused => Some("pause"),
            None => None,
        };
        let icon_size = f32::from(self.metrics.icon);
        let icon = icon.map_or_else(
            || self.activity_icon(job, icon_size),
            |icon| enz_icon(icon).size(icon_size).into_element(),
        );
        let progress = fraction.map(percent);
        let mut content = row![icon].spacing(8).align_y(iced::Alignment::Center);
        if let Some(value) = &progress {
            content = content
                .push(container(enz_text(value).size_px(size).into_element()).center_x(size * 3.1));
        } else if job.outcome.is_none() && job.work.state == activity::State::Running {
            // Work without a measure still reads as busy; a static glyph keeps
            // the bar from redrawing for the length of a long job.
            content = content.push(enz_icon("loader").size(icon_size * 0.85).into_element());
        }
        if jobs.len() > 1 {
            content = content.push(
                enz_text(format!("+{}", jobs.len() - 1))
                    .size_px(size)
                    .into_element(),
            );
        }
        let status = activity::status(job, snapshot.received, Instant::now())
            .map(|state| format!(", {state}"))
            .unwrap_or_default();
        let measure = progress
            .map(|value| format!(", {value}"))
            .unwrap_or_default();
        let others = if jobs.len() > 1 {
            tr!("shell-and-more", "arg0" => (jobs.len() - 1).to_string())
        } else {
            String::new()
        };
        let trigger = activity_trigger(
            content.into(),
            self.metrics,
            tr!("shell-open-live-activity", "arg0" => job.work.label.clone(), "arg1" => measure, "arg2" => status, "arg3" => others),
        );
        Some(
            mouse_area(trigger)
                .on_enter(Message::Notification(Panel::ActivityTriggerHover(true)))
                .on_exit(Message::Notification(Panel::ActivityTriggerHover(false)))
                .into(),
        )
    }
}

fn activity_trigger<'a>(
    content: Element<'a, Message>,
    metrics: enzui_shell::settings::Metrics,
    label: String,
) -> Element<'a, Message> {
    let content = container(content)
        .padding([0.0, f32::from(metrics.bar_padding(10))])
        .width(Length::Shrink.min(28.0))
        .center_y(Length::Fill);
    let content = stack![content, Element::new(ActivityMousePress)];
    enzui_shell::bar_item::trigger(
        enzui_widgets::clickable::enz_clickable(
            content,
            Message::Notification(crate::notification_panel::Message::ToggleActivity),
        ),
        metrics,
    )
    .padding(0)
    .aria_label(label)
    .id("bar-activity".into())
    .into_element()
}

struct ActivityMousePress;

impl Widget<Message, iced::Theme, iced::Renderer> for ActivityMousePress {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, _: &iced::Renderer, limits: &layout::Limits) {
        tree.size = limits.max();
    }

    fn draw(
        &self,
        _: &Tree,
        _: &mut iced::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        _: Layout,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
    }

    fn update(
        &mut self,
        _: &mut Tree,
        event: &iced::Event,
        layout: Layout,
        cursor: mouse::Cursor,
        _: &iced::Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && !shell.is_event_captured()
            && cursor.is_over(layout.bounds())
            && cursor.is_over(*viewport)
        {
            // Mapping the hover preview can change focus between press and release.
            shell.publish(Message::Notification(
                crate::notification_panel::Message::ToggleActivity,
            ));
            shell.capture_event();
        }
    }
}

fn percent(fraction: f32) -> String {
    tr!("shell-activity-percent", "arg0" => enzui_shell::i18n::number(format!("{:.0}", fraction * 100.0)))
}

impl Shell {
    // Notify only new controller connections, suppressing startup inventory and rapid
    // reconnects.
    pub(crate) fn controllers_changed(
        &mut self,
        generation: u64,
        list: Vec<enzui_shell::controllers::Controller>,
    ) -> Task<Message> {
        if generation != self.controllers_generation
            || !self.desired_settings.controller_notifications
        {
            return Task::none();
        }
        let mut tasks = Vec::new();
        for controller in self.controllers.connected(
            &list,
            self.desired_settings.controller_notifications,
            std::time::Instant::now(),
        ) {
            let mut body = format!("{} · {}", controller.name, controller.transport.label());
            if let Some(battery) = controller.battery {
                let _ = write!(body, " · {}", battery.label());
            }
            tasks.push(Task::perform(
                enzui_shell::battery_warnings::notify(
                    tr("shell-controller-connected").into(),
                    body,
                    false,
                ),
                Message::ControllerNotified,
            ));
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_changes_during_a_window_query_cannot_publish_stale_foreground_work() {
        let compositor = enzui_shell::compositor::Compositor::new(
            PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
        shell.desired_settings.activity_status = true;
        let snapshot = activity::parse(br#"{"op":"snapshot","version":1,"generation":"focus","revision":1,"jobs":[{"id":"1:copy","app_id":"org.example.Files","peer_pid":10,"work":{"job":"copy","label":"Copying files","state":"running"},"outcome":null,"age_ms":0}]}"#).unwrap();
        let _ = shell.activity_message(ActivityMessage::Service(Event::Snapshot(Box::new(
            snapshot.clone(),
        ))));
        let first = shell.activity.generation;
        assert!(
            shell.activity_item().is_none(),
            "wait for origin ownership before showing work"
        );
        let _ = shell.activity_message(ActivityMessage::Focus);
        let current = shell.activity.generation;
        let background = Windows {
            active: Some((20, None)),
            tagged: Vec::new(),
        };
        let _ = shell.activity_message(ActivityMessage::Windows(first, Ok(background.clone())));
        assert!(
            shell.activity_item().is_none(),
            "an obsolete reply cannot reveal the copy"
        );
        let _ = shell.activity_message(ActivityMessage::Windows(current, Ok(background)));
        assert!(shell.activity_item().is_some(), "a different app is active");
        let _ = shell.activity_message(ActivityMessage::Focus);
        let current = shell.activity.generation;
        assert!(
            shell.activity_item().is_none(),
            "hide while focus ownership is unknown"
        );
        let _ = shell.activity_message(ActivityMessage::Windows(
            current,
            Ok(Windows {
                active: Some((10, None)),
                tagged: Vec::new(),
            }),
        ));
        assert!(
            shell.activity_item().is_none(),
            "the originating app is active"
        );
        let mut always = snapshot;
        always.jobs[0].work.visibility = activity::Visibility::Always;
        let _ = shell.activity_message(ActivityMessage::Service(Event::Snapshot(Box::new(always))));
        assert!(
            shell.activity_item().is_some(),
            "producer explicitly requested foreground visibility"
        );
        shell.desired_settings.activity_status = false;
        shell.activity_settings_changed();
        let _ = shell.activity_message(ActivityMessage::Windows(current, Ok(Windows::default())));
        assert!(
            shell.activity_item().is_none(),
            "a late reply cannot restore a disabled activity"
        );
    }
}
