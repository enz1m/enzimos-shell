use super::*;
use iced::widget::column;

impl SettingsApp {
    fn backups_overview<'a>(&self, safety: &Safety) -> Element<'a, Parent> {
        let now = chrono::Local::now();
        let job = self.system.jobs.get(&LongJob::BackupNow);
        let (icon, title, subtitle, badge) = if !safety.destination() {
            (
                "shield-alert",
                tr("shell-backups-are-not-set-up").to_owned(),
                if safety.home_history {
                    tr("shell-home-history-keeps-earlier-versions-on-this-computer-but-a-backup-on-another").to_owned()
                } else {
                    tr("shell-nothing-is-kept-anywhere-else-turn-on-home-history-and-add-a-backup-drive").to_owned()
                },
                Some((tr("shell-not-protected"), BadgeTone::Warning)),
            )
        } else if safety.waiting_for_drive() && !matches!(job, Some(JobState::Running)) {
            let drive = safety
                .destination_label
                .clone()
                .unwrap_or_else(|| tr("shell-the-backup-drive").into());
            let since = safety
                .last_success
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|d| when(&d.to_rfc3339(), now));
            (
                "plug",
                tr!("shell-connect-to-back-up", "arg0" => drive),
                match since {
                    Some(since) => tr!("shell-waiting-for-drive-since", "arg0" => since),
                    None => tr("shell-waiting-for-drive").to_owned(),
                },
                Some((tr("shell-waiting-badge"), BadgeTone::Warning)),
            )
        } else if safety.run_state == "failed" && !matches!(job, Some(JobState::Running)) {
            (
                "shield-x",
                tr("shell-the-last-backup-did-not-run").to_owned(),
                safety.run_code.clone().unwrap_or_default(),
                Some((tr("shell-failed"), BadgeTone::Error)),
            )
        } else if matches!(job, Some(JobState::Running)) {
            (
                "refresh-cw",
                tr("shell-backing-up-now").to_owned(),
                tr("shell-you-can-keep-working-while-it-runs").to_owned(),
                Some((tr("shell-working"), BadgeTone::Accent)),
            )
        } else if let Some(last) = &safety.last_backup {
            let where_ = safety
                .destination_label
                .clone()
                .unwrap_or_else(|| tr("shell-the-backup-drive").into());
            let next = match (&safety.next_run, safety.schedule_enabled) {
                (Some(next), true) => {
                    tr!("shell-next-backup-2", "arg0" => when(next, now).to_lowercase())
                }
                (_, false) => tr("shell-automatic-backups-are-off").into(),
                _ => String::new(),
            };
            match last.state.as_str() {
                "completed" => (
                    "shield-check",
                    tr!("shell-last-backup", "arg0" => when(&last.time, now).to_lowercase()),
                    tr!("shell-kept-on", "arg0" => where_, "arg1" => next),
                    Some((tr("shell-protected"), BadgeTone::Success)),
                ),
                "uncertain" => (
                    "shield-question",
                    tr("shell-the-last-backup-s-result-is-not-known").to_owned(),
                    tr("shell-it-may-or-may-not-have-finished-back-up-again-to-be-sure").to_owned(),
                    Some((tr("shell-check"), BadgeTone::Warning)),
                ),
                _ => (
                    "shield-x",
                    tr!("shell-the-last-backup-failed", "arg0" => when(&last.time, now).to_lowercase()),
                    tr!("shell-back-up-again-or-check", "arg0" => where_, "arg1" => next),
                    Some((tr("shell-failed"), BadgeTone::Error)),
                ),
            }
        } else {
            let schedule = if safety.schedule_enabled
                && safety.recovery_available
                && safety.recovery_copy_state != "user-attested"
            {
                tr(
                    "shell-automatic-backups-are-blocked-until-you-confirm-a-current-separate-recovery",
                )
            } else if safety.schedule_enabled {
                tr("shell-backups-run-automatically-every-day")
            } else {
                tr("shell-automatic-backups-are-off-back-up-by-hand-below")
            };
            (
                "shield",
                tr("shell-backups-are-set-up").to_owned(),
                schedule.to_owned(),
                None,
            )
        };
        let mut status = crate::settings_app::settings_row(title)
            .subtitle(subtitle)
            .icon(icon);
        if let Some((label, tone)) = badge {
            status = status.control(self.status_badge(label, tone));
        }
        let mut rows =
            column![self.mark_setting("backups/overview", status.into_element())].spacing(2);
        // The backend does not measure the encrypted repository; free space
        // on the verified drive is shown alone then.
        match (
            safety.repository_bytes.or(safety.repository_measured),
            safety.destination_free,
        ) {
            (Some(used), Some(free)) => {
                rows = rows.push(crate::settings_app::settings_row(tr("shell-space")).subtitle(tr!("shell-used-by-backups-free", "arg0" => super::developer::size_text(used), "arg1" => super::developer::size_text(free))).icon("hard-drive").into_element());
            }
            (None, Some(free)) => {
                rows = rows.push(
                    crate::settings_app::settings_row(tr("shell-free-on-the-backup-drive"))
                        .subtitle(super::developer::size_text(free))
                        .icon("hard-drive")
                        .into_element(),
                );
            }
            _ => {}
        }
        rows.into()
    }

    fn snapshot_row<'a>(
        &self,
        snapshot: &Snapshot,
        now: chrono::DateTime<chrono::Local>,
    ) -> Element<'a, Parent> {
        // enzimOS's own descriptions read as plain words; an update hook's
        // description already says before or after what.
        let description = match snapshot.description.as_str() {
            d if d.starts_with("enzimOS required update fallback") => {
                tr("shell-before-an-update").to_owned()
            }
            "enzimOS home history" | "enzimOS backup source" => String::new(),
            d => d.to_owned(),
        };
        let subtitle = match (snapshot.kind, description.is_empty()) {
            (_, true) => kind_label(snapshot.kind).to_owned(),
            (SnapshotKind::Before | SnapshotKind::After, false) => description,
            (kind, false) => format!("{} · {}", kind_label(kind), description),
        };
        crate::settings_app::settings_row(when(&snapshot.date, now))
            .subtitle(subtitle)
            .icon(if snapshot.kind == SnapshotKind::Automatic {
                "history"
            } else {
                "bookmark"
            })
            .into_element()
    }

    fn snapshot_list<'a>(&self, safety: &Safety) -> Element<'a, Parent> {
        let state = &self.system.backups;
        let now = chrono::Local::now();
        let Some((system, home)) = &safety.lists else {
            let mut rows = column![].spacing(2);
            rows = rows.push(
                self.property_row(
                    tr("shell-home-snapshots"),
                    safety
                        .home_snapshots
                        .map_or_else(|| "—".to_owned(), |n| n.to_string()),
                ),
            );
            if let Some(latest) = &safety.latest_home {
                rows = rows
                    .push(self.property_row(tr("shell-latest-home-snapshot"), when(latest, now)));
            }
            rows = rows.push(
                self.property_row(
                    tr("shell-system-snapshots"),
                    safety
                        .root_snapshots
                        .map_or_else(|| "—".to_owned(), |n| n.to_string()),
                ),
            );
            return rows.into();
        };
        let tabs = enz_view_switcher(
            vec![
                SwitcherPage::new("home", enzui_shell::tr!("shell-home-folder-2", "arg0" => enzui_shell::i18n::argument(&(home.len())))),
                SwitcherPage::new("system", enzui_shell::tr!("shell-system-2", "arg0" => enzui_shell::i18n::argument(&(system.len())))),
            ],
            usize::from(state.system_tab),
            |index| send(BackupsMessage::SystemTab(index == 1)),
        )
        .stretch(true)
        .aria_label(tr("shell-snapshots-of"))
        .tab_id_prefix("backup-snapshots")
        .into_element();
        let list = if state.system_tab { system } else { home };
        let mut rows = column![iced::widget::container(tabs).padding(iced::Padding {
            top: 10.0,
            right: 10.0,
            bottom: 4.0,
            left: 10.0,
        })]
        .spacing(2);
        if list.is_empty() {
            rows = rows.push(self.caption_row(if state.system_tab {
                tr("shell-no-system-snapshots-yet-one-is-taken-before-and-after-each-update")
            } else if safety.home_history {
                tr("shell-the-first-snapshot-is-taken-within-the-hour")
            } else {
                tr("shell-turn-on-home-history-to-start-keeping-snapshots")
            }));
        }
        let shown = if state.all_snapshots {
            list.len()
        } else {
            list.len().min(SHORT_SNAPSHOTS)
        };
        for snapshot in &list[..shown] {
            rows = rows.push(self.snapshot_row(snapshot, now));
        }
        if list.len() > SHORT_SNAPSHOTS {
            let label = if state.all_snapshots {
                tr("shell-show-fewer").to_owned()
            } else {
                tr!("shell-show-all-2", "arg0" => format!("{:}", list.len()))
            };
            rows = rows.push(
                iced::widget::container(
                    enz_button(
                        label,
                        send(BackupsMessage::AllSnapshots(!state.all_snapshots)),
                    )
                    .kind(ButtonKind::Ghost)
                    .size(ButtonSize::Sm)
                    .into_element(),
                )
                .padding(iced::Padding {
                    left: 34.0,
                    ..iced::Padding::ZERO
                }),
            );
        }
        if state.system_tab {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-go-back-to-an-earlier-system"))
                    .subtitle(tr("shell-each-update-is-kept-as-its-own-version-start-an-earlier-one-from-updates-you"))
                    .icon("rotate-ccw")
                    .control(enz_button(tr("shell-versions"), Parent::Page(enzui_shell::settings_catalog::Category::Updates)).kind(ButtonKind::Secondary).size(ButtonSize::Sm).into_element())
                    .into_element(),
            );
        }
        rows.into()
    }

    fn point_row<'a>(
        &self,
        point: &Point,
        safety: &Safety,
        now: chrono::DateTime<chrono::Local>,
    ) -> Element<'a, Parent> {
        let state = &self.system.backups;
        let writes = self.system_writes();
        let busy = state.working.is_some();
        let mut subtitle = place_label(point.place).to_owned();
        if let Some(size) = point.size {
            subtitle = format!("{subtitle} · {}", super::developer::size_text(size));
        }
        if !point.complete {
            subtitle = tr!("shell-incomplete", "arg0" => subtitle.clone());
        }
        let id = point.id.get(..12).unwrap_or(&point.id);
        let mut actions = row![].spacing(6).align_y(iced::Alignment::Center);
        if state.working.as_deref() == Some(point.id.as_str()) {
            actions = actions.push(self.status_badge(tr("shell-working"), BadgeTone::Accent));
        } else {
            if safety.remote_configured && point.place == Place::Local {
                actions = actions.push(
                    enz_toolbar_icon(
                        "cloud-upload",
                        send(BackupsMessage::Move(point.id.clone(), true)),
                    )
                    .aria_label(tr("shell-copy-to-the-network-share"))
                    .tooltip(tr("shell-copy-to-the-network-share"))
                    .id(format!("backup-copy-{id}").into())
                    .enabled(writes && !busy)
                    .into_element(),
                );
            }
            if point.place == Place::Remote {
                actions = actions.push(
                    enz_button(
                        tr("shell-bring-back"),
                        send(BackupsMessage::Move(point.id.clone(), false)),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(format!("backup-retrieve-{id}").into())
                    .enabled(writes && !busy)
                    .into_element(),
                );
            } else {
                actions = actions.push(
                    enz_button(
                        tr("shell-restore"),
                        send(BackupsMessage::OpenRestore(point.id.clone())),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(format!("backup-restore-{id}").into())
                    .enabled(writes && !busy && point.complete)
                    .into_element(),
                );
            }
        }
        crate::settings_app::settings_row(when(&point.time, now))
            .subtitle(subtitle)
            .icon("archive")
            .control(actions)
            .into_element()
    }

    fn restore_form<'a>(&'a self, form: &'a RestoreForm, safety: &Safety) -> Element<'a, Parent> {
        let now = chrono::Local::now();
        let working = self.system.backups.working.is_some();
        let from = safety
            .points
            .iter()
            .flatten()
            .find(|p| p.id == form.point)
            .map_or_else(
                || tr("shell-the-chosen-backup").to_owned(),
                |p| when(&p.time, now).to_lowercase(),
            );
        let kind = enz_view_switcher(
            vec![
                SwitcherPage::new("file", tr("shell-a-file")),
                SwitcherPage::new("folder", tr("shell-a-folder")),
            ],
            usize::from(form.folder),
            |index| send(BackupsMessage::Folder(index == 1)),
        )
        .aria_label(tr("shell-restore-2"))
        .tab_id_prefix("backup-restore-kind")
        .into_element();
        let mut field = enz_text_field(
            enzui_shell::i18n::current().text("shell-its-place-in-your-home-folder"),
            if form.folder {
                "Pictures/Holiday 2026"
            } else {
                "Documents/report.odt"
            },
            &form.path,
            |text| send(BackupsMessage::Path(text)),
        )
        .on_submit(send(BackupsMessage::Restore))
        .id("backup-restore-path".into());
        if let Some(error) = &form.error {
            field = field.error(error.as_str());
        }
        let field = field.into_element();
        let single = !form.folder && safety.offers("own-file-restore");
        let label = match (working, &form.review) {
            (true, Some(_) | None) if !single => tr("shell-restoring"),
            (true, None) => tr("shell-checking"),
            (true, Some(_)) => tr("shell-restoring"),
            (false, None) if single => tr("shell-continue"),
            (false, _) => tr("shell-restore-2"),
        };
        let buttons = row![
            iced::widget::space::horizontal(),
            enz_button(tr("shell-cancel"), send(BackupsMessage::CloseRestore))
                .kind(ButtonKind::Ghost)
                .size(ButtonSize::Sm)
                .enabled(!working)
                .into_element(),
            enz_button(label, send(BackupsMessage::Restore))
                .kind(ButtonKind::Primary)
                .size(ButtonSize::Sm)
                .id("backup-restore-go".into())
                .enabled(!working && !form.path.trim().is_empty() && self.system_writes())
                .into_element(),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        let caption = match (&form.review, form.folder, single) {
            (Some(plan), _, _) => tr!("shell-it-arrives-as-in-your-home-folder-nothing-is-overwritten-an-administra", "arg0" => plan.filename.clone()),
            (None, true, _) => tr("shell-the-folder-is-saved-as-one-archive-file-in-your-home-folder-at-most-4096-ite").into(),
            (None, false, true) => tr("shell-the-file-arrives-as-a-new-file-in-your-home-folder-so-nothing-is-overwritten").into(),
            (None, false, false) => tr("shell-the-file-arrives-as-a-new-file-in-your-home-folder-nothing-is-overwritten-an").into(),
        };
        self.mark_setting(
            "backups/points",
            iced::widget::container(
                column![
                    enz_text(enzui_shell::tr!("shell-restore-from-the-backup-made", "arg0" => enzui_shell::i18n::argument(&(from))))
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                    kind,
                    field,
                    self.caption(caption),
                    buttons,
                ]
                .spacing(10),
            )
            .padding([10, 12]),
        )
    }

    fn retention_row<'a>(&self, retention: Retention, safety: &Safety) -> Element<'a, Parent> {
        let state = &self.system.backups;
        let parts: Vec<String> = [
            (retention.hourly, "hourly"),
            (retention.daily, "daily"),
            (retention.weekly, "weekly"),
            (retention.monthly, "monthly"),
            (retention.yearly, "yearly"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, kind)| format!("{n} {kind}"))
        .chain(std::iter::once(
            tr!("shell-taken-by-hand-2", "arg0" => format!("{:}", retention.manual)),
        ))
        .collect();
        let mut row = crate::settings_app::settings_row(tr("shell-snapshots-kept"))
            .subtitle(parts.join(", "))
            .subtitle_wrap(true)
            .icon("layers");
        if safety.offers("home-retention-set") && state.retention.is_none() {
            row = row.control(
                enz_button(
                    tr("shell-change-2"),
                    send(BackupsMessage::EditRetention(Some(true))),
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id("backup-retention-change".into())
                .enabled(self.system_writes())
                .into_element(),
            );
        }
        let mut column = column![row.into_element()].spacing(2);
        if let Some(form) = state.retention.as_ref().filter(|form| form.home.is_some()) {
            column = column.push(self.retention_form(form));
        }
        column.into()
    }

    fn cleanup_row<'a>(
        &self,
        title: &'static str,
        cleanup: &Cleanup,
        now: chrono::DateTime<chrono::Local>,
    ) -> Element<'a, Parent> {
        let outcome = match cleanup.state {
            CleanupState::Removed => {
                tr!("shell-removed-old-backups", "arg0" => format!("{:}", cleanup.removed.unwrap_or(0)))
            }
            CleanupState::Unchanged => tr("shell-no-old-backups-needed-removal").to_owned(),
            CleanupState::Off => tr("shell-automatic-cleanup-was-off").to_owned(),
            CleanupState::Failed => {
                tr("shell-cleanup-could-not-finish-check-before-the-backup-drive-fills-up")
                    .to_owned()
            }
        };
        let at = chrono::DateTime::<chrono::Utc>::from_timestamp(cleanup.at_unix, 0)
            .map(|at| when(&at.to_rfc3339(), now));
        let mut row = crate::settings_app::settings_row(title)
            .subtitle(at.map_or_else(|| outcome.clone(), |at| format!("{outcome} · {at}")))
            .icon(if cleanup.state == CleanupState::Failed {
                "shield-alert"
            } else {
                "archive"
            });
        if cleanup.state == CleanupState::Failed && cleanup.reason.is_some() {
            row = row.control(
                enz_button(
                    if self.system.backups.cleanup_details {
                        tr("shell-hide-details")
                    } else {
                        tr("shell-details")
                    },
                    send(BackupsMessage::CleanupDetails(
                        !self.system.backups.cleanup_details,
                    )),
                )
                .kind(ButtonKind::Quiet)
                .size(ButtonSize::Sm)
                .into_element(),
            );
        }
        let mut rows = column![row.into_element()];
        if self.system.backups.cleanup_details
            && let Some(reason) = &cleanup.reason
        {
            rows = rows.push(self.caption(reason.clone()));
        }
        rows.into()
    }

    fn retention_form<'a>(&self, form: &RetentionForm) -> Element<'a, Parent> {
        let asking = self.system.backups.asking;
        let stepper =
            |index: usize, title: &'static str, value: u32, min: u32| -> Element<'a, Parent> {
                crate::settings_app::settings_row(title)
                    .control(
                        enzui_widgets::stepper::enz_stepper(
                            i32::try_from(value).unwrap_or(0),
                            move |v| send(BackupsMessage::Count(index, v.max(0) as u32)),
                        )
                        .min(i32::try_from(min).unwrap_or(0))
                        .max(Retention::MAX as i32)
                        .id(format!("backup-keep-{index}"))
                        .enabled(!asking)
                        .into_element(),
                    )
                    .into_element()
            };
        let mut rows = column![].spacing(2);
        if let Some(home) = form.home {
            rows = rows
                .push(self.caption(tr("shell-home-snapshots-kept-by-future-cleanup")))
                .push(stepper(0, tr("shell-hourly"), home.hourly, 0))
                .push(stepper(1, tr("shell-daily"), home.daily, 0))
                .push(stepper(2, tr("shell-weekly"), home.weekly, 0))
                .push(stepper(3, tr("shell-monthly"), home.monthly, 0))
                .push(stepper(4, tr("shell-yearly"), home.yearly, 0))
                .push(stepper(5, tr("shell-taken-by-hand"), home.manual, 1));
        }
        if let Some(policy) = form.backup {
            rows = rows.push(crate::settings_app::settings_row(tr("shell-remove-old-backups-automatically"))
                .subtitle(tr("shell-cleanup-follows-successful-local-backups-and-network-copies-turning-it-off-c"))
                .switch(policy.automatic, |v| send(BackupsMessage::CleanupAutomatic(v))).enabled(!asking).into_element());
            let picker = enzui_widgets::combo_box::enz_combo_box(
                "",
                BackupPreset::ALL
                    .iter()
                    .map(|p| p.label().to_owned())
                    .collect(),
                Some(form.preset.label().to_owned()),
                |label| {
                    send(BackupsMessage::CleanupPreset(
                        BackupPreset::ALL
                            .into_iter()
                            .find(|p| p.label() == label)
                            .unwrap_or(BackupPreset::Custom),
                    ))
                },
            )
            .aria_label(enzui_shell::i18n::tr("shell-backup-retention-preset"))
            .id("backup-retention-preset".into())
            .width(210)
            .enabled(!asking)
            .into_element();
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-keep-backups"))
                    .control(picker)
                    .into_element(),
            );
            if form.preset == BackupPreset::Custom {
                rows = rows
                    .push(stepper(6, tr("shell-newest-backups"), policy.last, 1))
                    .push(stepper(7, tr("shell-daily-copies"), policy.daily, 0))
                    .push(stepper(8, tr("shell-weekly-copies"), policy.weekly, 0))
                    .push(stepper(9, tr("shell-monthly-copies"), policy.monthly, 0))
                    .push(stepper(10, tr("shell-yearly-copies"), policy.yearly, 0));
            }
            rows = rows.push(self.caption(tr("shell-keep-the-newest-backup-in-each-of-the-selected-day-week-month-and-year-bucke")));
        }
        let valid = form.home.is_none_or(Retention::valid)
            && form.backup.is_none_or(BackupRetention::valid);
        rows = rows.push(self.caption(if valid {
            tr("shell-fewer-applies-at-the-next-cleanup-and-can-remove-older-ones-then-nothing-is")
        } else {
            tr("shell-keep-at-least-one-snapshot-taken-by-hand-and-between-1-and-1000-timed-ones-i")
        }));
        rows = rows.push(
            row![
                iced::widget::space::horizontal(),
                enz_button(
                    tr("shell-cancel"),
                    send(BackupsMessage::EditRetention(None))
                )
                .kind(ButtonKind::Ghost)
                .size(ButtonSize::Sm)
                .enabled(!asking)
                .into_element(),
                enz_button(
                    if asking {
                        enzui_shell::i18n::tr("shell-waiting")
                    } else {
                        enzui_shell::i18n::tr("shell-apply")
                    },
                    send(BackupsMessage::ApplyRetention)
                )
                .kind(ButtonKind::Primary)
                .size(ButtonSize::Sm)
                .id("backup-retention-apply".into())
                .enabled(!asking && valid && self.system_writes())
                .into_element(),
            ]
            .spacing(8),
        );
        iced::widget::container(rows)
            .padding(iced::Padding {
                left: 28.0,
                top: 4.0,
                bottom: 8.0,
                right: 0.0,
            })
            .into()
    }

    fn outcome_banner<'a>(&self, in_history: bool) -> Option<Element<'a, Parent>> {
        let state = &self.system.backups;
        let outcome = state
            .outcome
            .as_ref()
            .filter(|_| state.outcome_in_history == in_history)?;
        let banner = match outcome {
            Outcome::Restored(name, true) => enz_banner(tr!("shell-the-folder-was-saved-as-one-archive-file", "arg0" => name.clone()))
                .tone(BannerTone::Success)
                .body(tr("shell-it-is-a-tar-archive-in-your-home-folder-open-it-in-files-to-take-out-what-yo"))
                .action_text(tr("shell-show-in-files"))
                .on_action(send(BackupsMessage::ShowHome)),
            Outcome::Restored(name, false) => enz_banner(tr!("shell-restored-as-in-your-home-folder", "arg0" => name.clone()))
                .tone(BannerTone::Success)
                .body(if name.ends_with(".bin") {
                    tr("shell-rename-it-to-use-it-the-file-you-have-now-was-not-replaced")
                } else {
                    tr("shell-it-keeps-its-type-so-it-opens-like-the-original-the-file-you-have-now-was-no")
                })
                .action_text(tr("shell-show-in-files"))
                .on_action(send(BackupsMessage::ShowHome)),
            Outcome::Copied(true) => enz_banner(tr("shell-the-backup-was-copied-to-the-network-share")).tone(BannerTone::Success),
            Outcome::Copied(false) => enz_banner(tr("shell-the-backup-was-brought-back-to-this-computer")).tone(BannerTone::Success),
            Outcome::Snapshot(number) => enz_banner(tr!("shell-snapshot-of-your-home-folder-was-taken", "arg0" => format!("{:}", number))).tone(BannerTone::Success),
            Outcome::Kept => enz_banner(tr("shell-what-is-kept-was-changed"))
                .tone(BannerTone::Success)
                .body(tr("shell-it-applies-from-the-next-cleanup-nothing-kept-now-was-removed")),
            Outcome::Failed(error) => enz_banner(tr("shell-that-did-not-finish")).tone(BannerTone::Danger).body(error.clone()),
        };
        Some(
            banner
                .on_dismiss(send(BackupsMessage::Dismiss))
                .into_element(),
        )
    }

    pub(in crate::settings_app) fn safety_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let reading = match self.reading(Panel::Safety) {
            Some(Reading::Safety(r)) => Some(r),
            _ => None,
        };
        let Some(Backed::Ready(safety)) = &reading else {
            let routes = [
                "backups/overview",
                "backups/history",
                "backups/snapshots",
                "backups/destination",
                "backups/schedule",
                "backups/now",
                "backups/points",
                "backups/rules",
                "backups/space",
            ];
            return body.push(
                self.section_card_for(
                    "backups/overview",
                    self.enzimos_placeholder(reading.as_ref(), &routes),
                )
                .title(tr("shell-history-backups"))
                .into_element(),
            );
        };
        let now = chrono::Local::now();
        let mut body = body.push(
            self.section_card_for("backups/overview", self.backups_overview(safety))
                .title(tr("shell-overview"))
                .into_element(),
        );

        let mut history =
            column![
                self.switch_setting("backups/history", safety.home_history, |on| Self::request(
                    Request::HomeHistory(on)
                ))
            ]
            .spacing(2);
        if safety.home_history
            && !safety.home_history_state.is_empty()
            && !matches!(safety.home_history_state.as_str(), "active" | "enabled")
        {
            history = history.push(self.caption_row(
                tr!("shell-home-history-is", "arg0" => safety.home_history_state.clone()),
            ));
        }
        if safety.home_history && safety.offers("home-history-create") {
            let asking = self.system.backups.asking;
            history = history.push(
                crate::settings_app::settings_row(tr("shell-take-a-snapshot-now"))
                    .subtitle(tr("shell-before-a-big-change-keep-the-home-folder-as-it-is-an-administrator-approves"))
                    .icon("camera")
                    .control(
                        enz_button(if asking { tr("shell-waiting") } else { tr("shell-take-snapshot") }, send(BackupsMessage::SnapshotNow))
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id("backup-snapshot-now".into())
                            .enabled(self.system_writes() && !asking)
                            .into_element(),
                    )
                    .into_element(),
            );
        }
        if let Some(retention) = safety.home_retention {
            history = history.push(self.retention_row(retention, safety));
        }
        if let Some(banner) = self.outcome_banner(true) {
            history = history.push(iced::widget::container(banner).padding([6, 4]));
        }
        history = history.push(self.mark_setting("backups/snapshots", self.snapshot_list(safety)));
        body = body.push(
            self.section_card_for("backups/history", history)
                .title(tr("shell-snapshots"))
                .description(tr("shell-quick-snapshots-of-your-home-folder-and-of-the-system-kept-on-this-disk-they"))
                .into_element(),
        );

        // Where backups go: the drive, and a spot for the network share.
        let mut backups = column![].spacing(2);
        if safety.local_configured || safety.remote_configured {
            backups = backups.push(self.mark_setting(
                "backups/destination",
                self.info_row(
                    "hard-drive",
                    tr("shell-backup-drive"),
                    match (&safety.destination_label, safety.local_configured) {
                        (Some(label), true) => label.clone(),
                        (None, true) => tr("shell-a-backup-drive").to_owned(),
                        (_, false) => tr("shell-not-set-up").to_owned(),
                    },
                ),
            ));
            backups = backups.push(self.backup_share_rows(safety));
        } else {
            backups = backups.push(self.mark_setting(
                "backups/destination",
                self.info_row("hard-drive", tr("shell-destination"), tr("shell-none-yet")),
            ));
            backups = backups.push(self.backup_share_rows(safety));
        }
        if (safety.setup_available || self.system.backups.setup.job_present())
            && self.system.backups.setup.shown(safety.local_configured)
        {
            backups = backups.push(self.backup_setup_view(safety));
        } else if !safety.destination() {
            backups = backups.push(self.caption_row(tr("shell-the-system-service-does-not-offer-backup-drive-enrollment-in-settings-existi")));
        }
        let recovery = &self.system.backups;
        if safety.recovery_available {
            let attested = safety.recovery_copy_state == "user-attested";
            let detail = if recovery.recovery_busy {
                tr("shell-complete-the-recovery-password-dialog-to-continue")
            } else {
                match safety.recovery_copy_state.as_str() {
                    "user-attested" => tr("shell-recovery-saved-detail"),
                    "needs-review" => tr("shell-confirm-your-separate-recovery-copy"),
                    "stale" => tr("shell-save-a-new-recovery-copy"),
                    _ => tr(
                        "shell-you-need-this-password-if-your-computer-fails-save-it-on-a-separate-drive-or",
                    ),
                }
            };
            let delivery_enabled =
                self.system_writes() && safety.local_configured && !recovery.recovery_busy;
            let deliver = |label: &'static str, delivery, id: &'static str| {
                enz_button(label, send(BackupsMessage::RecoveryDeliver(delivery)))
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(id.into())
                    .enabled(delivery_enabled)
                    .into_element()
            };
            use enzui_shell::system_settings::backup_recovery::Delivery;
            backups = backups.push(
                crate::settings_app::settings_row(tr("shell-recovery-password"))
                    .subtitle(detail)
                    .subtitle_wrap(true)
                    .icon(if attested {
                        "shield-check"
                    } else {
                        "key-round"
                    })
                    .accessory_width(enzui_theme::text_scale::ui_px(240.0))
                    .control(
                        row![
                            deliver(
                                tr("shell-save-to-another-drive"),
                                Delivery::Save,
                                "backup-recovery-save"
                            ),
                            deliver(
                                tr("shell-print-2"),
                                Delivery::Print,
                                "backup-recovery-print"
                            ),
                        ]
                        .spacing(6),
                    )
                    .into_element(),
            );
            if let Some(notice) = &recovery.recovery_notice {
                backups = backups.push(
                    iced::widget::container(self.caption(notice.clone())).padding(iced::Padding {
                        top: 2.0,
                        right: 10.0,
                        bottom: 6.0,
                        left: 44.0,
                    }),
                );
            }
            if recovery.recovery_delivery.is_some() {
                backups = backups.push(crate::settings_app::settings_row(tr("shell-i-have-the-complete-password-outside-this-computer"))
                .subtitle(tr("shell-only-confirm-after-checking-your-saved-file-or-printed-paper-this-asks-for-s"))
                .subtitle_wrap(true)
                .icon("shield-check")
                .control(enz_button(tr("shell-confirm-separate-copy"), send(BackupsMessage::RecoveryConfirm)).kind(ButtonKind::Secondary).size(ButtonSize::Sm).id("backup-recovery-confirm".into()).enabled(self.system_writes() && !recovery.recovery_busy).into_element()).into_element());
            }
        }
        let can_schedule = safety.schedule_enabled
            || safety.destination()
                && (safety.recovery_copy_state == "user-attested" || !safety.recovery_available);
        let schedule_detail = if safety.schedule_enabled
            && safety.recovery_available
            && safety.recovery_copy_state != "user-attested"
        {
            tr("shell-backups-waiting-for-recovery-copy")
        } else if safety.schedule_enabled {
            if safety.next_run.is_some() {
                tr("shell-backups-run-automatically-every-day")
            } else {
                tr("shell-backups-start-after-restart")
            }
        } else if can_schedule {
            tr("shell-backups-off-detail")
        } else if !safety.destination() {
            tr("shell-set-up-a-backup-destination-first")
        } else {
            tr("shell-save-or-print-the-recovery-password-and-confirm-your-separate-copy-first")
        };
        backups = backups.push(
            self.mark_setting(
                "backups/schedule",
                crate::settings_app::settings_row(tr("shell-automatic-backups"))
                    .subtitle(schedule_detail)
                    .subtitle_wrap(true)
                    .icon("calendar-sync")
                    .id_prefix("backup-schedule")
                    .switch(safety.schedule_enabled, |on| {
                        Self::request(Request::BackupSchedule(on))
                    })
                    .enabled(self.system_writes() && can_schedule && !recovery.recovery_busy)
                    .into_element(),
            ),
        );
        if let (true, Some(next)) = (
            safety.schedule_enabled
                && (!safety.recovery_available || safety.recovery_copy_state == "user-attested"),
            &safety.next_run,
        ) {
            backups = backups.push(
                crate::settings_app::settings_row(tr("shell-next-backup"))
                    .subtitle(when(next, now))
                    .icon("clock")
                    .into_element(),
            );
        }
        if let Some(policy) = safety.backup_retention {
            let summary = tr!("shell-newest-daily-weekly-monthly-yearly", "arg0" => policy.preset().label().to_owned(), "arg1" => format!("{:}", policy.last), "arg2" => format!("{:}", policy.daily), "arg3" => format!("{:}", policy.weekly), "arg4" => format!("{:}", policy.monthly), "arg5" => format!("{:}", policy.yearly), "arg6" => (if policy.automatic { tr("shell-automatic-cleanup-on") } else { tr("shell-automatic-cleanup-off") }).to_owned());
            let mut row = crate::settings_app::settings_row(tr("shell-keep-backups"))
                .subtitle(summary)
                .icon("archive");
            if safety.offers("backup-retention-set") && self.system.backups.retention.is_none() {
                row = row.control(
                    enz_button(
                        tr("shell-change-2"),
                        send(BackupsMessage::EditRetention(Some(false))),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id("backup-keep-change".into())
                    .enabled(self.system_writes())
                    .into_element(),
                );
            }
            backups = backups.push(row.into_element());
            if let Some(form) = self
                .system
                .backups
                .retention
                .as_ref()
                .filter(|form| form.backup.is_some())
            {
                backups = backups.push(self.retention_form(form));
            }
        } else if let Some(keep) = safety.keep_last {
            backups = backups.push(crate::settings_app::settings_row(tr("shell-backups-kept")).subtitle(tr!("shell-the-last-this-backend-does-not-report-the-current-cleanup-policy", "arg0" => format!("{:}", keep))).into_element());
        }
        if let Some(cleanup) = &safety.local_cleanup {
            backups = backups.push(self.cleanup_row(tr("shell-last-local-cleanup"), cleanup, now));
        }
        if safety.remote_configured
            && let Some(cleanup) = &safety.remote_cleanup
        {
            backups =
                backups.push(self.cleanup_row(tr("shell-last-network-cleanup"), cleanup, now));
        }
        backups = backups.push(self.mark_setting(
            "backups/now",
            if safety.destination() || self.system.jobs.contains_key(&LongJob::BackupNow) {
                self.job_row(
                    LongJob::BackupNow,
                    tr("shell-back-up-now"),
                    tr("shell-makes-a-new-encrypted-backup-of-the-system-and-your-files"),
                    tr("shell-back-up-now"),
                )
            } else {
                crate::settings_app::settings_row(tr("shell-back-up-now"))
                    .subtitle(tr("shell-available-once-a-backup-destination-is-set-up"))
                    .icon("hard-drive-upload")
                    .into_element()
            },
        ));
        body = body.push(
            self.section_card_for("backups/schedule", backups)
                .title(tr("shell-encrypted-backups"))
                .description(tr("shell-encrypted-copies-of-the-whole-system-and-your-files-on-another-drive-or-a-ne"))
                .into_element(),
        );
        body = body.push(self.backup_rules_card(safety));
        body = body.push(self.backup_space_card(safety));

        let Some(points) = &safety.points else {
            let row = self.info_row(
                "archive",
                if safety.points_error.is_some() { tr("shell-backups-could-not-be-read") } else { tr("shell-backup-browsing-is-not-available") },
                safety.points_error.as_deref().unwrap_or_else(|| tr("shell-this-system-service-does-not-offer-a-backup-list-for-settings-this-does-not")),
            );
            return body.push(
                self.section_card_for("backups/points", self.mark_setting("backups/points", row))
                    .title(tr("shell-restore-from-a-backup"))
                    .description(tr("shell-bring-back-a-file-or-folder-from-any-backup-it-arrives-as-a-new-file-so-noth"))
                    .into_element(),
            );
        };
        {
            let state = &self.system.backups;
            let mut rows = column![].spacing(2);
            if let Some(banner) = self.outcome_banner(false) {
                rows = rows.push(iced::widget::container(banner).padding([6, 4]));
            }
            if let Some(form) = &state.restore {
                rows = rows.push(self.restore_form(form, safety));
            }
            if points.is_empty() {
                rows = rows.push(self.mark_setting(
                    "backups/points",
                    self.info_row(
                        "archive",
                        tr("shell-no-backups-yet"),
                        tr("shell-each-backup-you-make-appears-here-ready-to-restore-from"),
                    ),
                ));
            }
            let shown = if state.all_points {
                points.len()
            } else {
                points.len().min(SHORT_LIST)
            };
            for (index, point) in points[..shown].iter().enumerate() {
                let row = self.point_row(point, safety, now);
                rows = rows.push(if index == 0 && state.restore.is_none() {
                    self.mark_setting("backups/points", row)
                } else {
                    row
                });
            }
            if points.len() > SHORT_LIST {
                let label = if state.all_points {
                    tr("shell-show-fewer").to_owned()
                } else {
                    tr!("shell-show-all-2", "arg0" => format!("{:}", points.len()))
                };
                rows = rows.push(
                    iced::widget::container(
                        enz_button(label, send(BackupsMessage::AllPoints(!state.all_points)))
                            .kind(ButtonKind::Ghost)
                            .size(ButtonSize::Sm)
                            .into_element(),
                    )
                    .padding([4, 8]),
                );
            }
            body = body.push(
                self.section_card_for("backups/points", rows)
                    .title(tr("shell-restore-from-a-backup"))
                    .description(tr("shell-bring-back-a-file-or-folder-from-any-backup-it-arrives-as-a-new-file-so-noth"))
                    .into_element(),
            );
        }
        body
    }
}
