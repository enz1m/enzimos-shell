use super::*;
use enzui_shell::disks::{Disk, FILESYSTEMS, size_text, valid_label};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::{column, container, row};
use iced::{Alignment, Length};

/// A change being prepared or confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::settings_app) enum Form {
    Format {
        disk: String,
        path: String,
        name: String,
        mounted: bool,
        size: u64,
    },
    Create {
        disk: String,
        table: String,
        offset: u64,
        max: u64,
    },
    Delete {
        disk: String,
        path: String,
        name: String,
        mounted: bool,
    },
    Erase {
        disk: String,
        name: String,
    },
}

impl Form {
    fn disk(&self) -> &str {
        match self {
            Self::Format { disk, .. }
            | Self::Create { disk, .. }
            | Self::Delete { disk, .. }
            | Self::Erase { disk, .. } => disk,
        }
    }
}

#[derive(Default)]
pub(super) struct Disks {
    disks: Option<Result<Vec<Disk>, String>>,
    loading: bool,
    epoch: u64,
    form: Option<Form>,
    /// Name and size of the open form's disk as the list showed them.
    target: Option<(String, u64)>,
    fs: String,
    label: String,
    /// Size in GB as typed (create).
    size: String,
    /// Partition table for Erase.
    table: String,
    busy: bool,
    error: Option<String>,
    done: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum DisksMessage {
    Read(u64, Result<Vec<Disk>, String>),
    Open(Form),
    Fs(String),
    Label(String),
    Size(String),
    Table(String),
    Cancel,
    Confirm,
    Done(Result<String, String>),
}

fn wrap(message: DisksMessage) -> Parent {
    Parent::System(Message::Disks(message))
}

fn fs_name(fs: &str) -> &'static str {
    match fs {
        "ext4" => tr("shell-disks-fs-ext4"),
        "btrfs" => tr("shell-disks-fs-btrfs"),
        "exfat" => tr("shell-disks-fs-exfat"),
        "ntfs" => tr("shell-disks-fs-ntfs"),
        "vfat" => tr("shell-disks-fs-fat32"),
        "crypto_LUKS" => tr("shell-disks-fs-encrypted"),
        "swap" => tr("shell-disks-fs-swap"),
        "" => tr("shell-disks-fs-empty"),
        _ => tr("shell-disks-fs-other"),
    }
}

/// Typed gigabytes (decimal comma or point) to bytes, within `max`.
fn parse_size(text: &str, max: u64) -> Option<u64> {
    let value: f64 = text.trim().replace(',', ".").parse().ok()?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let bytes = (value * 1_000_000_000.0) as u64;
    // Within 1 MB of the whole region means "all of it".
    if bytes.saturating_add(1_000_000) >= max {
        Some(0)
    } else {
        Some(bytes / (1024 * 1024) * (1024 * 1024)).filter(|b| *b >= 16 * 1024 * 1024)
    }
}

impl SettingsApp {
    pub(super) fn disks_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.disks;
        if self.system.backend.demo {
            if state.disks.is_none() {
                state.disks = Some(Ok(enzui_shell::disks::demo()));
            }
            return Task::none();
        }
        // The list follows drives being plugged in and removed; an open form keeps
        // the list it was opened from.
        if state.loading || state.busy || !(force || state.disks.is_none() || state.form.is_none())
        {
            return Task::none();
        }
        let Some(epoch) = state.epoch.checked_add(1) else {
            return Task::none();
        };
        state.epoch = epoch;
        state.loading = true;
        Task::perform(enzui_shell::disks::read(), move |result| {
            wrap(DisksMessage::Read(epoch, result))
        })
    }

    pub(super) fn disks_message(&mut self, message: DisksMessage) -> Task<Parent> {
        let demo = self.system.backend.demo;
        // A read-only preview shows this computer's hardware without changing it.
        if !self.system.backend.writes_allowed && matches!(message, DisksMessage::Confirm) {
            self.error = Some(tr("shell-system-changes-are-off-in-this-preview").into());
            return Task::none();
        }
        let state = &mut self.system.disks;
        match message {
            DisksMessage::Read(epoch, result) => {
                if epoch == state.epoch {
                    state.loading = false;
                    state.disks = Some(result);
                }
                Task::none()
            }
            DisksMessage::Open(form) => {
                if state.busy {
                    return Task::none();
                }
                let disk = state
                    .disks
                    .as_ref()
                    .and_then(|d| d.as_ref().ok())
                    .and_then(|d| d.iter().find(|disk| disk.path == form.disk()));
                let removable = disk.is_some_and(|disk| disk.removable);
                state.target = disk.map(|disk| (disk.name.clone(), disk.size));
                // Drives that travel get the file system every computer reads.
                state.fs = if removable {
                    "exfat".into()
                } else {
                    "ext4".into()
                };
                state.label.clear();
                state.size = match &form {
                    Form::Create { max, .. } => format!("{:.1}", *max as f64 / 1_000_000_000.0),
                    _ => String::new(),
                };
                state.table = if removable {
                    "dos".into()
                } else {
                    "gpt".into()
                };
                state.error = None;
                state.done = None;
                state.form = Some(form);
                Task::none()
            }
            DisksMessage::Fs(fs) => {
                self.menu = None;
                if FILESYSTEMS.iter().any(|(name, _)| *name == fs) {
                    self.system.disks.fs = fs;
                }
                Task::none()
            }
            DisksMessage::Label(label) => {
                state.label = label.chars().take(64).collect();
                Task::none()
            }
            DisksMessage::Size(size) => {
                state.size = size
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                    .take(12)
                    .collect();
                Task::none()
            }
            DisksMessage::Table(table) => {
                self.menu = None;
                if table == "gpt" || table == "dos" {
                    self.system.disks.table = table;
                }
                Task::none()
            }
            DisksMessage::Cancel => {
                if !state.busy {
                    state.form = None;
                    state.error = None;
                }
                Task::none()
            }
            DisksMessage::Confirm => {
                let Some(form) = state.form.clone() else {
                    return Task::none();
                };
                if state.busy {
                    return Task::none();
                }
                let fs = state.fs.clone();
                let label = state.label.trim().to_owned();
                if matches!(form, Form::Format { .. } | Form::Create { .. })
                    && !valid_label(&fs, &label)
                {
                    state.error = Some(tr("shell-disks-label-invalid").into());
                    return Task::none();
                }
                // Btrfs cannot be made on less than about 115 MB.
                let room = match &form {
                    Form::Format { size, .. } => *size,
                    Form::Create { max, .. } => match parse_size(&state.size, *max) {
                        Some(0) => *max,
                        Some(size) => size,
                        None => *max,
                    },
                    _ => u64::MAX,
                };
                if fs == "btrfs" && room < 120_000_000 {
                    state.error = Some(tr("shell-disks-btrfs-too-small").into());
                    return Task::none();
                }
                let size = match &form {
                    Form::Create { max, .. } => match parse_size(&state.size, *max) {
                        Some(size) => size,
                        None => {
                            state.error = Some(tr("shell-disks-size-invalid").into());
                            return Task::none();
                        }
                    },
                    _ => 0,
                };
                state.busy = true;
                state.error = None;
                if demo {
                    return Task::done(wrap(DisksMessage::Done(Ok(tr("shell-disks-done").into()))));
                }
                let table = state.table.clone();
                let target = state.target.clone();
                Task::perform(
                    async move {
                        // Another drive can take a removed drive's place under the
                        // same device path: act only on the one the form named.
                        let current = enzui_shell::disks::read().await?;
                        if !current.iter().any(|disk| {
                            disk.path == form.disk()
                                && target.as_ref().is_some_and(|(name, size)| {
                                    disk.name == *name && disk.size == *size
                                })
                        }) {
                            return Err(tr("shell-disks-changed").to_owned());
                        }
                        match form {
                            Form::Format { path, .. } => {
                                enzui_shell::disks::format(&path, &fs, &label).await
                            }
                            Form::Create {
                                disk,
                                table,
                                offset,
                                ..
                            } => {
                                enzui_shell::disks::create(&disk, &table, offset, size, &fs, &label)
                                    .await
                            }
                            Form::Delete { path, .. } => enzui_shell::disks::delete(&path).await,
                            Form::Erase { disk, .. } => {
                                enzui_shell::disks::erase(&disk, &table).await
                            }
                        }
                        .map(|()| tr("shell-disks-done").to_owned())
                    },
                    |result| wrap(DisksMessage::Done(result)),
                )
            }
            DisksMessage::Done(result) => {
                state.busy = false;
                match result {
                    Ok(done) => {
                        state.form = None;
                        state.done = Some(done);
                    }
                    Err(error) => state.error = Some(error),
                }
                if demo {
                    Task::none()
                } else {
                    self.disks_read(true)
                }
            }
        }
    }

    fn disk_bar<'a>(&self, disk: &Disk) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        let mut pieces: Vec<(u64, u64, bool, String)> = disk
            .partitions
            .iter()
            .map(|p| {
                (
                    p.offset,
                    p.size,
                    true,
                    if p.label.is_empty() {
                        fs_name(&p.content).to_owned()
                    } else {
                        p.label.clone()
                    },
                )
            })
            .chain(
                disk.free
                    .iter()
                    .map(|f| (f.offset, f.size, false, String::new())),
            )
            .collect();
        pieces.sort_by_key(|(offset, ..)| *offset);
        if pieces.is_empty() {
            pieces.push((
                0,
                disk.size,
                !disk.content.is_empty(),
                fs_name(&disk.content).to_owned(),
            ));
        }
        let total = disk.size.max(1);
        let mut bar = row![].spacing(3).height(Length::Fixed(28.0));
        for (index, (_, size, used, name)) in pieces.into_iter().enumerate() {
            // Every piece stays visible, however small.
            let portion =
                u16::try_from((size.saturating_mul(1000) / total).clamp(12, 1000)).unwrap_or(12);
            let fill = if !used {
                palette.surface_alt
            } else if disk.system {
                palette.surface_selected
            } else if index % 2 == 0 {
                palette.accent_soft
            } else {
                palette.surface_hover
            };
            let caption = if portion >= 90 && !name.is_empty() {
                format!("{name} · {}", size_text(size))
            } else {
                String::new()
            };
            bar = bar.push(
                container(enz_text(caption).size(TextSize::Caption).into_element())
                    .padding([4, 8])
                    .width(Length::FillPortion(portion))
                    .height(Length::Fill)
                    .align_y(Alignment::Center)
                    .clip(true)
                    .style(move |_| container::Style {
                        background: Some(fill.into()),
                        border: iced::Border {
                            radius: 6.0.into(),
                            width: if used { 0.0 } else { 1.0 },
                            color: palette.border_strong,
                        },
                        ..Default::default()
                    }),
            );
        }
        bar.into()
    }

    fn disk_form<'a>(&'a self, form: &Form) -> Element<'a, Parent> {
        let state = &self.system.disks;
        let route_fs = "disks/form-fs";
        let route_table = "disks/form-table";
        let mut rows = column![].spacing(2);
        let (title, warning, confirm) = match form {
            Form::Format { name, mounted, .. } => (
                tr!("shell-disks-format-title", "arg0" => name.clone()),
                if *mounted {
                    tr("shell-disks-format-warning-mounted").to_owned()
                } else {
                    tr("shell-disks-format-warning").to_owned()
                },
                tr("shell-disks-format"),
            ),
            Form::Create { max, .. } => (
                tr!("shell-disks-create-title", "arg0" => size_text(*max)),
                tr("shell-disks-create-detail").to_owned(),
                tr("shell-disks-create"),
            ),
            Form::Delete { name, mounted, .. } => (
                tr!("shell-disks-delete-title", "arg0" => name.clone()),
                if *mounted {
                    tr("shell-disks-delete-warning-mounted").to_owned()
                } else {
                    tr("shell-disks-delete-warning").to_owned()
                },
                tr("shell-disks-delete"),
            ),
            Form::Erase { name, .. } => (
                tr!("shell-disks-erase-title", "arg0" => name.clone()),
                tr("shell-disks-erase-warning").to_owned(),
                tr("shell-disks-erase"),
            ),
        };
        rows = rows.push(self.info_row("triangle-alert", &title, &warning));
        if matches!(form, Form::Format { .. } | Form::Create { .. }) {
            let choices: Vec<(String, Parent)> = FILESYSTEMS
                .iter()
                .map(|(fs, _)| {
                    (
                        fs_name(fs).to_owned(),
                        wrap(DisksMessage::Fs((*fs).to_owned())),
                    )
                })
                .collect();
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-disks-file-system"))
                    .subtitle(match state.fs.as_str() {
                        "exfat" => tr("shell-disks-fs-exfat-detail"),
                        "vfat" => tr("shell-disks-fs-fat32-detail"),
                        "ntfs" => tr("shell-disks-fs-ntfs-detail"),
                        "btrfs" => tr("shell-disks-fs-btrfs-detail"),
                        _ => tr("shell-disks-fs-ext4-detail"),
                    })
                    .icon("hard-drive")
                    .value_menu(
                        fs_name(&state.fs).to_owned(),
                        self.menu.as_deref() == Some(route_fs),
                        Parent::Menu(route_fs.into(), true),
                        Parent::Menu(route_fs.into(), false),
                        choices,
                    )
                    .into_element(),
            );
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-disks-name"))
                    .icon("tag")
                    .entry(&state.label, tr("shell-disks-name-placeholder"), |text| {
                        wrap(DisksMessage::Label(text))
                    })
                    .into_element(),
            );
        }
        if matches!(form, Form::Create { .. }) {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-disks-size-gb"))
                    .icon("ruler")
                    .entry(&state.size, "", |text| wrap(DisksMessage::Size(text)))
                    .into_element(),
            );
        }
        if matches!(form, Form::Erase { .. }) {
            let choices = vec![
                (
                    tr("shell-disks-table-gpt").to_owned(),
                    wrap(DisksMessage::Table("gpt".into())),
                ),
                (
                    tr("shell-disks-table-mbr").to_owned(),
                    wrap(DisksMessage::Table("dos".into())),
                ),
            ];
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-disks-table"))
                    .subtitle(tr("shell-disks-table-detail"))
                    .icon("layout-list")
                    .value_menu(
                        if state.table == "dos" {
                            tr("shell-disks-table-mbr")
                        } else {
                            tr("shell-disks-table-gpt")
                        },
                        self.menu.as_deref() == Some(route_table),
                        Parent::Menu(route_table.into(), true),
                        Parent::Menu(route_table.into(), false),
                        choices,
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
        let destructive = !matches!(form, Form::Create { .. });
        let buttons = row![
            iced::widget::Space::new().width(Length::Fill),
            enz_button(tr("shell-cancel"), wrap(DisksMessage::Cancel))
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id("disks-cancel".into())
                .enabled(!state.busy)
                .into_element(),
            enz_button(
                if state.busy {
                    tr("shell-disks-working")
                } else {
                    confirm
                },
                wrap(DisksMessage::Confirm)
            )
            .kind(if destructive {
                ButtonKind::Destructive
            } else {
                ButtonKind::Accent
            })
            .size(ButtonSize::Sm)
            .id("disks-confirm".into())
            .enabled(!state.busy)
            .into_element(),
        ]
        .spacing(8)
        .padding([6, 12]);
        container(rows.push(buttons))
            .padding(4)
            .style({
                let palette = self.tokens().palette;
                move |_| container::Style {
                    background: Some(palette.surface_alt.into()),
                    border: iced::Border {
                        radius: 10.0.into(),
                        width: 1.0,
                        color: palette.border_strong,
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    pub(super) fn disks_page<'a>(
        &'a self,
        mut body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let route = "disks/list";
        let state = &self.system.disks;
        let disks = match &state.disks {
            None => {
                return body.push(
                    self.section_card_for(
                        route,
                        column![self.mark_setting(route, self.loading_row())],
                    )
                    .title(tr("shell-disks"))
                    .into_element(),
                );
            }
            Some(Err(error)) => {
                return body.push(
                    self.section_card_for(
                        route,
                        column![self.mark_setting(
                            route,
                            self.info_row(
                                "hard-drive",
                                tr("shell-disks-service-did-not-answer"),
                                error
                            )
                        )],
                    )
                    .title(tr("shell-disks"))
                    .into_element(),
                );
            }
            Some(Ok(disks)) => disks,
        };
        if let Some(done) = &state.done {
            body = body.push(
                enz_banner(done.clone())
                    .tone(BannerTone::Success)
                    .into_element(),
            );
        }
        let idle = !state.busy && state.form.is_none();
        for (disk_index, disk) in disks.iter().enumerate() {
            let locked = disk.system || disk.read_only;
            let mut rows = column![self.disk_bar(disk)].spacing(6);
            if let Some(form) = state.form.as_ref().filter(|f| f.disk() == disk.path) {
                rows = rows.push(self.disk_form(form));
            }
            let mut list = column![].spacing(2);
            if disk.system {
                list = list.push(self.info_row(
                    "lock",
                    tr("shell-disks-system-disk"),
                    tr("shell-disks-system-disk-detail"),
                ));
            }
            for (index, part) in disk.partitions.iter().enumerate() {
                let name = if !part.label.is_empty() {
                    part.label.clone()
                } else if !part.kind.is_empty() {
                    part.kind.clone()
                } else {
                    tr!("shell-disks-partition-n", "arg0" => format!("{}", part.number))
                };
                let mut subtitle = format!("{} · {}", fs_name(&part.content), size_text(part.size));
                if let Some(mount) = part.mounts.first() {
                    subtitle = format!(
                        "{subtitle} · {}",
                        tr!("shell-disks-mounted-at", "arg0" => mount.clone())
                    );
                }
                let mounted = !part.mounts.is_empty();
                let control: Element<'a, Parent> = if locked || part.system {
                    self.status_badge(tr("shell-disks-in-use"), BadgeTone::Default)
                } else {
                    row![
                        enz_button(
                            tr("shell-disks-format"),
                            wrap(DisksMessage::Open(Form::Format {
                                disk: disk.path.clone(),
                                path: part.path.clone(),
                                name: name.clone(),
                                mounted,
                                size: part.size
                            }))
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id(format!("disks-format-{disk_index}-{index}").into())
                        .enabled(idle)
                        .into_element(),
                        enz_button(
                            tr("shell-disks-delete"),
                            wrap(DisksMessage::Open(Form::Delete {
                                disk: disk.path.clone(),
                                path: part.path.clone(),
                                name: name.clone(),
                                mounted
                            }))
                        )
                        .kind(ButtonKind::DestructiveOutlined)
                        .size(ButtonSize::Sm)
                        .id(format!("disks-delete-{disk_index}-{index}").into())
                        .enabled(idle)
                        .into_element(),
                    ]
                    .spacing(6)
                    .into()
                };
                let row = crate::settings_app::settings_row(name)
                    // Room for both buttons beside the title.
                    .accessory_width(if locked || part.system {
                        enzui_theme::text_scale::ui_px(90.0)
                    } else {
                        enzui_theme::text_scale::ui_px(200.0)
                    })
                    .subtitle(subtitle)
                    // The disk's own row says it is locked; partitions keep the drive glyph.
                    .icon("hard-drive")
                    .control(control)
                    .into_element();
                list = list.push(if disk_index == 0 && index == 0 {
                    self.mark_setting(route, row)
                } else {
                    row
                });
            }
            if disk.partitions.is_empty() && !disk.table.is_empty() && disk.free.is_empty() {
                list = list.push(self.info_row("hard-drive", tr("shell-disks-no-partitions"), ""));
            }
            if disk.table.is_empty() && !disk.content.is_empty() && !locked {
                list = list.push(
                    crate::settings_app::settings_row(if disk.label.is_empty() {
                        fs_name(&disk.content).to_owned()
                    } else {
                        disk.label.clone()
                    })
                    .subtitle(format!(
                        "{} · {}",
                        fs_name(&disk.content),
                        tr("shell-disks-no-partition-table")
                    ))
                    .icon("hard-drive")
                    .control(
                        enz_button(
                            tr("shell-disks-format"),
                            wrap(DisksMessage::Open(Form::Format {
                                disk: disk.path.clone(),
                                path: disk.path.clone(),
                                name: disk.name.clone(),
                                mounted: false,
                                size: disk.size,
                            })),
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id(format!("disks-format-whole-{disk_index}").into())
                        .enabled(idle)
                        .into_element(),
                    )
                    .into_element(),
                );
            }
            for (index, free) in disk.free.iter().enumerate() {
                list = list.push(
                    crate::settings_app::settings_row(tr("shell-disks-unallocated"))
                        .subtitle(size_text(free.size))
                        .icon("square-dashed")
                        .control(
                            enz_button(
                                tr("shell-disks-create"),
                                wrap(DisksMessage::Open(Form::Create {
                                    disk: disk.path.clone(),
                                    table: disk.table.clone(),
                                    offset: free.offset,
                                    max: free.size,
                                })),
                            )
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id(format!("disks-create-{disk_index}-{index}").into())
                            .enabled(idle && !locked)
                            .into_element(),
                        )
                        .into_element(),
                );
            }
            if !locked {
                list = list.push(
                    crate::settings_app::settings_row(tr("shell-disks-erase-disk"))
                        .subtitle(tr("shell-disks-erase-disk-detail"))
                        .icon("eraser")
                        .control(
                            enz_button(
                                tr("shell-disks-erase"),
                                wrap(DisksMessage::Open(Form::Erase {
                                    disk: disk.path.clone(),
                                    name: disk.name.clone(),
                                })),
                            )
                            .kind(ButtonKind::DestructiveOutlined)
                            .size(ButtonSize::Sm)
                            .id(format!("disks-erase-{disk_index}").into())
                            .enabled(idle)
                            .into_element(),
                        )
                        .into_element(),
                );
            }
            rows = rows.push(list);
            let kind = if disk.removable {
                if disk.bus == "usb" {
                    tr("shell-disks-usb-drive")
                } else {
                    tr("shell-disks-removable")
                }
            } else {
                tr("shell-disks-internal")
            };
            let table = match disk.table.as_str() {
                "gpt" => " · GPT",
                "dos" => " · MBR",
                _ => "",
            };
            let description = format!("{} · {kind}{table}", size_text(disk.size));
            body = body.push(
                self.section_card_for(route, rows)
                    .title(&disk.name)
                    .description(&description)
                    .into_element(),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn typed_sizes_round_to_whole_mebibytes_and_the_whole_region() {
        let max = 400_000_000_000;
        assert_eq!(parse_size("400", max), Some(0), "all of it");
        assert_eq!(
            parse_size("100,5", max),
            Some(100_500_000_000 / 1_048_576 * 1_048_576)
        );
        assert_eq!(parse_size("0", max), None);
        assert_eq!(parse_size("0.001", max), None, "too small");
        assert_eq!(parse_size("abc", max), None);
    }
}
