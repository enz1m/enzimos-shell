use super::*;
use enzui_shell::developer_tools::{Operation, Overview, Profile, Review};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::{column, container, row};

pub(super) struct Job {
    pub epoch: u64,
    pub review: Review,
    pub state: String,
    pub transfer: Option<(u64, u64)>,
    pub outcome: Option<Result<(), String>>,
}

#[derive(Default)]
pub(super) struct Developer {
    pub overview: Option<Result<Option<Overview>, String>>,
    pub loading: bool,
    pub read_epoch: u64,
    pub handler_epoch: u64,
    pub handler_pending: Option<u64>,
    pub review: Option<Result<Review, String>>,
    pub review_profile: Option<String>,
    pub reviewing: Option<(u64, Operation, String)>,
    pub epoch: u64,
    pub job: Option<Job>,
    pub handlers: Option<(Option<String>, Vec<enzui_shell::default_apps::App>)>,
    pub handler_error: Option<String>,
    pub tools_expanded: std::collections::HashSet<String>,
}

impl Developer {
    fn retire_precommit(&mut self) {
        self.reviewing = None;
        self.review = None;
        self.review_profile = None;
        self.loading = false;
        self.read_epoch = self.read_epoch.saturating_add(1);
        self.handler_pending = None;
        self.handler_epoch = self.handler_epoch.saturating_add(1);
        if self.job.as_ref().is_some_and(|job| {
            job.outcome.is_some() || enzui_shell::developer_tools::cancellable(&job.state)
        }) {
            self.job = None;
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum DeveloperMessage {
    Read(u64, Result<Option<Overview>, String>),
    Handlers(u64, Option<String>, Vec<enzui_shell::default_apps::App>),
    Review(Operation, String),
    Reviewed(u64, Result<Review, String>),
    Start(u64),
    Discard(u64),
    Progress(u64, enzui_shell::developer_tools::Progress),
    SetHandler(String),
    HandlerSet(u64, Result<(), String>),
    Tools(String, bool),
}

fn profile_icon(id: &str) -> &'static str {
    match id {
        "build" => "hammer",
        "runtimes" => "layers",
        "containers" => "container",
        "virtualization" => "monitor-cog",
        _ => "package",
    }
}

pub(super) fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => {
            tr!("shell-gb-3", "arg0" => enzui_shell::i18n::number(format!("{:.1}", b as f64 / f64::from(1u32 << 30))))
        }
        b if b >= 1 << 20 => {
            tr!("shell-mb-5", "arg0" => enzui_shell::i18n::number(format!("{:.0}", b as f64 / f64::from(1u32 << 20))))
        }
        b => tr!("shell-kb-3", "arg0" => format!("{:}", b.div_ceil(1024))),
    }
}

fn names(list: &[String]) -> String {
    let shown: Vec<&str> = list.iter().take(6).map(String::as_str).collect();
    let more = list.len().saturating_sub(shown.len());
    if more > 0 {
        tr!("shell-and-more-2", "arg0" => shown.join(", "), "arg1" => format!("{:}", more))
    } else {
        shown.join(", ")
    }
}

fn demo_overview(installed: &[&str]) -> Overview {
    let profile = |id: &str, label: &str, packages: &[&str]| Profile {
        id: id.into(),
        label: label.into(),
        packages: packages.iter().map(|p| (*p).to_owned()).collect(),
        tools: packages
            .iter()
            .map(|p| enzui_shell::developer_tools::Tool {
                package: (*p).into(),
                status: enzui_shell::developer_tools::PackageStatus::Unknown,
            })
            .collect(),
        installed: installed.contains(&id),
        enrolled: installed.contains(&id),
    };
    Overview {
        profiles: vec![
            profile(
                "build",
                tr("shell-git-and-build-tools"),
                &["git", "base-devel", "cmake", "ninja", "meson", "pkgconf"],
            ),
            profile("runtimes", tr("shell-language-runtimes"), &["mise"]),
            profile(
                "containers",
                tr("shell-rootless-containers"),
                &["podman", "buildah", "skopeo", "passt", "fuse-overlayfs"],
            ),
            profile(
                "virtualization",
                tr("shell-local-virtual-machines"),
                &["qemu-base", "libvirt", "edk2-ovmf", "swtpm", "passt"],
            ),
        ],
        ready: true,
        unavailable: None,
        removal: true,
        download: true,
    }
}

/// The key of the one "Package status" section listing every tool set.
const ALL_TOOLS: &str = "all";

impl SettingsApp {
    pub(super) fn developer_read(&mut self, force: bool) -> Task<Parent> {
        if self.system.backend.demo {
            let state = &mut self.system.developer;
            if state.overview.is_none() {
                state.overview = Some(Ok(Some(demo_overview(&["build"]))));
                let app = |id: &str, name: &str| enzui_shell::default_apps::App {
                    id: id.into(),
                    name: name.into(),
                    program: None,
                    icon: None,
                };
                state.handlers = Some((
                    Some("dev.zed.Zed.desktop".into()),
                    vec![
                        app("org.gnome.Builder.desktop", "Builder"),
                        app("dev.zed.Zed.desktop", "Zed"),
                    ],
                ));
            }
            return Task::none();
        }
        let state = &mut self.system.developer;
        if state.loading
            || state.handler_pending.is_some()
            || !(force || state.overview.is_none())
            || state.job.as_ref().is_some_and(|job| job.outcome.is_none())
        {
            return Task::none();
        }
        let Some(epoch) = state.read_epoch.checked_add(1) else {
            return Task::none();
        };
        state.read_epoch = epoch;
        state.loading = true;
        Task::batch([
            Task::perform(enzui_shell::developer_tools::read(), move |result| {
                Parent::System(Message::Developer(DeveloperMessage::Read(epoch, result)))
            }),
            Task::perform(
                async {
                    tokio::task::spawn_blocking(enzui_shell::default_apps::project_handlers)
                        .await
                        .unwrap_or_default()
                },
                move |(current, apps)| {
                    Parent::System(Message::Developer(DeveloperMessage::Handlers(
                        epoch, current, apps,
                    )))
                },
            ),
        ])
    }

    pub(super) fn developer_message(&mut self, message: DeveloperMessage) -> Task<Parent> {
        let state = &mut self.system.developer;
        match message {
            DeveloperMessage::Read(epoch, result) => {
                if self.closing || epoch != state.read_epoch {
                    return Task::none();
                }
                state.loading = false;
                state.overview = Some(result);
                Task::none()
            }
            DeveloperMessage::Handlers(epoch, current, apps) => {
                if self.closing || epoch != state.read_epoch {
                    return Task::none();
                }
                state.handlers = Some((current, apps));
                Task::none()
            }
            DeveloperMessage::Review(operation, profile) => {
                if self.closing
                    || self.page != Category::Developer
                    || !self.filter.trim().is_empty()
                    || !self.system.backend.writes_allowed
                    || state.reviewing.is_some()
                    || state.job.as_ref().is_some_and(|job| job.outcome.is_none())
                {
                    return Task::none();
                }
                let Some(Ok(Some(overview))) = &state.overview else {
                    return Task::none();
                };
                let Some(offered) = overview.profiles.iter().find(|p| p.id == profile) else {
                    return Task::none();
                };
                if match operation {
                    Operation::Install => {
                        !overview.ready || (offered.enrolled && offered.installed)
                    }
                    Operation::Remove => !overview.removal || !offered.enrolled,
                    Operation::Download => !overview.download || offered.installed,
                } {
                    return Task::none();
                }
                let Some(epoch) = state.epoch.checked_add(1) else {
                    return Task::none();
                };
                state.epoch = epoch;
                state.job = None;
                state.review = None;
                state.review_profile = Some(profile.clone());
                state.reviewing = Some((epoch, operation, profile.clone()));
                if self.system.backend.demo {
                    let packages = state
                        .overview
                        .as_ref()
                        .and_then(|o| {
                            o.as_ref()
                                .ok()?
                                .as_ref()?
                                .profiles
                                .iter()
                                .find(|p| p.id == profile)
                                .map(|p| p.packages.clone())
                        })
                        .unwrap_or_default();
                    let review = Review {
                        operation,
                        profile,
                        plan_sha256: "0".repeat(64),
                        owner: None,
                        packages,
                        others: vec!["libslirp".into(), "conmon".into()],
                        download_bytes: (operation != Operation::Remove).then_some(84 << 20),
                        installed: if operation == Operation::Install {
                            enzui_shell::developer_tools::Space::Known(310 << 20)
                        } else {
                            enzui_shell::developer_tools::Space::NotReported
                        },
                        free_bytes: Some(24 << 30),
                        cache_free_bytes: Some(24 << 30),
                    };
                    return Task::done(Parent::System(Message::Developer(
                        DeveloperMessage::Reviewed(epoch, Ok(review)),
                    )));
                }
                Task::perform(
                    enzui_shell::developer_tools::review(operation, profile),
                    move |result| {
                        Parent::System(Message::Developer(DeveloperMessage::Reviewed(
                            epoch, result,
                        )))
                    },
                )
            }
            DeveloperMessage::Reviewed(epoch, result) => {
                let Some((pending, operation, profile)) = &state.reviewing else {
                    return Task::none();
                };
                if self.closing || epoch != *pending || epoch != state.epoch {
                    return Task::none();
                }
                let result = result.and_then(|review| {
                    if review.operation == *operation && review.profile == *profile {
                        Ok(review)
                    } else {
                        Err(
                            tr("shell-developer-review-does-not-match-the-requested-profile")
                                .into(),
                        )
                    }
                });
                state.reviewing = None;
                state.review = Some(result);
                Task::none()
            }
            DeveloperMessage::Start(epoch) => {
                if epoch != state.epoch
                    || self.closing
                    || self.page != Category::Developer
                    || !self.filter.trim().is_empty()
                    || !self.system.backend.writes_allowed
                    || state.reviewing.is_some()
                    || state.job.as_ref().is_some_and(|job| job.outcome.is_none())
                {
                    return Task::none();
                }
                let Some(Ok(review)) = state.review.take() else {
                    return Task::none();
                };
                let epoch = state.epoch;
                let download = review.operation == Operation::Download;
                state.job = Some(Job {
                    epoch,
                    review,
                    state: String::new(),
                    transfer: None,
                    outcome: None,
                });
                if self.system.backend.demo {
                    let finish = move |progress| {
                        Parent::System(Message::Developer(DeveloperMessage::Progress(
                            epoch, progress,
                        )))
                    };
                    return Task::batch([
                        Task::done(finish(enzui_shell::developer_tools::Progress::State(
                            if download {
                                "downloading".into()
                            } else {
                                "committing".into()
                            },
                        ))),
                        Task::perform(tokio::time::sleep(Duration::from_secs(3)), move |()| {
                            finish(enzui_shell::developer_tools::Progress::Finished(Ok(())))
                        }),
                    ]);
                }
                Task::none()
            }
            DeveloperMessage::Discard(epoch) => {
                if epoch != state.epoch {
                    return Task::none();
                }
                state.retire_precommit();
                Task::none()
            }
            DeveloperMessage::Progress(epoch, progress) => {
                let Some(job) = state
                    .job
                    .as_mut()
                    .filter(|job| job.epoch == epoch && job.outcome.is_none())
                else {
                    return Task::none();
                };
                match progress {
                    enzui_shell::developer_tools::Progress::State(now) => {
                        job.state = now;
                        job.transfer = None;
                        Task::none()
                    }
                    enzui_shell::developer_tools::Progress::Transfer { bytes, total_bytes } => {
                        job.state = "downloading".into();
                        job.transfer = Some((bytes, total_bytes));
                        Task::none()
                    }
                    enzui_shell::developer_tools::Progress::Finished(result) => {
                        if self.system.backend.demo
                            && result.is_ok()
                            && job.review.operation != Operation::Download
                            && let Some(Ok(Some(overview))) = state.overview.as_mut()
                            && let Some(profile) = overview
                                .profiles
                                .iter_mut()
                                .find(|p| p.id == job.review.profile)
                        {
                            profile.installed = job.review.operation == Operation::Install;
                            profile.enrolled = profile.installed;
                        }
                        job.outcome = Some(result);
                        self.developer_read(true)
                    }
                }
            }
            DeveloperMessage::SetHandler(id) => {
                if self.closing
                    || self.page != Category::Developer
                    || !self.filter.trim().is_empty()
                    || !self.system.backend.writes_allowed
                    || state.handler_pending.is_some()
                    || !state
                        .handlers
                        .as_ref()
                        .is_some_and(|(_, apps)| apps.iter().any(|app| app.id == id))
                {
                    return Task::none();
                }
                self.menu = None;
                let state = &mut self.system.developer;
                // The preview's handlers are samples: choose among them, touch nothing.
                if self.system.backend.demo {
                    if let Some((current, _)) = state.handlers.as_mut() {
                        *current = Some(id);
                    }
                    return Task::none();
                }
                let Some(epoch) = state.handler_epoch.checked_add(1) else {
                    return Task::none();
                };
                state.handler_epoch = epoch;
                state.handler_pending = Some(epoch);
                state.handler_error = None;
                state.loading = false;
                state.read_epoch = state.read_epoch.saturating_add(1);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            if !enzui_shell::default_apps::project_handlers()
                                .1
                                .iter()
                                .any(|app| app.id == id)
                            {
                                return Err(tr(
                                    "shell-the-project-application-is-no-longer-available",
                                )
                                .into());
                            }
                            enzui_shell::default_apps::set_project_handler(&id)
                        })
                        .await
                        .unwrap_or_else(|_| Err(tr("shell-settings-stopped").into()))
                    },
                    move |result| {
                        Parent::System(Message::Developer(DeveloperMessage::HandlerSet(
                            epoch, result,
                        )))
                    },
                )
            }
            DeveloperMessage::Tools(profile, expanded) => {
                if self.closing
                    || profile != ALL_TOOLS
                        && !state.overview.as_ref().is_some_and(|result| {
                            result.as_ref().is_ok_and(|overview| {
                                overview.as_ref().is_some_and(|overview| {
                                    overview
                                        .profiles
                                        .iter()
                                        .any(|offered| offered.id == profile)
                                })
                            })
                        })
                {
                    return Task::none();
                }
                if expanded {
                    state.tools_expanded.insert(profile);
                } else {
                    state.tools_expanded.remove(&profile);
                }
                Task::none()
            }
            DeveloperMessage::HandlerSet(epoch, result) => {
                if self.closing || state.handler_pending != Some(epoch) {
                    return Task::none();
                }
                state.handler_pending = None;
                state.handler_error = result.err();
                self.developer_read(true)
            }
        }
    }

    pub(in crate::settings_app) fn developer_leave(&mut self) {
        self.system.developer.retire_precommit();
    }

    pub(crate) fn developer_subscription(&self) -> iced::Subscription<Parent> {
        match &self.system.developer.job {
            Some(job) if job.outcome.is_none() && !self.system.backend.demo => {
                iced::Subscription::run_with(
                    (job.epoch, job.review.clone()),
                    |(_, review): &(u64, Review)| enzui_shell::developer_tools::run(review.clone()),
                )
                .with(job.epoch)
                .map(|(epoch, progress)| {
                    Parent::System(Message::Developer(DeveloperMessage::Progress(
                        epoch, progress,
                    )))
                })
            }
            _ => iced::Subscription::none(),
        }
    }

    fn developer_review_panel<'a>(&'a self, review: &'a Review) -> Element<'a, Parent> {
        let mut lines = column![].spacing(4);
        let (verb, primary) = match review.operation {
            Operation::Install if review.packages.is_empty() && review.others.is_empty() => {
                (tr("shell-installs"), tr("shell-set-up"))
            }
            Operation::Install => (tr("shell-installs"), tr("shell-install")),
            Operation::Remove => (tr("shell-removes"), tr("shell-remove")),
            Operation::Download => (tr("shell-caches"), tr("shell-download-only")),
        };
        lines = lines.push(self.caption(if review.packages.is_empty() {
            tr!("shell-nothing-new", "arg0" => verb.to_owned())
        } else {
            format!("{verb} {}", review.packages.join(", "))
        }));
        if !review.others.is_empty() {
            lines = lines.push(self.caption(match review.operation {
                Operation::Install => tr!("shell-also-installs", "arg0" => review.others.join(", ")),
                Operation::Remove => tr!("shell-keeps-still-needed-or-installed-before", "arg0" => review.others.join(", ")),
                Operation::Download => tr!("shell-also-caches", "arg0" => review.others.join(", ")),
            }));
        }
        if let Some(bytes) = review.download_bytes.filter(|b| *b > 0) {
            let cached = review.operation == Operation::Install
                && matches!(
                    review.installed,
                    enzui_shell::developer_tools::Space::Known(_)
                );
            lines = lines.push(self.caption(if cached { tr!("shell-uses-of-packages-already-downloaded-and-verified", "arg0" => size_text(bytes)) } else { tr!("shell-downloads-2", "arg0" => size_text(bytes)) }));
            if review.operation == Operation::Download
                && let Some(free) = review.cache_free_bytes.filter(|free| bytes > *free)
            {
                lines = lines.push(self.caption(tr!("shell-only-is-free-for-the-download-free-up-space-first", "arg0" => size_text(free))));
            }
        }
        match review.installed {
            enzui_shell::developer_tools::Space::Known(bytes) if bytes > 0 => {
                lines =
                    lines.push(self.caption(
                        tr!("shell-needs-on-the-system-drive", "arg0" => size_text(bytes)),
                    ));
                if let Some(free) = review.free_bytes.filter(|free| bytes > *free) {
                    lines = lines.push(self.caption(tr!("shell-only-is-free-on-the-system-drive-free-up-space-before-setting-up-or-se", "arg0" => size_text(free))));
                }
            }
            enzui_shell::developer_tools::Space::Unknown => {
                lines = lines.push(self.caption(tr(
                    "shell-the-space-it-needs-is-shown-once-the-packages-are-downloaded",
                )));
            }
            _ => {}
        }
        if review.operation == Operation::Download {
            lines = lines.push(self.caption(tr(
                "shell-downloads-verified-packages-and-signatures-to-the-os-cache-only-no-packages",
            )));
        }
        if review.operation == Operation::Remove {
            lines = lines.push(self.caption(tr("shell-projects-containers-images-and-settings-are-kept-stop-running-containers-and")));
        }
        let buttons = row![
            enz_button(
                tr("shell-cancel"),
                Parent::System(Message::Developer(DeveloperMessage::Discard(
                    self.system.developer.epoch
                )))
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .id("developer-review-cancel".into())
            .into_element(),
            enz_button(
                primary,
                Parent::System(Message::Developer(DeveloperMessage::Start(
                    self.system.developer.epoch
                )))
            )
            .kind(if review.operation == Operation::Remove {
                ButtonKind::Destructive
            } else {
                ButtonKind::Accent
            })
            .size(ButtonSize::Sm)
            .id("developer-review-start".into())
            .enabled(self.controls_live())
            .into_element(),
        ]
        .spacing(8);
        container(column![lines, container(buttons).align_right(iced::Length::Fill)].spacing(10))
            .padding(iced::Padding {
                top: 4.0,
                right: 12.0,
                bottom: 10.0,
                left: 44.0,
            })
            .into()
    }

    fn developer_profile<'a>(
        &'a self,
        index: usize,
        profile: &'a Profile,
        overview: &'a Overview,
    ) -> Element<'a, Parent> {
        let state = &self.system.developer;
        let busy = state.reviewing.is_some()
            || state.job.as_ref().is_some_and(|job| job.outcome.is_none());
        let review = |operation: Operation| {
            Parent::System(Message::Developer(DeveloperMessage::Review(
                operation,
                profile.id.clone(),
            )))
        };
        let remove = overview.removal.then(|| {
            enz_button(tr("shell-remove-2"), review(Operation::Remove))
                .kind(if profile.installed {
                    ButtonKind::Secondary
                } else {
                    ButtonKind::Plain
                })
                .size(ButtonSize::Sm)
                .enabled(!busy && self.controls_live())
                .id(format!("developer-remove-{index}").into())
                .into_element()
        });
        // Every action of a tool set sits in its row; a quiet secondary
        // action leads the main one.
        let mut controls = row![].spacing(6).align_y(iced::Alignment::Center);
        let mut labels: Vec<&str> = Vec::new();
        match (profile.enrolled, profile.installed) {
            (true, true) => {
                controls =
                    controls.push(self.status_badge(tr("shell-installed"), BadgeTone::Success));
                labels.push(tr("shell-installed"));
                if let Some(remove) = remove {
                    controls = controls.push(remove);
                    labels.push(tr("shell-remove-2"));
                }
            }
            (true, false) => {
                if let Some(remove) = remove {
                    controls = controls.push(remove);
                    labels.push(tr("shell-remove-2"));
                }
                controls = controls.push(
                    enz_button(tr("shell-restore-tools"), review(Operation::Install))
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .enabled(!busy && overview.ready && self.controls_live())
                        .id(format!("developer-restore-{}", profile.id).into())
                        .into_element(),
                );
                labels.push(tr("shell-restore-tools"));
            }
            (false, installed) => {
                if overview.download && !installed {
                    controls = controls.push(
                        enz_button(tr("shell-download-only-2"), review(Operation::Download))
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Sm)
                            .enabled(!busy && self.controls_live())
                            .id(format!("developer-download-{}", profile.id).into())
                            .into_element(),
                    );
                    labels.push(tr("shell-download-only-2"));
                }
                let label = if installed {
                    tr("shell-set-up-2")
                } else {
                    tr("shell-install-2")
                };
                controls = controls.push(
                    enz_button(label, review(Operation::Install))
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .enabled(!busy && overview.ready && self.controls_live())
                        .id(format!("developer-install-{index}").into())
                        .into_element(),
                );
                labels.push(label);
            }
        }
        let control: Element<'a, Parent> = controls.into();
        let subtitle = match (profile.installed, profile.enrolled) {
            (true, false) => {
                tr!("shell-already-installed-setting-up-prepares-your-account", "arg0" => names(&profile.packages))
            }
            (false, true) => {
                tr!("shell-set-up-but-some-tools-were-removed", "arg0" => names(&profile.packages))
            }
            _ => names(&profile.packages),
        };
        let mut tokens = crate::settings_app::settings_row_tokens();
        let badge = enzui_widgets::badge::BadgeSize::Md;
        let button = enzui_widgets::button::ButtonSize::Sm.tokens();
        let width = |text: &str, size| {
            enzui_widgets::text_measure::enz_text_measure_width(
                text,
                enzui_widgets::text_measure::MeasureTypography::new(TextSize::Body)
                    .font_family(
                        self.tokens()
                            .font_catalog
                            .family(enzui_theme::FontRole::Ui)
                            .to_iced(),
                    )
                    .weight(enzui_theme::text_defaults::weight(
                        enzui_theme::WeightRung::Semibold,
                    ))
                    .pin_size_px(size),
            )
        };
        // The row keeps its actions beside the title rather than restacking.
        let slot: f32 = labels
            .iter()
            .map(|label| {
                width(label, button.text_size) + button.padding_x.max(badge.padding_x()) * 2.0 + 6.0
            })
            .sum();
        tokens.inline_field_width = tokens.inline_field_width.max(slot + 4.0);
        let mut column = column![
            crate::settings_app::settings_row(profile.label.clone())
                .tokens(tokens)
                .subtitle(subtitle)
                .subtitle_wrap(true)
                .icon(profile_icon(&profile.id))
                .control(control)
                .into_element()
        ];
        if state
            .reviewing
            .as_ref()
            .is_some_and(|(_, _, id)| *id == profile.id)
        {
            column = column.push(
                container(self.caption(tr("shell-preparing-the-review"))).padding(iced::Padding {
                    top: 0.0,
                    right: 12.0,
                    bottom: 8.0,
                    left: 44.0,
                }),
            );
        }
        match &state.review {
            Some(Ok(review)) if review.profile == profile.id => {
                column = column.push(self.developer_review_panel(review));
            }
            Some(Err(error))
                if state.job.is_none() && state.review_profile.as_deref() == Some(&profile.id) =>
            {
                column = column.push(container(self.caption(error.as_str())).padding(
                    iced::Padding {
                        top: 0.0,
                        right: 12.0,
                        bottom: 8.0,
                        left: 44.0,
                    },
                ));
            }
            _ => {}
        }
        if let Some(job) = state.job.as_ref().filter(|job| {
            job.review.profile == profile.id
                && (job.review.operation == Operation::Download
                    || !matches!(job.outcome, Some(Ok(()))))
        }) {
            let line: Element<'a, Parent> = match &job.outcome {
                None => {
                    let mut line = row![enzui_widgets::spinner::enz_spinner().size(14.0).into_element(), self.caption(job.transfer.map_or_else(|| enzui_shell::developer_tools::state_label(&job.state).to_owned(), |(bytes, total)| enzui_shell::tr!("shell-downloading-of", "arg0" => enzui_shell::i18n::argument(&(size_text(bytes))), "arg1" => enzui_shell::i18n::argument(&(size_text(total))))))]
                        .spacing(8)
                        .align_y(iced::Alignment::Center);
                    if enzui_shell::developer_tools::cancellable(&job.state) {
                        line = line.push(iced::widget::Space::new().width(iced::Length::Fill)).push(
                            enz_button(tr("shell-cancel"), Parent::System(Message::Developer(DeveloperMessage::Discard(job.epoch)))).kind(ButtonKind::Secondary).size(ButtonSize::Sm).id("developer-job-cancel".into()).into_element(),
                        );
                    }
                    line.into()
                }
                Some(Ok(())) => self.caption(if job.review.operation != Operation::Download { "" }
                    else if profile.enrolled && !profile.installed { tr("shell-download-verified-nothing-installed-restore-tools-needs-a-new-review-and-app") }
                    else { tr("shell-download-verified-nothing-installed-install-needs-a-new-review-and-approval") }),
                Some(Err(error)) => self.caption(error.as_str()),
            };
            column = column.push(container(line).padding(iced::Padding {
                top: 0.0,
                right: 12.0,
                bottom: 10.0,
                left: 44.0,
            }));
        }
        column.into()
    }

    /// Package versions of every tool set, folded into one section.
    fn developer_versions<'a>(&'a self, overview: &'a Overview) -> Option<Element<'a, Parent>> {
        let profiles: Vec<&Profile> = overview
            .profiles
            .iter()
            .filter(|p| !p.tools.is_empty())
            .collect();
        if profiles.is_empty() {
            return None;
        }
        let mut tools = column![self.caption(tr(
            "shell-installed-package-versions-reported-by-enzimos-this-does-not-resolve-a-proje"
        ))]
        .spacing(4);
        for profile in profiles {
            tools = tools.push(
                container(
                    enz_text(profile.label.clone())
                        .size(TextSize::Footnote)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                )
                .padding(iced::Padding {
                    top: 6.0,
                    ..iced::Padding::ZERO
                }),
            );
            for tool in &profile.tools {
                let status = match &tool.status {
                    enzui_shell::developer_tools::PackageStatus::Installed(version) => {
                        version.as_str()
                    }
                    enzui_shell::developer_tools::PackageStatus::Missing => {
                        tr("shell-not-installed")
                    }
                    enzui_shell::developer_tools::PackageStatus::Unknown => tr("shell-unknown"),
                };
                tools = tools.push(self.caption(format!("{} — {status}", tool.package)));
            }
        }
        // A row like its neighbours; the versions follow it when shown.
        let open = self.system.developer.tools_expanded.contains(ALL_TOOLS);
        let toggle = crate::settings_app::settings_row(tr("shell-installed-package-versions"))
            .icon("package")
            .action(
                if open {
                    tr("shell-hide")
                } else {
                    tr("shell-show")
                },
                Parent::System(Message::Developer(DeveloperMessage::Tools(
                    ALL_TOOLS.into(),
                    !open,
                ))),
            )
            .id("developer-tools-versions".into())
            .into_element();
        Some(if open {
            column![
                toggle,
                container(tools).padding(iced::Padding {
                    left: 44.0,
                    right: 12.0,
                    bottom: 8.0,
                    ..iced::Padding::ZERO
                })
            ]
            .spacing(2)
            .into()
        } else {
            toggle
        })
    }

    pub(super) fn developer_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let route = "developer/tools";
        let state = &self.system.developer;
        let mut rows = column![].spacing(2);
        match &state.overview {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(Err(error)) => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "triangle-alert",
                        tr("shell-developer-tools-could-not-be-read"),
                        error,
                    ),
                ));
            }
            Some(Ok(None)) => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "code",
                        tr("shell-developer-tools-are-not-available"),
                        tr("shell-this-system-does-not-offer-the-developer-preset"),
                    ),
                ));
            }
            Some(Ok(Some(overview))) => {
                if !overview.ready {
                    let reason = match overview.unavailable.as_deref() {
                        Some("extension-not-installed") | None => {
                            tr("shell-install-the-enzimos-developer-extension-to-add-these-tools")
                        }
                        Some(_) => tr(
                            "shell-the-developer-extension-does-not-match-this-system-version-update-the-system",
                        ),
                    };
                    rows = rows.push(self.mark_setting(
                        route,
                        self.info_row(
                            "package",
                            tr("shell-the-developer-extension-is-not-installed"),
                            reason,
                        ),
                    ));
                }
                for (index, profile) in overview.profiles.iter().enumerate() {
                    let row = self.developer_profile(index, profile, overview);
                    rows = rows.push(if index == 0 && overview.ready {
                        self.mark_setting(route, row)
                    } else {
                        row
                    });
                }
                if let Some(versions) = self.developer_versions(overview) {
                    rows = rows.push(versions);
                }
            }
        }
        let projects = "developer/projects";
        let mut project_rows = column![].spacing(2);
        match &state.handlers {
            None => {
                project_rows = project_rows.push(self.mark_setting(projects, self.loading_row()));
            }
            Some((_, apps)) if apps.is_empty() => {
                project_rows = project_rows.push(self.mark_setting(projects, self.info_row("folder-code", tr("shell-no-development-app-opens-folders"), tr("shell-install-an-editor-or-ide-that-opens-project-folders-to-choose-it-here"))));
            }
            Some((current, apps)) => {
                let ids: Vec<(String, String)> = apps
                    .iter()
                    .map(|app| {
                        let label =
                            if apps.iter().filter(|other| other.name == app.name).count() > 1 {
                                format!("{} ({})", app.name, app.id)
                            } else {
                                app.name.clone()
                            };
                        (label, app.id.clone())
                    })
                    .collect();
                let current_label = current
                    .as_ref()
                    .and_then(|id| ids.iter().find(|(_, known)| known == id))
                    .map_or_else(
                        || tr("shell-choose-2").to_owned(),
                        |(label, _)| label.clone(),
                    );
                let choices: Vec<(String, Parent)> = ids
                    .into_iter()
                    .map(|(label, id)| {
                        (
                            label,
                            Parent::System(Message::Developer(DeveloperMessage::SetHandler(id))),
                        )
                    })
                    .collect();
                let menu = "developer/project-handler";
                let row = crate::settings_app::settings_row(tr("shell-open-projects-with"))
                    .subtitle(state.handler_error.clone().unwrap_or_else(|| {
                        tr("shell-used-by-open-project-ordinary-folders-keep-opening-in-files")
                            .into()
                    }))
                    .icon("folder-code")
                    .id_prefix("developer-project-handler")
                    .value_menu(
                        current_label,
                        self.menu.as_deref() == Some(menu),
                        Parent::Menu(menu.into(), true),
                        Parent::Menu(menu.into(), false),
                        choices,
                    )
                    .enabled(
                        self.controls_live() && !state.loading && state.handler_pending.is_none(),
                    )
                    .into_element();
                project_rows = project_rows.push(self.mark_setting(projects, row));
            }
        }
        rows = rows.push(crate::settings_app::settings_row(tr("shell-ports-servers-and-containers"))
            .subtitle(tr("shell-open-dev-tools-to-inspect-local-activity-nothing-is-checked-while-its-window"))
            .icon("activity")
            .control(enz_button(tr("shell-open-dev-tools"), Parent::OpenDevTools).kind(ButtonKind::Secondary).size(ButtonSize::Sm).id("developer-open-monitor".into()).enabled(!self.dev_tools_launch.pending()).into_element())
            .into_element());
        body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-developer-tools"))
                .description(tr("shell-optional-tool-sets-from-enzimos-each-change-is-reviewed-first-and-needs-an-a"))
                .into_element(),
        )
        .push(
            self.section_card_for(projects, project_rows)
                .title(tr("shell-projects"))
                .description(tr("shell-opening-a-project-never-runs-its-scripts-or-tasks-a-project-s-runtime-settin"))
                .into_element(),
        )
    }
}
