//! Backups: where the space goes and what backups skip.
use super::*;
use enzui_shell::history_client::{self, Rules, Usage};
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::enzimos::Safety;
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::Length;
use iced::widget::{column, row};

/// Space limits offered for the backup drive.
const LIMITS: [Option<u64>; 6] = [
    None,
    Some(50 << 30),
    Some(100 << 30),
    Some(250 << 30),
    Some(500 << 30),
    Some(1 << 40),
];
/// Free space home history keeps on this computer.
const FLOORS: [u32; 5] = [10, 15, 20, 25, 30];

#[derive(Default)]
pub(super) struct BackupSpace {
    usage: Option<Result<Usage, String>>,
    rules: Option<Rules>,
    removals: Vec<(String, i64)>,
    loading: bool,
    epoch: u64,
    /// A change waiting for the service (or an approval).
    busy: bool,
    error: Option<String>,
    /// The top-level folder whose subfolders are shown.
    open_folder: Option<String>,
    show_skipped: bool,
    measuring: bool,
}

/// Everything the cards show, read together.
type Loaded = (Usage, Rules, Vec<(String, i64)>);

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum SpaceMessage {
    Read(u64, Box<Result<Loaded, String>>),
    Automatic(bool),
    Exclude(String, bool),
    Changed(Result<(), String>),
    Measure,
    Measured(Result<(), String>),
    Floor(u32),
    Limit(Option<u64>),
    OpenFolder(Option<String>),
    ShowSkipped(bool),
}

fn wrap(message: SpaceMessage) -> Parent {
    Parent::System(Message::BackupSpace(message))
}

async fn read_all() -> Result<Loaded, String> {
    let usage = history_client::usage().await?;
    let rules = history_client::rules().await?;
    let removals = history_client::removals().await.unwrap_or_default();
    Ok((usage, rules, removals))
}

/// `1.2 GB`, `340 MB`.
pub(super) fn size(bytes: u64) -> String {
    super::developer::size_text(bytes)
}

fn reason_label(key: &str) -> &'static str {
    match key {
        "build" => tr("shell-skip-reason-build"),
        "cache" => tr("shell-skip-reason-cache"),
        "games" => tr("shell-skip-reason-games"),
        "trash" => tr("shell-skip-reason-trash"),
        _ => tr("shell-skip-reason-excluded"),
    }
}

fn limit_label(limit: Option<u64>) -> String {
    match limit {
        None => tr("shell-backup-limit-none").to_owned(),
        Some(bytes) => size(bytes),
    }
}

/// A usage bar under a row, lined up with the row's text.
fn row_bar<'a>(fraction: f32) -> Element<'a, Parent> {
    iced::widget::container(enzui_widgets::progress_bar::enz_progress_bar(
        fraction.clamp(0.0, 1.0),
    ))
    .padding(iced::Padding {
        top: 0.0,
        right: 10.0,
        bottom: 6.0,
        left: 44.0,
    })
    .width(Length::Fill)
    .into()
}

impl SettingsApp {
    pub(super) fn backup_space_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.backup_space;
        if self.system.backend.demo {
            if state.usage.is_none() {
                let (usage, rules, removals) = history_client::demo();
                state.usage = Some(Ok(usage));
                state.rules = Some(rules);
                state.removals = removals;
            }
            return Task::none();
        }
        if state.loading || state.busy || !(force || state.usage.is_none()) {
            return Task::none();
        }
        state.epoch = state.epoch.wrapping_add(1);
        state.loading = true;
        let epoch = state.epoch;
        Task::perform(read_all(), move |r| {
            wrap(SpaceMessage::Read(epoch, Box::new(r)))
        })
    }

    pub(super) fn backup_space_message(&mut self, message: SpaceMessage) -> Task<Parent> {
        let demo = self.system.backend.demo;
        let state = &mut self.system.backup_space;
        let change = |task: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), String>> + Send>,
        >| { Task::perform(task, |r| wrap(SpaceMessage::Changed(r))) };
        match message {
            SpaceMessage::Read(epoch, result) => {
                if epoch == state.epoch {
                    state.loading = false;
                    match *result {
                        Ok((usage, rules, removals)) => {
                            state.usage = Some(Ok(usage));
                            state.rules = Some(rules);
                            state.removals = removals;
                        }
                        Err(e) => state.usage = Some(Err(e)),
                    }
                }
                Task::none()
            }
            SpaceMessage::Automatic(on) => {
                if state.busy {
                    return Task::none();
                }
                state.error = None;
                if let Some(rules) = state.rules.as_mut() {
                    rules.automatic = on;
                }
                if demo {
                    return Task::none();
                }
                state.busy = true;
                change(Box::pin(history_client::set_automatic(on)))
            }
            SpaceMessage::Exclude(path, excluded) => {
                if state.busy {
                    return Task::none();
                }
                state.error = None;
                if let Some(rules) = state.rules.as_mut() {
                    rules.excluded.retain(|p| *p != path);
                    if excluded {
                        rules.excluded.push(path.clone());
                    }
                }
                if demo {
                    return Task::none();
                }
                state.busy = true;
                change(Box::pin(history_client::set_excluded(path, excluded)))
            }
            SpaceMessage::Floor(percent) => {
                self.menu = None;
                if state.busy || demo {
                    if let Some(Ok(usage)) = state.usage.as_mut() {
                        usage.keep_free_percent = Some(percent);
                    }
                    return Task::none();
                }
                state.busy = true;
                state.error = None;
                change(Box::pin(history_client::set_free_percent(percent)))
            }
            SpaceMessage::Limit(limit) => {
                self.menu = None;
                let Some(mut policy) = self.safety_now().and_then(|s| s.backup_retention) else {
                    return Task::none();
                };
                policy.budget = limit;
                let state = &mut self.system.backup_space;
                if state.busy {
                    return Task::none();
                }
                state.busy = true;
                state.error = None;
                let backend = self.system.backend.clone();
                Task::perform(
                    async move { backend.set_backup_retention(policy).await },
                    |r| wrap(SpaceMessage::Changed(r)),
                )
            }
            SpaceMessage::Changed(result) => {
                state.busy = false;
                if let Err(e) = result {
                    state.error = Some(e);
                }
                Task::batch([self.backup_space_read(true), self.read_system(true)])
            }
            SpaceMessage::Measure => {
                if state.measuring || demo {
                    return Task::none();
                }
                state.measuring = true;
                Task::perform(history_client::measure(), |r| {
                    wrap(SpaceMessage::Measured(r))
                })
            }
            SpaceMessage::Measured(result) => {
                state.measuring = false;
                if let Err(e) = result {
                    state.error = Some(e);
                }
                Task::none()
            }
            SpaceMessage::OpenFolder(folder) => {
                state.open_folder = folder;
                Task::none()
            }
            SpaceMessage::ShowSkipped(show) => {
                state.show_skipped = show;
                Task::none()
            }
        }
    }

    fn bar_row<'a>(
        &self,
        label: String,
        detail: String,
        fraction: f32,
        icon: &'static str,
    ) -> Element<'a, Parent> {
        column![
            crate::settings_app::settings_row(label)
                .subtitle(detail)
                .icon(icon)
                .into_element(),
            row_bar(fraction),
        ]
        .spacing(2)
        .into()
    }

    /// A small label that groups the rows under it inside one card.
    pub(in crate::settings_app) fn group_heading<'a>(
        &self,
        text: impl Into<String>,
    ) -> Element<'a, Parent> {
        iced::widget::container(
            enz_text(text.into())
                .size(TextSize::Footnote)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .muted_on(self.tokens().palette.surface)
                .into_element(),
        )
        .padding(iced::Padding {
            top: 12.0,
            right: 10.0,
            bottom: 2.0,
            left: 10.0,
        })
        .into()
    }

    /// The Backups page's "Space" card.
    pub(super) fn backup_space_card<'a>(&'a self, safety: &Safety) -> Element<'a, Parent> {
        let route = "backups/space";
        let state = &self.system.backup_space;
        let mut rows = column![].spacing(2);
        // The backup drive: used against the limit (or the drive's size).
        if safety.local_configured {
            rows = rows.push(self.group_heading(tr("shell-backup-drive")));
            let used = safety.repository_bytes.or(safety.repository_measured);
            let limit = safety.backup_retention.and_then(|p| p.budget);
            let detail = match (used, limit, safety.destination_free) {
                (Some(u), Some(l), _) => {
                    tr!("shell-backup-space-of-limit", "arg0" => size(u), "arg1" => size(l))
                }
                (Some(u), None, Some(f)) => {
                    tr!("shell-used-by-backups-free", "arg0" => size(u), "arg1" => size(f))
                }
                (Some(u), None, None) => tr!("shell-backup-space-used", "arg0" => size(u)),
                (None, _, _) => tr("shell-backup-space-unknown").to_owned(),
            };
            let fraction = match (used, limit, safety.destination_free) {
                (Some(u), Some(l), _) => u as f32 / l as f32,
                (Some(u), None, Some(f)) => u as f32 / (u + f).max(1) as f32,
                _ => 0.0,
            };
            rows = rows.push(
                self.bar_row(
                    safety
                        .destination_label
                        .clone()
                        .unwrap_or_else(|| tr("shell-the-backup-drive").to_owned()),
                    detail,
                    fraction,
                    "hard-drive",
                ),
            );
            if let Some(policy) = safety.backup_retention
                && safety.offers("backup-retention-set")
            {
                let menu = "backups/space-limit";
                let choices: Vec<(String, Parent)> = LIMITS
                    .iter()
                    .map(|l| (limit_label(*l), wrap(SpaceMessage::Limit(*l))))
                    .collect();
                rows = rows.push(
                    crate::settings_app::settings_row(tr("shell-backup-limit"))
                        .subtitle(tr("shell-backup-limit-detail"))
                        .icon("gauge")
                        .value_menu(
                            limit_label(policy.budget),
                            self.menu.as_deref() == Some(menu),
                            Parent::Menu(menu.into(), true),
                            Parent::Menu(menu.into(), false),
                            choices,
                        )
                        .into_element(),
                );
            }
            if safety.budget_over {
                rows = rows.push(
                    enz_banner(tr("shell-backup-limit-too-small"))
                        .tone(BannerTone::Warning)
                        .into_element(),
                );
            }
        }
        let usage = match &state.usage {
            Some(Ok(usage)) => Some(usage),
            Some(Err(error)) => {
                rows = rows.push(self.caption_row(error.clone()));
                None
            }
            None => {
                rows = rows.push(self.loading_row());
                None
            }
        };
        if let Some(usage) = usage {
            rows = rows.push(self.group_heading(tr("shell-on-this-computer")));
            let history = match (usage.history_bytes, usage.oldest) {
                (Some(bytes), Some(oldest)) => {
                    tr!("shell-history-space-detail", "arg0" => size(bytes), "arg1" => usage.history_count.to_string(),
                    "arg2" => chrono::DateTime::from_timestamp(oldest, 0).map(|d| d.with_timezone(&chrono::Local).format("%-d %b %Y").to_string()).unwrap_or_default())
                }
                (Some(bytes), None) => size(bytes),
                _ => tr("shell-not-measured-yet").to_owned(),
            };
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-history-space"))
                    .subtitle(history)
                    .icon("history")
                    .into_element(),
            );
            let system = match usage.system_bytes {
                Some(bytes) => {
                    tr!("shell-system-snapshots-space", "arg0" => size(bytes), "arg1" => usage.system_count.to_string())
                }
                None => tr("shell-not-measured-yet").to_owned(),
            };
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-system-snapshots"))
                    .subtitle(system)
                    .icon("layers")
                    .into_element(),
            );
            if let Some(keep) = usage.keep_free_percent {
                let menu = "backups/keep-free";
                let choices: Vec<(String, Parent)> = FLOORS
                    .iter()
                    .map(|p| (format!("{p}%"), wrap(SpaceMessage::Floor(*p))))
                    .collect();
                let free = usage.free_percent.map_or_else(
                    String::new,
                    |f| tr!("shell-keep-free-now", "arg0" => format!("{f:.0}")),
                );
                rows = rows.push(
                    crate::settings_app::settings_row(tr("shell-keep-free"))
                        .subtitle(format!("{} {free}", tr("shell-keep-free-detail")))
                        .icon("hard-drive-download")
                        .value_menu(
                            format!("{keep}%"),
                            self.menu.as_deref() == Some(menu),
                            Parent::Menu(menu.into(), true),
                            Parent::Menu(menu.into(), false),
                            choices,
                        )
                        .into_element(),
                );
            }
            // Your home folder.
            if usage.home_total > 0 {
                rows = rows.push(self.group_heading(
                    tr!("shell-home-folder-space", "arg0" => size(usage.home_total)),
                ));
                let excluded = state
                    .rules
                    .as_ref()
                    .map(|r| r.excluded.clone())
                    .unwrap_or_default();
                let home = std::env::var("HOME").unwrap_or_default();
                for (path, bytes) in usage.top_folders(8) {
                    let open = state.open_folder.as_deref() == Some(path.as_str());
                    let absolute = format!("{home}/{path}");
                    let is_excluded = excluded.contains(&absolute);
                    let toggle = enz_button(
                        if is_excluded {
                            tr("shell-include-in-backups")
                        } else {
                            tr("shell-exclude-from-backups")
                        },
                        wrap(SpaceMessage::Exclude(absolute.clone(), !is_excluded)),
                    )
                    .id(format!("backup-space-exclude-{absolute}").into())
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .enabled(!state.busy)
                    .into_element();
                    let expand = enz_button(
                        if open {
                            tr("shell-hide")
                        } else {
                            tr("shell-subfolders")
                        },
                        wrap(SpaceMessage::OpenFolder((!open).then(|| path.clone()))),
                    )
                    .id(format!("backup-space-folder-{absolute}").into())
                    .kind(ButtonKind::Ghost)
                    .size(ButtonSize::Sm)
                    .into_element();
                    let label = if is_excluded {
                        format!("{path} · {}", tr("shell-skip-reason-excluded"))
                    } else {
                        path.clone()
                    };
                    rows = rows.push(
                        column![
                            crate::settings_app::settings_row(label)
                                .subtitle(size(bytes))
                                .icon(if is_excluded { "folder-x" } else { "folder" })
                                .accessory_width(enzui_theme::text_scale::ui_px(260.0))
                                .control(if usage.children(&path, 1).is_empty() {
                                    toggle
                                } else {
                                    Element::from(row![expand, toggle].spacing(6))
                                })
                                .into_element(),
                            row_bar(bytes as f32 / usage.home_total as f32),
                        ]
                        .spacing(2),
                    );
                    if open {
                        for (child, child_bytes) in usage.children(&path, 6) {
                            let absolute = format!("{home}/{child}");
                            let is_excluded = excluded.contains(&absolute);
                            rows = rows.push(
                                iced::widget::container(
                                    crate::settings_app::settings_row(
                                        child.rsplit('/').next().unwrap_or(&child).to_owned(),
                                    )
                                    .subtitle(size(child_bytes))
                                    .icon(if is_excluded { "folder-x" } else { "folder" })
                                    .accessory_width(enzui_theme::text_scale::ui_px(160.0))
                                    .control(
                                        enz_button(
                                            if is_excluded {
                                                tr("shell-include-in-backups")
                                            } else {
                                                tr("shell-exclude-from-backups")
                                            },
                                            wrap(SpaceMessage::Exclude(
                                                absolute.clone(),
                                                !is_excluded,
                                            )),
                                        )
                                        .id(format!("backup-space-exclude-{absolute}").into())
                                        .kind(ButtonKind::Secondary)
                                        .size(ButtonSize::Sm)
                                        .enabled(!state.busy)
                                        .into_element(),
                                    )
                                    .into_element(),
                                )
                                .padding(iced::Padding {
                                    left: 34.0,
                                    ..iced::Padding::ZERO
                                }),
                            );
                        }
                    }
                }
            }
            // What is skipped, by reason.
            let skipped_total: u64 = usage.skipped.iter().map(|(_, b)| b).sum();
            if skipped_total > 0 {
                let parts: Vec<String> = usage
                    .skipped
                    .iter()
                    .filter(|(_, b)| *b > 0)
                    .map(|(k, b)| format!("{} {}", reason_label(k), size(*b)))
                    .collect();
                rows = rows.push(
                    crate::settings_app::settings_row(
                        tr!("shell-not-backed-up", "arg0" => size(skipped_total)),
                    )
                    .subtitle(parts.join(" · "))
                    .icon("eye-off")
                    .control(
                        enz_button(
                            if state.show_skipped {
                                tr("shell-hide")
                            } else {
                                tr("shell-show")
                            },
                            wrap(SpaceMessage::ShowSkipped(!state.show_skipped)),
                        )
                        .id("backup-space-skipped".into())
                        .kind(ButtonKind::Ghost)
                        .size(ButtonSize::Sm)
                        .into_element(),
                    )
                    .into_element(),
                );
                if state.show_skipped {
                    for (path, reason, bytes) in usage.skipped_folders.iter().take(12) {
                        rows = rows.push(
                            iced::widget::container(
                                crate::settings_app::settings_row(path.clone())
                                    .subtitle(format!(
                                        "{} · {}",
                                        reason_label(reason),
                                        size(*bytes)
                                    ))
                                    .icon("folder-minus")
                                    .into_element(),
                            )
                            .padding(iced::Padding {
                                left: 34.0,
                                ..iced::Padding::ZERO
                            }),
                        );
                    }
                }
            }
            let measured = match usage.time {
                Some(t) => {
                    let ago = super::backups::when(
                        &chrono::DateTime::from_timestamp(t, 0)
                            .map(|d| d.to_rfc3339())
                            .unwrap_or_default(),
                        chrono::Local::now(),
                    );
                    if usage.complete {
                        tr!("shell-measured", "arg0" => ago)
                    } else {
                        tr!("shell-measured-partly", "arg0" => ago)
                    }
                }
                None => tr("shell-not-measured-yet").to_owned(),
            };
            rows = rows.push(
                crate::settings_app::settings_row(measured)
                    .subtitle(tr("shell-measured-detail"))
                    .icon("ruler")
                    .control(
                        enz_button(
                            if state.measuring {
                                tr("shell-measuring")
                            } else {
                                tr("shell-measure-now")
                            },
                            wrap(SpaceMessage::Measure),
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("backup-measure".into())
                        .enabled(!state.measuring)
                        .into_element(),
                    )
                    .into_element(),
            );
        }
        if let Some(error) = &state.error {
            rows = rows.push(
                enz_banner(error.clone())
                    .tone(BannerTone::Danger)
                    .into_element(),
            );
        }
        self.section_card_for(
            route,
            column![self.mark_setting(route, Element::from(rows))],
        )
        .title(tr("shell-backup-space"))
        .description(tr("shell-backup-space-description"))
        .into_element()
    }

    /// The Backups page's "What's backed up" card.
    pub(super) fn backup_rules_card<'a>(&'a self, safety: &Safety) -> Element<'a, Parent> {
        let route = "backups/rules";
        let state = &self.system.backup_space;
        let mut rows = column![].spacing(2);
        let Some(rules) = &state.rules else {
            rows = rows.push(self.loading_row());
            return self
                .section_card_for(
                    route,
                    column![self.mark_setting(route, Element::from(rows))],
                )
                .title(tr("shell-whats-backed-up"))
                .into_element();
        };
        rows = rows.push(
            crate::settings_app::settings_row(tr("shell-skip-rebuildable"))
                .subtitle(tr("shell-skip-rebuildable-detail"))
                .icon("hammer")
                .switch(rules.automatic, |on| wrap(SpaceMessage::Automatic(on)))
                .enabled(!state.busy)
                .into_element(),
        );
        let home = std::env::var("HOME").unwrap_or_default();
        for path in &rules.excluded {
            let shown = path
                .strip_prefix(&home)
                .map_or_else(|| path.clone(), |rest| format!("~{rest}"));
            rows = rows.push(
                crate::settings_app::settings_row(shown)
                    .subtitle(tr("shell-excluded-detail"))
                    .icon("folder-x")
                    .control(
                        enz_button(
                            tr("shell-include-in-backups"),
                            wrap(SpaceMessage::Exclude(path.clone(), false)),
                        )
                        .id(format!("backup-space-include-{path}").into())
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .enabled(!state.busy)
                        .into_element(),
                    )
                    .into_element(),
            );
        }
        rows = rows.push(self.info_row(
            "folder-minus",
            tr("shell-exclude-a-folder"),
            tr("shell-exclude-from-files"),
        ));
        let pending = safety.pending_removals.max(state.removals.len() as u64);
        if pending > 0 {
            let share_only = safety.share_only();
            let place = if share_only {
                tr("shell-the-network-share").to_owned()
            } else {
                safety
                    .destination_label
                    .clone()
                    .unwrap_or_else(|| tr("shell-the-backup-drive").to_owned())
            };
            rows = rows.push(
                crate::settings_app::settings_row(match (pending, share_only) {
                    (1, false) => tr("shell-removals-pending-one").to_owned(),
                    (1, true) => tr("shell-removals-pending-share-one").to_owned(),
                    (_, false) => tr!("shell-removals-pending", "arg0" => pending.to_string()),
                    (_, true) => tr!("shell-removals-pending-share", "arg0" => pending.to_string()),
                })
                .subtitle(tr!("shell-removals-pending-detail", "arg0" => place))
                .icon("trash-2")
                .into_element(),
            );
            for (path, _) in state.removals.iter().take(5) {
                let shown = path
                    .strip_prefix(&home)
                    .map_or_else(|| path.clone(), |rest| format!("~{rest}"));
                rows = rows.push(iced::widget::container(self.caption(shown)).padding(
                    iced::Padding {
                        top: 2.0,
                        right: 10.0,
                        bottom: 2.0,
                        left: 44.0,
                    },
                ));
            }
        }
        self.section_card_for(
            route,
            column![self.mark_setting(route, Element::from(rows))],
        )
        .title(tr("shell-whats-backed-up"))
        .description(tr("shell-whats-backed-up-description"))
        .into_element()
    }
}
