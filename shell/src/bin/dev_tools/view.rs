use super::*;
use iced::widget::column;

impl App {
    pub(super) fn view(&self) -> Element<'_, Message> {
        i18n::localizer().view(|| {
        let theme = self.themes.theme();
        let _defaults = enzui_theme::text_defaults::TextDefaultsScope::enter(theme);
        let _scale =
            enzui_theme::text_scale::TextScaleScope::enter(theme.typography.scale_factor());
        let page: Element<'_, Message> = enz_app_shell(|| self.content())
            .layout(ShellLayout::TitlebarContent)
            .titlebar(|| enz_title_bar().title(tr("shell-dev-tools")).into_element())
            .on_close(Message::Close)
            .on_window_gesture(Message::Window)
            .into();
        let dialog = self.review.as_ref().map(|review| {
            let identity = match &review.target {
                Target::Process(id) => tr!("shell-pid-start-time", "arg0" => format!("{:}", id.pid), "arg1" => format!("{:}", id.start_time)),
                Target::Container(id) => {
                    let display_id = id.id.as_bytes().chunks(32).map(|part| String::from_utf8_lossy(part)).collect::<Vec<_>>().join("\n");
                    format!("{} · {}\nContainer ID (continued on next line):\n{display_id}", id.engine, id.socket)
                }
            };
            let warning = if review.operation == Operation::ForceStop { tr("shell-force-stop-can-lose-unsaved-work-this-sends-sigkill-to-the-displayed-process") } else { tr("shell-the-displayed-identity-is-used-exactly-the-service-may-refuse-protected-chan") };
            enz_confirm_dialog(format!("{} {}?", review.operation.label(), review.label),
                format!("{identity}\n\n{warning}"), &review.state, Message::Confirm)
                .confirm_label(review.operation.label()).destructive(review.operation != Operation::Start)
                .id_prefix("dev-tools").into_surface()
        });
        let mut host = enz_modal_host(page, dialog, |event| {
            Message::Confirm(ConfirmDialogEvent::from_host(event))
        })
        .label(tr("shell-confirm-dev-tools-action"));
        if let Some(review) = &self.review {
            host = host.scoped_to(&review.focus);
        }
        host.into_element()

        })
    }

    fn content(&self) -> Element<'_, Message> {
        let mut body = column![
            enz_view_switcher(
                vec![
                    SwitcherPage::new("ports", enzui_shell::i18n::tr("shell-ports-servers")),
                    SwitcherPage::new("containers", enzui_shell::i18n::tr("shell-containers"))
                ],
                usize::from(self.options.containers),
                Message::Containers
            )
            .into_element(),
            row![
                enz_checkbox(
                    self.options.system,
                    enzui_shell::i18n::current().text("shell-system-listeners"),
                    Message::System
                )
                .id("developer-system-listeners".into())
                .into_element(),
                enz_checkbox(self.options.udp, "UDP", Message::Udp).into_element()
            ]
            .spacing(20),
            enz_text(tr(
                "shell-refreshes-only-while-this-window-is-open-closing-stops-requests-the-service"
            ))
            .size(TextSize::Footnote)
            .into_element(),
        ]
        .spacing(14);
        if let Some(error) = &self.error {
            body = body.push(
                enz_text(tr!("shell-dev-monitor-unavailable", "arg0" => error.clone()))
                    .into_element(),
            );
        }
        if let Some(status) = &self.status {
            body = body.push(enz_text(status.clone()).into_element());
        }
        if let Some((Target::Process(id), _, label)) = &self.force {
            body = body.push(
                enz_button(
                    tr!("shell-force-stop-pid-start", "arg0" => label.clone(), "arg1" => format!("{:}", id.pid), "arg2" => format!("{:}", id.start_time)),
                    Message::ForceReview,
                )
                .id("dev-tools-force".into())
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .enabled(!self.busy)
                .into_element(),
            );
        }
        if let Some(full_snapshot) = self
            .snapshot
            .as_ref()
            .filter(|_| self.container_detail.is_none())
        {
            let filtered = self
                .project
                .as_deref()
                .and_then(|key| full_snapshot.in_project(key));
            let snapshot = filtered.as_ref().unwrap_or(full_snapshot);
            if !full_snapshot.projects.is_empty() {
                body = body.push(
                    enz_text(tr(
                        "shell-projects-shared-directory-groups-no-combined-memory-total",
                    ))
                    .size(TextSize::Title3)
                    .into_element(),
                );
                let mut groups = row![
                    enz_button(tr("shell-all-activity"), Message::Project(None))
                        .id("dev-tools-project-all".into())
                        .size(ButtonSize::Sm)
                        .into_element()
                ]
                .spacing(8);
                let (range, _, _) = page_range(
                    full_snapshot.projects.len(),
                    self.pages[Section::Groups.index()],
                    Section::Groups.size(),
                );
                for (index, group) in full_snapshot
                    .projects
                    .iter()
                    .enumerate()
                    .skip(range.start)
                    .take(range.len())
                {
                    let (processes, launchers, containers) = full_snapshot.group_inventory(group);
                    groups = groups.push(enz_button(tr!("shell-processes-launchers-containers", "arg0" => group.name.clone(), "arg1" => format!("{:}", processes), "arg2" => format!("{:}", launchers), "arg3" => format!("{:}", containers)), Message::Project(Some(group.key.clone())))
                        .id(format!("dev-tools-project-{index}").into()).size(ButtonSize::Sm).into_element());
                }
                body = body.push(groups.wrap());
                if full_snapshot.projects.len() > Section::Groups.size() {
                    body = body.push(self.pager(Section::Groups, full_snapshot.projects.len()));
                }
                if let Some(group) = self
                    .project
                    .as_deref()
                    .and_then(|key| full_snapshot.project(key))
                {
                    body = body.push(enz_text(group.directory.as_ref().map_or_else(|| tr!("shell-group", "arg0" => group.name.clone()), |directory| tr!("shell-group-2", "arg0" => group.name.clone(), "arg1" => directory.clone()))).size(TextSize::Footnote).into_element());
                }
            }
            for warning in &snapshot.warnings {
                body = body.push(enz_text(warning.clone()).into_element());
            }
            if self.options.containers {
                body = body.push(self.containers(snapshot));
            } else {
                body = body.push(
                    enz_text_field(
                        enzui_shell::i18n::current().text("shell-look-up-a-port"),
                        "1–65535",
                        &self.port_query,
                        Message::PortQuery,
                    )
                    .id("dev-tools-port-query".into())
                    .on_submit(Message::Lookup)
                    .into_element(),
                );
                body = body.push(
                    enz_button(
                        tr("shell-look-up-port-includes-udp-system-engines"),
                        Message::Lookup,
                    )
                    .id("dev-tools-port-lookup".into())
                    .size(ButtonSize::Sm)
                    .enabled(
                        !self.busy
                            && !self.explicit_pending
                            && self.port_query.parse::<u16>().is_ok_and(|port| port > 0),
                    )
                    .into_element(),
                );
                if let Some((port, client, lookup)) = &self.lookup {
                    body = body.push(enz_text(tr!("shell-explicit-port-lookup-not-continuously-refreshed", "arg0" => format!("{:}", port))).size(TextSize::Title3).into_element());
                    for warning in &lookup.warnings {
                        body = body.push(enz_text(warning.clone()).into_element());
                    }
                    body = body.push(
                        enz_button(tr("shell-return-to-live-ports"), Message::ClearDetail)
                            .id("dev-tools-lookup-clear".into())
                            .size(ButtonSize::Sm)
                            .into_element(),
                    );
                    body = body.push(self.ports(lookup, "lookup", Some(client)));
                } else {
                    body = body.push(self.ports(snapshot, "snapshot", self.client.as_ref()));
                }
            }
        } else if self.error.is_none() && self.container_detail.is_none() {
            body = body.push(enz_text(tr("shell-reading-local-developer-activity")).into_element());
        }
        if self.explicit_pending {
            body = body.push(
                enz_text(tr("shell-reading-explicit-details"))
                    .size(TextSize::Footnote)
                    .into_element(),
            );
        }
        if let Some(detail) = &self.detail {
            body = body.push(enz_text(tr!("shell-pid-start-explicit-read-not-live-following", "arg0" => detail.label.clone(), "arg1" => format!("{:}", detail.identity.pid), "arg2" => format!("{:}", detail.identity.start_time))).size(TextSize::Title3).into_element());
            body = body.push(enz_text(tr!("shell-threads-rss-pss-descriptors", "arg0" => format!("{:}", detail.threads), "arg1" => memory(detail.rss_bytes), "arg2" => memory(detail.pss_bytes), "arg3" => detail.open_descriptors.map_or_else(|| tr("shell-unknown").into(), |count| count.to_string()))).into_element());
            body = body.push(enz_text(tr!("shell-storage-read-written", "arg0" => memory(detail.read_bytes), "arg1" => memory(detail.written_bytes))).into_element());
            for value in [&detail.exe, &detail.cwd, &detail.unit]
                .into_iter()
                .flatten()
            {
                body = body.push(
                    enz_text(value.clone())
                        .size(TextSize::Footnote)
                        .into_element(),
                );
            }
            body = body.push(
                enz_text(serde_json::to_string(&detail.cmdline).unwrap_or_default())
                    .size(TextSize::Footnote)
                    .into_element(),
            );
            for child in &detail.children {
                body = body.push(enz_text(tr!("shell-child-pid-start-rss", "arg0" => child.label.clone(), "arg1" => format!("{:}", child.identity.pid), "arg2" => format!("{:}", child.identity.start_time), "arg3" => memory(child.rss_bytes).clone())).size(TextSize::Footnote).into_element());
            }
            body = body.push(
                enz_button(tr("shell-close-details"), Message::ClearDetail)
                    .size(ButtonSize::Sm)
                    .into_element(),
            );
        }
        if let Some((id, detail)) = &self.container_detail {
            let display_id = id
                .id
                .as_bytes()
                .chunks(32)
                .map(|part| String::from_utf8_lossy(part))
                .collect::<Vec<_>>()
                .join("\n");
            let mut lines = column![
                enz_text(format!(
                    "{} · {} · {} · explicit displayed details, not live",
                    detail.name, id.engine, id.socket
                ))
                .size(TextSize::Title3)
                .into_element(),
                enz_text(format!("Container ID:\n{display_id}"))
                    .size(TextSize::Footnote)
                    .into_element(),
                enz_button(
                    tr("shell-close-container-details"),
                    Message::CloseContainerDetails
                )
                .id("dev-tools-container-details-close".into())
                .size(ButtonSize::Sm)
                .into_element()
            ]
            .spacing(8);
            for section in [Section::ContainerPorts, Section::ContainerMounts] {
                let count = if section == Section::ContainerPorts {
                    detail.ports.len()
                } else {
                    detail.mounts.len()
                };
                let (range, _, _) = page_range(count, self.pages[section.index()], section.size());
                lines = lines.push(enz_text(tr!("shell-entries", "arg0" => section.name().to_owned(), "arg1" => format!("{:}", count))).size(TextSize::Title3).into_element());
                if count > section.size() {
                    lines = lines.push(self.pager(section, count));
                }
                if section == Section::ContainerPorts {
                    for port in detail.ports.iter().skip(range.start).take(range.len()) {
                        lines = lines.push(
                            enz_text(format!(
                                "{} {}:{} → {}",
                                port.protocol,
                                port.host_address,
                                port.host_port,
                                port.container_port
                            ))
                            .size(TextSize::Footnote)
                            .into_element(),
                        );
                    }
                } else {
                    for mount in detail.mounts.iter().skip(range.start).take(range.len()) {
                        lines = lines.push(
                            enz_text(format!(
                                "{} {} → {} ({})",
                                mount.kind,
                                mount.source,
                                mount.destination,
                                if mount.writable {
                                    "read/write"
                                } else {
                                    "read-only"
                                }
                            ))
                            .size(TextSize::Footnote)
                            .into_element(),
                        );
                    }
                }
            }
            body = body.push(lines);
        }
        if let Some(logs) = &self.logs {
            body = body.push(
                enz_text(tr("shell-recent-logs-explicit-read-not-live-following"))
                    .size(TextSize::Title3)
                    .into_element(),
            );
            let end = logs
                .char_indices()
                .nth(16000)
                .map_or(logs.len(), |(offset, _)| offset);
            body =
                body.push(enz_text(logs.get(..end).unwrap_or_default().to_owned()).into_element());
            if end < logs.len() {
                body = body.push(
                    enz_text(
                        tr("shell-display-limited-to-the-first-16-000-characters-of-this-explicit-log-read"),
                    )
                    .size(TextSize::Footnote)
                    .into_element(),
                );
            }
        }
        enz_scroll_view(container(body).padding(20).width(Length::Fill))
            .id("dev-tools-scroll".into())
            .height(Length::Fill)
            .into_element()
    }

    fn ports<'a>(
        &self,
        snapshot: &Snapshot,
        scope: &str,
        retained_client: Option<&Client>,
    ) -> Element<'a, Message> {
        let mut rows = Vec::new();
        if snapshot.ports.is_empty() {
            rows.push((
                "empty".into(),
                enz_text(tr("shell-no-matching-listening-ports")).into_element(),
            ));
        }
        let section = if scope == "lookup" {
            Section::Lookup
        } else {
            Section::Ports
        };
        let (range, _, _) = page_range(
            snapshot.ports.len(),
            self.pages[section.index()],
            section.size(),
        );
        if snapshot.ports.len() > section.size() {
            rows.push((
                "pager-top".into(),
                self.pager(section, snapshot.ports.len()),
            ));
        }
        for port in snapshot.ports.iter().skip(range.start).take(range.len()) {
            let row_key = format!(
                "{scope}-{}-{}-{}-{:?}-{}",
                port.protocol,
                port.address,
                port.port,
                port.process_id(),
                retained_client.map_or("", Client::authority_key)
            );
            let exposure = match port.exposure.as_str() {
                "all" => tr("shell-wildcard-binding"),
                "loopback" => tr("shell-loopback"),
                _ => tr("shell-specific-address"),
            };
            let mut item = column![
                enz_text(format!(
                    "{} {}:{} · {exposure}",
                    port.protocol.to_uppercase(),
                    port.address,
                    port.port
                ))
                .size(TextSize::Title3)
                .into_element()
            ]
            .spacing(6);
            if let Some(process) = snapshot.process(port)
                && let Some(client) = retained_client.cloned()
            {
                let launcher = &process.launcher;
                item = item.push(
                    enz_text(tr!("shell-processes", "arg0" => process.label.clone(), "arg1" => launcher.label.clone(), "arg2" => format!("{:}", launcher.processes), "arg3" => (if launcher.tree_limited {
                            tr("shell-partial-tree")
                        } else {
                            ""
                        }).to_owned()))
                    .into_element(),
                );
                item = item.push(
                    enz_text(tr!("shell-launcher-cpu-rss-shared-pages-may-be-counted-twice", "arg0" => cpu(launcher.cpu_percent).clone(), "arg1" => memory(launcher.rss_bytes).clone()))
                    .size(TextSize::Footnote)
                    .into_element(),
                );
                if let Some(project) = &process.project {
                    item = item.push(
                        enz_text(tr!("shell-project", "arg0" => project.name.clone(), "arg1" => project.root.clone()))
                            .size(TextSize::Footnote)
                            .into_element(),
                    );
                }
                if let Some(cwd) = &process.cwd {
                    item = item.push(
                        enz_text(cwd.clone())
                            .size(TextSize::Footnote)
                            .into_element(),
                    );
                }
                let target = Target::Process(launcher.identity);
                let actions = row![
                    self.action_button(
                        "Stop…",
                        target.clone(),
                        Operation::Stop,
                        launcher.label.clone(),
                        &row_key,
                        client.clone()
                    ),
                    enz_button(
                        enzui_shell::i18n::tr("shell-logs"),
                        Message::Logs(Target::Process(process.identity), client.clone())
                    )
                    .id(format!("dev-tools-logs-{row_key}").into())
                    .size(ButtonSize::Sm)
                    .enabled(!self.busy && !self.explicit_pending)
                    .into_element(),
                    enz_button(
                        enzui_shell::i18n::tr("shell-inspect"),
                        Message::Inspect(process.identity, client.clone())
                    )
                    .id(format!("dev-tools-inspect-{row_key}").into())
                    .size(ButtonSize::Sm)
                    .enabled(!self.busy && !self.explicit_pending)
                    .into_element(),
                    enz_button(
                        tr("shell-copy-command-argv"),
                        Message::Copy(serde_json::to_string(&process.cmdline).unwrap_or_default())
                    )
                    .id(format!("dev-tools-command-{row_key}").into())
                    .size(ButtonSize::Sm)
                    .into_element(),
                ]
                .spacing(8)
                .wrap();
                item = item.push(actions);
                item = item.push(
                    row![
                        enz_button(
                            tr("shell-open-project-folder"),
                            Message::Shortcut(
                                Target::Process(process.identity),
                                ShortcutKind::Folder,
                                client.clone()
                            )
                        )
                        .id(format!("dev-tools-folder-{row_key}").into())
                        .size(ButtonSize::Sm)
                        .enabled(
                            !self.busy
                                && !self.explicit_pending
                                && !enzui_shell::desktop_session::mode().nested()
                        )
                        .into_element(),
                        enz_button(
                            tr("shell-open-project-terminal"),
                            Message::Shortcut(
                                Target::Process(process.identity),
                                ShortcutKind::Terminal,
                                client
                            )
                        )
                        .id(format!("dev-tools-terminal-{row_key}").into())
                        .size(ButtonSize::Sm)
                        .enabled(
                            !self.busy
                                && !self.explicit_pending
                                && enzui_shell::desktop_session::mode()
                                    == enzui_shell::desktop_session::Mode::Installed
                        )
                        .into_element()
                    ]
                    .spacing(8)
                    .wrap(),
                );
            } else {
                item = item.push(
                    enz_text(if port.source == "published" {
                        tr("shell-container-published-mapping-not-a-host-process-owner")
                    } else {
                        tr("shell-process-identity-unavailable-or-owned-by-another-user")
                    })
                    .size(TextSize::Footnote)
                    .into_element(),
                );
            }
            if let Some(url) = &port.url {
                item = item.push(
                    enz_button(
                        tr("shell-copy-suggested-url-not-probed"),
                        Message::Copy(url.clone()),
                    )
                    .id(format!("dev-tools-url-{row_key}").into())
                    .size(ButtonSize::Sm)
                    .into_element(),
                );
            }
            if let Ok(launch) = UrlLaunch::from_port(port) {
                item = item.push(
                    enz_button(
                        tr("shell-open-suggested-url-not-probed"),
                        Message::OpenUrl(launch),
                    )
                    .id(format!("dev-tools-open-url-{row_key}").into())
                    .size(ButtonSize::Sm)
                    .enabled(
                        !self.busy
                            && !self.explicit_pending
                            && !enzui_shell::desktop_session::mode().nested(),
                    )
                    .into_element(),
                );
            }
            for claim in &port.published_by {
                item = item.push(
                    enz_text(tr!("shell-published-mapping-claim", "arg0" => claim.name.clone(), "arg1" => claim.engine.clone()))
                    .size(TextSize::Footnote)
                    .into_element(),
                );
            }
            rows.push((row_key, item.into()));
        }
        if snapshot.ports.len() > section.size() {
            rows.push((
                "pager-bottom".into(),
                self.pager_at(section, snapshot.ports.len(), "bottom"),
            ));
        }
        enzui_widgets::keyed_column::keyed_column(rows)
            .spacing(16.0)
            .into()
    }

    fn pager<'a>(&self, section: Section, count: usize) -> Element<'a, Message> {
        self.pager_at(section, count, "top")
    }
    fn pager_at<'a>(
        &self,
        section: Section,
        count: usize,
        placement: &str,
    ) -> Element<'a, Message> {
        let (range, page, pages) = page_range(count, self.pages[section.index()], section.size());
        column![
            enz_text(format!(
                "{} {}–{} of {count}",
                section.name(),
                range.start + 1,
                range.end
            ))
            .size(TextSize::Footnote)
            .into_element(),
            enzui_widgets::pagination::enz_pagination(pages as i32, page as i32 + 1, move |page| {
                Message::Page(section, page.saturating_sub(1) as usize)
            })
            .id_prefix(format!("dev-tools-pages-{}-{placement}", section.index()))
            .group_label(section.name())
            .into_element()
        ]
        .spacing(8)
        .into()
    }

    fn containers<'a>(&self, snapshot: &Snapshot) -> Element<'a, Message> {
        let mut rows = column![].spacing(16);
        if snapshot.engines.is_empty() {
            rows = rows.push(
                enz_text(tr("shell-no-accessible-docker-or-podman-endpoints")).into_element(),
            );
            if enzui_shell::dev_monitor::podman::api_off() {
                rows = rows.push(enz_text(tr("shell-podman-is-installed-but-its-api-socket-for-your-account-is-off-dev-tools-rea"))
                    .size(TextSize::Footnote).into_element());
                rows = rows.push(
                    enz_button(tr("shell-turn-on-the-podman-api"), Message::PodmanApi)
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("dev-tools-podman-api".into())
                        .enabled(!self.busy)
                        .into_element(),
                );
            }
        }
        let count = snapshot
            .engines
            .iter()
            .map(|engine| engine.containers.len())
            .sum();
        let (range, _, _) = page_range(
            count,
            self.pages[Section::Containers.index()],
            Section::Containers.size(),
        );
        if count > Section::Containers.size() {
            rows = rows.push(self.pager(Section::Containers, count));
        }
        let mut offset = 0;
        for engine in &snapshot.engines {
            let end = offset + engine.containers.len();
            let local_start = range
                .start
                .saturating_sub(offset)
                .min(engine.containers.len());
            let local_end = range
                .end
                .saturating_sub(offset)
                .min(engine.containers.len());
            offset = end;
            if local_start == local_end && !engine.containers.is_empty() {
                continue;
            }
            rows = rows.push(
                enz_text(format!(
                    "{} · {} · {}",
                    engine.endpoint.engine,
                    engine.endpoint.socket,
                    if engine.endpoint.rootless {
                        "rootless"
                    } else {
                        tr("shell-system-endpoint")
                    }
                ))
                .size(TextSize::Title3)
                .into_element(),
            );
            if engine.stale || engine.truncated {
                rows = rows.push(
                    enz_text(tr(
                        "shell-inventory-is-stale-or-incomplete-controls-are-disabled",
                    ))
                    .into_element(),
                );
            }
            if let Some(error) = &engine.error {
                rows = rows.push(enz_text(error.clone()).into_element());
            }
            let mut containers: Vec<_> = engine.containers.iter().collect();
            containers.sort_by_key(|c| (c.state != "running", &c.name));
            for container in containers
                .into_iter()
                .skip(local_start)
                .take(local_end.saturating_sub(local_start))
            {
                let startup = match container.autostart {
                    Some(true) => tr("shell-configured-automatic-startup"),
                    Some(false) => tr("shell-no-configured-automatic-startup"),
                    None => tr("shell-automatic-startup-unknown"),
                };
                let mut item = column![
                    enz_text(format!(
                        "{} · {}{}",
                        container.name,
                        container.state,
                        container
                            .health
                            .as_ref()
                            .map(|h| format!(" · {h}"))
                            .unwrap_or_default()
                    ))
                    .into_element(),
                    enz_text(format!("{} · {startup}", container.image))
                        .size(TextSize::Footnote)
                        .into_element(),
                    enz_text(format!(
                        "{} CPU · {} memory · {} limit",
                        cpu(container.cpu_percent),
                        memory(container.memory_bytes),
                        memory(container.memory_limit_bytes)
                    ))
                    .size(TextSize::Footnote)
                    .into_element(),
                ]
                .spacing(6);
                if matches!(container.state.as_str(), "running" | "paused") {
                    item = item.push(
                        enz_text(observed::uptime(
                            container.started_at.as_deref(),
                            chrono::Utc::now(),
                        ))
                        .size(TextSize::Footnote)
                        .into_element(),
                    );
                }
                if let Some(startup) = &container.startup {
                    item = item.push(
                        enz_text(tr!("shell-login-boot", "arg0" => known(startup.at_login).to_owned(), "arg1" => known(startup.at_boot).to_owned(), "arg2" => startup.reason.clone()))
                        .size(TextSize::Footnote)
                        .into_element(),
                    );
                }
                if let Some(startup) = &container.startup {
                    item = item.push(enz_text(tr!("shell-engine-start-policy-user-unit-linger", "arg0" => known(startup.on_engine_start).to_owned(), "arg1" => known(startup.user_unit).to_owned(), "arg2" => known(startup.linger).to_owned())).size(TextSize::Footnote).into_element());
                    if let Some(unit) = &startup.unit {
                        item = item.push(enz_text(tr!("shell-unit", "arg0" => unit.clone(), "arg1" => startup.unit_file_state.as_deref().unwrap_or_else(|| tr("shell-enablement-unknown")).to_owned(), "arg2" => startup.verification_age_ms.map_or_else(|| tr("shell-probe-age-unknown").into(), |age| format!("Probe {age} ms ago")).clone())).size(TextSize::Footnote).into_element());
                    }
                }
                if let Some(compose) = &container.compose {
                    item = item.push(
                        enz_text(tr!("shell-compose", "arg0" => compose.project.clone(), "arg1" => compose
                                .working_dir
                                .as_deref()
                                .unwrap_or_else(|| tr("shell-host-directory-unknown")).to_owned()))
                        .size(TextSize::Footnote)
                        .into_element(),
                    );
                }
                if let Some(id) = engine.identity(container) {
                    let key = format!("{}-{}-{}", id.engine, id.socket, id.id);
                    item = item.push(
                        enz_button(
                            tr("shell-details"),
                            Message::ContainerDetails(id, Box::new(container.clone())),
                        )
                        .id(format!("dev-tools-details-{key}").into())
                        .size(ButtonSize::Sm)
                        .into_element(),
                    );
                }
                for port in container.ports.iter().take(32) {
                    item = item.push(
                        enz_text(format!(
                            "{} {}:{} → {}",
                            port.protocol, port.host_address, port.host_port, port.container_port
                        ))
                        .size(TextSize::Footnote)
                        .into_element(),
                    );
                }
                for mount in container.mounts.iter().take(32) {
                    item = item.push(
                        enz_text(format!(
                            "{} {} → {} ({})",
                            mount.kind,
                            mount.source,
                            mount.destination,
                            if mount.writable {
                                "read/write"
                            } else {
                                "read-only"
                            }
                        ))
                        .size(TextSize::Footnote)
                        .into_element(),
                    );
                }
                if container.ports.len() > 32 || container.mounts.len() > 32 {
                    item = item.push(
                        enz_text(tr("shell-first-32-port-mount-entries-shown-here-details-contains-the-full-received-li"))
                            .size(TextSize::Footnote)
                            .into_element(),
                    );
                }
                for error in [&container.detail_error, &container.stats_error]
                    .into_iter()
                    .flatten()
                {
                    item = item.push(enz_text(error.clone()).into_element());
                }
                if let Some(id) = engine.identity(container)
                    && let Some(client) = self.client.clone()
                {
                    let row_key = format!("{}-{}-{}", id.engine, id.socket, id.id);
                    let target = Target::Container(id);
                    // Offer only what the engine's reported state allows; an
                    // unrecognised state keeps every control.
                    let (start, stop) = match container.state.as_str() {
                        "running" | "paused" | "restarting" => (false, true),
                        "created" | "exited" | "dead" => (true, false),
                        _ => (true, true),
                    };
                    let mut controls = row![].spacing(8);
                    if start {
                        controls = controls.push(self.action_button(
                            "Start…",
                            target.clone(),
                            Operation::Start,
                            container.name.clone(),
                            &row_key,
                            client.clone(),
                        ));
                    }
                    if stop {
                        controls = controls.push(self.action_button(
                            "Stop…",
                            target.clone(),
                            Operation::Stop,
                            container.name.clone(),
                            &row_key,
                            client.clone(),
                        ));
                        controls = controls.push(self.action_button(
                            "Restart…",
                            target.clone(),
                            Operation::Restart,
                            container.name.clone(),
                            &row_key,
                            client.clone(),
                        ));
                    }
                    controls = controls.push(
                        enz_button(
                            tr("shell-logs"),
                            Message::Logs(target.clone(), client.clone()),
                        )
                        .id(format!("dev-tools-logs-{row_key}").into())
                        .size(ButtonSize::Sm)
                        .enabled(!self.busy && !self.explicit_pending)
                        .into_element(),
                    );
                    item = item.push(controls.wrap());
                    if container.compose.is_some() {
                        item = item.push(
                            row![
                                enz_button(
                                    tr("shell-open-compose-folder"),
                                    Message::Shortcut(
                                        target.clone(),
                                        ShortcutKind::Folder,
                                        client.clone()
                                    )
                                )
                                .id(format!("dev-tools-folder-{row_key}").into())
                                .size(ButtonSize::Sm)
                                .enabled(
                                    !self.busy
                                        && !self.explicit_pending
                                        && !enzui_shell::desktop_session::mode().nested()
                                )
                                .into_element(),
                                enz_button(
                                    tr("shell-open-compose-terminal"),
                                    Message::Shortcut(target, ShortcutKind::Terminal, client)
                                )
                                .id(format!("dev-tools-terminal-{row_key}").into())
                                .size(ButtonSize::Sm)
                                .enabled(
                                    !self.busy
                                        && !self.explicit_pending
                                        && enzui_shell::desktop_session::mode()
                                            == enzui_shell::desktop_session::Mode::Installed
                                )
                                .into_element()
                            ]
                            .spacing(8)
                            .wrap(),
                        );
                    }
                }
                rows = rows.push(item);
            }
        }
        if count > Section::Containers.size() {
            rows = rows.push(self.pager_at(Section::Containers, count, "bottom"));
        }
        rows.into()
    }

    fn action_button<'a>(
        &self,
        label: &'a str,
        target: Target,
        operation: Operation,
        name: String,
        row_key: &str,
        client: Client,
    ) -> Element<'a, Message> {
        let id = match &target {
            Target::Process(id) => format!(
                "dev-tools-process-{}-{}-{}",
                id.pid,
                id.start_time,
                operation.label()
            ),
            Target::Container(id) => format!(
                "dev-tools-container-{}-{}-{}-{}",
                id.engine,
                id.socket,
                id.id,
                operation.label()
            ),
        };
        let id = format!("{id}-{row_key}");
        enz_button(
            label,
            Message::Review(target, operation, name, id.clone(), client),
        )
        .id(id.into())
        .size(ButtonSize::Sm)
        .kind(ButtonKind::Secondary)
        .enabled(!self.busy)
        .into_element()
    }
}
