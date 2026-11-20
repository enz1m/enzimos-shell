use super::{Message, Shell};
use enzui_shell::activity::{self, Event, Snapshot, Windows};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{icon::enz_icon, progress_bar::enz_progress_bar, text::enz_text};
use iced::widget::{container, row};
use iced::{Element, Subscription, Task};
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
        let counting = self
            .activity
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| activity::counting(&self.activity.visible(snapshot)));
        Subscription::batch([
            activity::subscription(socket)
                .map(|event| Message::Activity(ActivityMessage::Service(event))),
            self.compositor
                .focus_changes()
                .map(|()| Message::Activity(ActivityMessage::Focus)),
            // Only while an estimate is on screen.
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
                    .map(|job| (job.peer_pid, job.work.window_token.clone()))
                    .collect();
                self.activity.snapshot = Some(*snapshot);
                // New publishers or tokens: learn which windows they own.
                if !self.activity.windows_current
                    || owners
                        .iter()
                        .any(|owner| !self.activity.read_for.contains(owner))
                {
                    self.activity.read_for = owners;
                    self.activity.invalidate_windows();
                    return self.read_activity_windows();
                }
                Task::none()
            }
            ActivityMessage::Service(Event::Cleared) => {
                self.quiet = self.activity.snapshot.is_none();
                self.activity.snapshot = None;
                self.activity.read_for.clear();
                self.activity.invalidate_windows();
                Task::none()
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
                Task::none()
            }
            ActivityMessage::Tick => Task::none(),
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

    pub(crate) fn activity_view(&self) -> Element<'_, Message> {
        use enzui_widgets::scroll_view::enz_scroll_view;
        use iced::{Length, widget::column};
        let header = enz_text(tr("shell-live-activity"))
            .size_px(enzui_shell::panel_style::title_px(f32::from(
                self.metrics.text,
            )))
            .weight_rung(enzui_theme::WeightRung::Semibold)
            .into_element();
        let mut rows = column![].spacing(20).width(Length::Fill);
        let jobs = self
            .activity
            .snapshot
            .as_ref()
            .map(|snapshot| self.activity.visible(snapshot))
            .unwrap_or_default();
        if jobs.is_empty() {
            rows = rows.push(enz_text(tr("shell-no-live-activity")).into_element());
        }
        for job in jobs {
            let mut content = column![
                enz_text(&job.work.label)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element()
            ]
            .spacing(6)
            .width(Length::Fill);
            if !job.work.detail.is_empty() {
                content = content.push(enz_text(&job.work.detail).into_element());
            }
            if job.outcome.is_none()
                && let Some(fraction) = activity::fraction(job)
            {
                content = content.push(enz_progress_bar(fraction).bar_height(4.0).into_element());
            }
            let status = self
                .activity
                .snapshot
                .as_ref()
                .and_then(|snapshot| activity::status(job, snapshot.received, Instant::now()));
            let measurement = activity::fraction(job).filter(|_| job.outcome.is_none())
                .map(|fraction| tr!("shell-activity-percent", "arg0" => enzui_shell::i18n::number(format!("{:.0}", fraction * 100.0))));
            let detail = [measurement, status]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            content = content.push(
                enz_text(if detail.is_empty() {
                    tr("shell-in-progress").to_owned()
                } else {
                    detail
                })
                .size_px(13.0)
                .muted_on(self.tokens().palette.surface)
                .into_element(),
            );
            rows = rows.push(content);
        }
        column![
            header,
            enz_scroll_view(rows).height(Length::Fill).into_element()
        ]
        .spacing(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    // The bar's center while other windows' work runs, in place of the time
    // and notifications; `None` shows those as usual.
    pub(crate) fn activity_item(&self) -> Option<Element<'_, Message>> {
        if !self.activity_enabled() {
            return None;
        }
        let snapshot = self.activity.snapshot.as_ref()?;
        let jobs = self.activity.visible(snapshot);
        let job = *jobs.first()?;
        let size = f32::from(self.metrics.text);
        let icon: Element<'_, Message> =
            match self.activity.icons.get(&job.app_id).cloned().flatten() {
                Some(path) if path.extension().is_some_and(|e| e == "svg") => {
                    iced::widget::svg(iced::widget::svg::Handle::from_path(path))
                        .width(f32::from(self.metrics.icon))
                        .height(f32::from(self.metrics.icon))
                        .into()
                }
                Some(path) => iced::widget::image(path)
                    .width(f32::from(self.metrics.icon))
                    .height(f32::from(self.metrics.icon))
                    .into(),
                None => enz_icon("activity")
                    .size(f32::from(self.metrics.icon))
                    .into_element(),
            };
        let status = activity::status(job, snapshot.received, Instant::now());
        let fraction = activity::fraction(job);
        let mut content = row![
            icon,
            enz_text(&job.work.label)
                .size_px(size)
                .single_line_elided()
                .into_element()
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        // The trigger names the work; the bar only paints the measured part.
        // Work of unknown size says so instead of drawing an empty bar.
        let status = match (job.outcome, fraction) {
            (None, Some(fraction)) => {
                content = content.push(
                    container(enz_progress_bar(fraction).bar_height(4.0).into_element())
                        .width(80.0)
                        .center_y(f32::from(self.metrics.bar)),
                );
                status
            }
            (None, None) => status.or_else(|| Some(tr("shell-in-progress").into())),
            _ => status,
        };
        if let Some(status) = &status {
            content = content.push(
                enz_text(status)
                    .size_px(size)
                    .muted_on(self.tokens().palette.surface)
                    .into_element(),
            );
        }
        if jobs.len() > 1 {
            content = content.push(
                enz_text(format!("+{}", jobs.len() - 1))
                    .size_px(size)
                    .into_element(),
            );
        }
        let percent = fraction
            .map(|f| tr!("shell-percent", "arg0" => enzui_shell::i18n::number(format!("{:.0}", f * 100.0))))
            .unwrap_or_default();
        let others = if jobs.len() > 1 {
            tr!("shell-and-more", "arg0" => format!("{:}", jobs.len() - 1))
        } else {
            String::new()
        };
        let state = status.map(|s| format!(", {s}")).unwrap_or_default();
        // The popup contains only live work; completion never enters notification history.
        Some(
            enzui_shell::bar_item::trigger(
                enzui_widgets::clickable::enz_clickable(
                    container(content).padding([0, self.metrics.islands.map_or(0, |_| self.metrics.bar_padding(10))]),
                    Message::Notification(crate::notification_panel::Message::ToggleActivity),
                ),
                self.metrics,
            )
            .aria_label(tr!("shell-open-live-activity", "arg0" => job.work.label.clone(), "arg1" => percent, "arg2" => state, "arg3" => others))
            .id("bar-activity".into())
            .into_element(),
        )
    }
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
