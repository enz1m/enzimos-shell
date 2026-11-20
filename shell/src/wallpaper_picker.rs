use super::{Message as AppMessage, SettingsApp};
use enzui_core::file_browser::{EntryKind, FileBrowserModel, FileEntry, SortKey};
use enzui_shell::i18n::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    file_browser::enz_file_browser,
    keyed_column::keyed_column,
    search_field::enz_search_field,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::{
    Element, Length, Task,
    widget::{column, container, row},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) struct State {
    pub theme: enzui_theme::EnzTheme,
    ticket: u64,
    folder: PathBuf,
    location: String,
    all: FileBrowserModel,
    shown: FileBrowserModel,
    selected: Option<PathBuf>,
    truncated: bool,
    more_matches: bool,
    error: Option<String>,
}
impl State {
    fn project(&mut self) {
        let visible = self.all.visible();
        self.more_matches = visible.len() > 256;
        self.shown
            .set_entries(visible.into_iter().take(256).cloned().collect());
        self.shown
            .set_sort(self.all.sort_key(), self.all.direction());
        if self
            .selected
            .as_ref()
            .is_some_and(|path| self.shown.entry_at(path).is_none())
        {
            self.selected = None;
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Listing {
    folder: PathBuf,
    entries: Vec<FileEntry>,
    truncated: bool,
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    Location(String),
    Go(u64),
    Up(u64),
    Filter(String),
    Cancel(u64),
    Select(u64, PathBuf),
    Activate(u64, PathBuf),
    Sort(u64, SortKey),
    Accept(u64, Option<PathBuf>),
    Loaded(u64, Result<Listing, String>),
}
fn message(value: Message) -> AppMessage {
    AppMessage::WallpaperPicker(value)
}

fn listing(path: PathBuf) -> Result<Listing, String> {
    if !path.is_absolute() {
        return Err(tr("shell-enter-an-absolute-local-folder-path").into());
    }
    let started = Instant::now();
    let mut folder = path
        .canonicalize()
        .map_err(|_| tr("shell-cannot-open-this-folder"))?;
    if folder.is_file() {
        folder = folder
            .parent()
            .ok_or_else(|| tr("shell-image-has-no-parent-folder"))?
            .to_path_buf();
    }
    let directory = std::fs::read_dir(&folder).map_err(|_| tr("shell-cannot-read-this-folder"))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut bytes = 0;
    for (index, entry) in directory.enumerate() {
        if index >= 4096 || started.elapsed() > Duration::from_secs(5) || bytes >= 2 * 1024 * 1024 {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') || name.chars().any(char::is_control) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let item = if kind.is_dir() {
            FileEntry::directory(path, name)
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| {
                    ["png", "jpg", "jpeg", "webp"]
                        .iter()
                        .any(|valid| ext.eq_ignore_ascii_case(valid))
                })
        {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > 32 * 1024 * 1024 {
                continue;
            }
            FileEntry::file(path, name, metadata.len())
        } else {
            continue;
        };
        bytes += item.path.as_os_str().len() + item.name.len() + 128;
        entries.push(item);
    }
    Ok(Listing {
        folder,
        entries,
        truncated,
    })
}

impl SettingsApp {
    fn picker_ticket(&mut self) -> Option<u64> {
        let next = self.picker_generation.checked_add(1)?;
        self.picker_generation = next;
        Some(next)
    }
    pub(super) fn start_wallpaper_picker(&mut self) -> Task<AppMessage> {
        self.start_image_picker(super::ImagePurpose::Desktop)
    }
    pub(super) fn start_image_picker(&mut self, purpose: super::ImagePurpose) -> Task<AppMessage> {
        if self.closing
            || self.wallpaper.busy
            || self.picker_read.is_some()
            || (!self.wallpaper.supported && matches!(purpose, super::ImagePurpose::Desktop))
        {
            return Task::none();
        }
        let source = Path::new(self.wallpaper.path.trim());
        let folder = if source.is_absolute() {
            source.to_path_buf()
        } else {
            let gallery = super::wallpaper_settings::preview::gallery_folder();
            if gallery.is_dir() {
                gallery
            } else {
                gallery.parent().map(Path::to_path_buf).unwrap_or(gallery)
            }
        };
        let folder = folder
            .ancestors()
            .find(|path| path.exists())
            .map_or_else(|| PathBuf::from("/"), Path::to_path_buf);
        self.image_purpose = purpose;
        self.menu = None;
        self.picker = Some(State {
            theme: self.tokens(),
            ticket: 0,
            location: folder.to_string_lossy().into_owned(),
            folder: folder.clone(),
            all: FileBrowserModel::new(),
            shown: FileBrowserModel::new(),
            selected: None,
            truncated: false,
            more_matches: false,
            error: None,
        });
        self.load_picker_folder(folder)
    }
    fn load_picker_folder(&mut self, folder: PathBuf) -> Task<AppMessage> {
        if self.picker_read.is_some() || self.closing {
            return Task::none();
        }
        let Some(ticket) = self.picker_ticket() else {
            self.error = Some(tr("shell-folder-request-identifiers-exhausted").into());
            return Task::none();
        };
        let Some(picker) = &mut self.picker else {
            return Task::none();
        };
        picker.ticket = ticket;
        picker.selected = None;
        picker.error = None;
        picker.location = folder.to_string_lossy().into_owned();
        picker.all.set_entries(Vec::new());
        picker.shown.set_entries(Vec::new());
        self.picker_read = Some(ticket);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || listing(folder))
                    .await
                    .map_err(|_| tr("shell-folder-reader-stopped").to_owned())
                    .and_then(|result| result)
            },
            move |result| message(Message::Loaded(ticket, result)),
        )
    }
    fn activate_picker_path(&mut self, ticket: u64, path: PathBuf) -> Task<AppMessage> {
        let Some(picker) = &self.picker else {
            return Task::none();
        };
        if picker.ticket != ticket || self.picker_read.is_some() || self.closing {
            return Task::none();
        }
        let Some(entry) = picker.shown.entry_at(&path) else {
            return Task::none();
        };
        if entry.kind == EntryKind::Directory {
            return self.load_picker_folder(path);
        }
        if entry.kind != EntryKind::File {
            return Task::none();
        }
        let Some(path) = path.to_str() else {
            return Task::none();
        };
        path.clone_into(&mut self.wallpaper.path);
        self.picker = None;
        self.wallpaper_import()
    }
    pub(super) fn wallpaper_picker_message(&mut self, event: Message) -> Task<AppMessage> {
        match event {
            Message::Cancel(ticket) if self.picker.as_ref().is_some_and(|p| p.ticket == ticket) => {
                self.picker = None;
                self.image_purpose = super::ImagePurpose::Desktop;
            }
            Message::Loaded(ticket, result) => {
                if self.picker_read != Some(ticket) {
                    return Task::none();
                }
                self.picker_read = None;
                let Some(picker) = &mut self.picker else {
                    return Task::none();
                };
                if picker.ticket != ticket {
                    return Task::none();
                }
                match result {
                    Ok(listing) => {
                        picker.folder = listing.folder;
                        picker.location = picker.folder.to_string_lossy().into_owned();
                        picker.truncated = listing.truncated;
                        picker.all.set_entries(listing.entries);
                        picker.project();
                    }
                    Err(error) => picker.error = Some(error),
                }
            }
            Message::Location(value) if self.picker_read.is_none() => {
                if let Some(picker) = &mut self.picker {
                    picker.location = value.chars().take(4096).collect();
                }
            }
            Message::Go(ticket) => {
                if let Some(picker) = &self.picker
                    && picker.ticket == ticket
                {
                    return self.load_picker_folder(PathBuf::from(picker.location.trim()));
                }
            }
            Message::Up(ticket) => {
                if let Some(picker) = &self.picker
                    && picker.ticket == ticket
                    && let Some(path) = picker.folder.parent().map(Path::to_path_buf)
                {
                    return self.load_picker_folder(path);
                }
            }
            Message::Filter(value) if self.picker_read.is_none() => {
                let Some(ticket) = self.picker_ticket() else {
                    return Task::none();
                };
                if let Some(picker) = &mut self.picker {
                    picker.ticket = ticket;
                    picker
                        .all
                        .set_query(value.chars().take(128).collect::<String>());
                    picker.project();
                }
            }
            Message::Select(ticket, path) => {
                if let Some(picker) = &mut self.picker
                    && picker.ticket == ticket
                    && self.picker_read.is_none()
                    && picker.shown.entry_at(&path).is_some()
                {
                    picker.selected = Some(path);
                }
            }
            Message::Activate(ticket, path) => return self.activate_picker_path(ticket, path),
            Message::Accept(ticket, Some(path)) => return self.activate_picker_path(ticket, path),
            Message::Sort(ticket, key)
                if self.picker_read.is_none()
                    && self.picker.as_ref().is_some_and(|p| p.ticket == ticket) =>
            {
                let Some(next) = self.picker_ticket() else {
                    return Task::none();
                };
                if let Some(picker) = &mut self.picker {
                    picker.ticket = next;
                    picker.all.toggle_sort(key);
                    picker.project();
                }
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn wallpaper_picker_view(&self) -> Element<'_, AppMessage> {
        let Some(picker) = &self.picker else {
            return iced::widget::space().into();
        };
        let ticket = picker.ticket;
        let enabled = self.picker_read.is_none() && !self.closing;
        let folder_selected = picker
            .selected
            .as_ref()
            .and_then(|path| picker.shown.entry_at(path))
            .is_some_and(|entry| entry.kind == EntryKind::Directory);
        let browser = enz_file_browser(&picker.shown)
            .theme(&picker.theme)
            .columns(Vec::new())
            .row_height(36.0)
            .selected(picker.selected.iter().cloned().collect::<Vec<_>>())
            .id_prefix("wallpaper-files")
            .empty_title(if self.picker_read.is_some() {
                tr("shell-reading-folder")
            } else {
                tr("shell-no-images-or-folders")
            })
            .empty_description(tr(
                "shell-png-jpeg-and-webp-up-to-32-mib-links-and-hidden-entries-are-excluded",
            ))
            .on_select(move |path| message(Message::Select(ticket, path)))
            .on_activate(move |path| message(Message::Activate(ticket, path)))
            .on_sort(move |key| message(Message::Sort(ticket, key)))
            .into_element();
        let mut body = column![
            row![
                enz_text(match self.image_purpose {
                    super::ImagePurpose::Desktop => tr("shell-choose-wallpaper-image"),
                    super::ImagePurpose::Lock => tr("shell-choose-login-background"),
                    super::ImagePurpose::Avatar(_) => tr("shell-choose-profile-picture"),
                })
                .size(TextSize::Title3)
                .into_element(),
                iced::widget::space().width(Length::Fill),
                enz_button(
                    enzui_shell::i18n::tr("shell-cancel"),
                    message(Message::Cancel(ticket))
                )
                .kind(ButtonKind::Quiet)
                .into_element()
            ]
            .spacing(8),
            row![
                enz_button(
                    enzui_shell::i18n::tr("shell-up"),
                    message(Message::Up(ticket))
                )
                .kind(ButtonKind::Quiet)
                .icon("arrow-up")
                .enabled(enabled && picker.folder.parent().is_some())
                .into_element(),
                container(
                    enz_text_field(
                        "",
                        enzui_shell::i18n::current().text("shell-absolute-folder-path"),
                        &picker.location,
                        |value| { message(Message::Location(value)) }
                    )
                    .id("wallpaper-folder".into())
                    .enabled(enabled)
                    .into_element()
                )
                .width(Length::Fill),
                enz_button(
                    enzui_shell::i18n::tr("shell-go"),
                    message(Message::Go(ticket))
                )
                .kind(ButtonKind::Secondary)
                .enabled(enabled)
                .into_element()
            ]
            .spacing(6),
            enz_search_field(picker.all.query(), |value| message(Message::Filter(value)))
                .placeholder(tr("shell-filter-image-names"))
                .id("wallpaper-file-filter")
                .enabled(enabled)
                .into_element(),
        ]
        .spacing(10);
        if let Some(error) = &picker.error {
            body = body.push(enz_text(error).into_element());
        }
        if picker.truncated || picker.more_matches {
            body = body.push(
                enz_text(tr(
                    "shell-listing-is-limited-filter-image-names-or-choose-a-smaller-folder",
                ))
                .size(TextSize::Caption)
                .into_element(),
            );
        }
        body = body
            .push(keyed_column(vec![(ticket.to_string(), browser)]).height(Length::Fill))
            .push(keyed_column(vec![(
                format!("{ticket}:{:?}", picker.selected),
                enz_button(
                    if folder_selected {
                        tr("shell-open-folder")
                    } else {
                        tr("shell-use-image")
                    },
                    message(Message::Accept(ticket, picker.selected.clone())),
                )
                .icon(if folder_selected {
                    "folder-open"
                } else {
                    "check"
                })
                .enabled(enabled && picker.selected.is_some())
                .into_element(),
            )]));
        container(body)
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
