//! XDG Desktop Portal file chooser; closing a request window cancels that request.

use enzui_core::file_browser::{FileBrowserModel, FileEntry, SortKey};
use enzui_icons::file_type::FileTypeTable;
use enzui_shell::chooser_places::{self as places, Place};
use enzui_shell::desktop_portal::{self, AccessRequest, Event, Kind, Outcome, Request};
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    breadcrumb::{BreadcrumbEditEvent, BreadcrumbTokens, edit_focus_operation, enz_breadcrumb},
    button::{ButtonKind, enz_button},
    file_browser::{DEFAULT_COLUMNS, FileBrowserView, FileColumn, enz_file_browser},
    search_field::enz_search_field,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
    toolbar_icon::enz_toolbar_icon,
};
use iced::widget::{Space, column, container, row, scrollable};
use iced::{Alignment, Element, Length, Subscription, Task, window};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

// An event from the portal, handed over once (its reply channel is not
// clonable).
#[derive(Clone, Debug)]
struct Incoming(Arc<Mutex<Option<Event>>>);

#[derive(Clone, Debug)]
enum Message {
    Portal(Incoming),
    Go(window::Id, PathBuf),
    Up(window::Id),
    Activate(window::Id, PathBuf),
    Select(window::Id, PathBuf),
    Sort(window::Id, SortKey),
    Name(window::Id, String),
    Filter(window::Id, Option<usize>),
    Hidden(window::Id),
    Accept(window::Id),
    Replace(window::Id),
    Cancel(window::Id),
    Closed(window::Id),
    Key(window::Id, iced::keyboard::Key, iced::keyboard::Modifiers),
    Grant(window::Id, bool),
    Query(window::Id, String),
    PathEdit(window::Id, BreadcrumbEditEvent),
    Crumb(window::Id, usize),
    NewFolder(window::Id, Option<String>),
    CreateFolder(window::Id),
    Recent(window::Id),
    Grid(window::Id),
}

const PATH_BAR: &str = "chooser-path";
const SEARCH: &str = "chooser-search";
const NEW_FOLDER: &str = "chooser-new-folder";
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

struct Dialog {
    request: Request,
    reply: Option<oneshot::Sender<Outcome>>,
    folder: PathBuf,
    model: FileBrowserModel,
    selected: Vec<PathBuf>,
    name: String,
    filter: Option<usize>,
    notice: Option<String>,
    // Save: the files that already exist and would be replaced.
    replacing: Vec<PathBuf>,
    query: String,
    // The location being typed in the path bar.
    editing: Option<String>,
    new_folder: Option<String>,
    // Showing recently used files instead of a folder.
    recent: bool,
    grid: bool,
    standard: Vec<Place>,
    pinned: Vec<Place>,
    devices: Vec<Place>,
}

struct Consent {
    request: AccessRequest,
    reply: Option<oneshot::Sender<bool>>,
}

struct App {
    dialogs: HashMap<window::Id, Dialog>,
    consents: HashMap<window::Id, Consent>,
    // Requests the portal closed before their window was shown.
    closed: std::collections::HashSet<u64>,
    light: bool,
}

fn home() -> PathBuf {
    places::home()
}

fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => {
            tr!("shell-gb-3", "arg0" => enzui_shell::i18n::number(format!("{:.1}", b as f64 / f64::from(1u32 << 30))))
        }
        b if b >= 1 << 20 => {
            tr!("shell-mb-5", "arg0" => enzui_shell::i18n::number(format!("{:.1}", b as f64 / f64::from(1u32 << 20))))
        }
        b if b >= 1 << 10 => {
            tr!("shell-kb-3", "arg0" => format!("{:}", b.div_ceil(1024)))
        }
        b => tr!("shell-bytes-2", "arg0" => format!("{:}", b)),
    }
}

impl Dialog {
    fn new(request: Request, reply: oneshot::Sender<Outcome>) -> Self {
        let folder = request
            .current_file
            .as_ref()
            .and_then(|f| f.parent().map(Path::to_path_buf))
            .or_else(|| request.current_folder.clone())
            .filter(|f| f.is_dir())
            .unwrap_or_else(home);
        let name = request
            .current_file
            .as_ref()
            .and_then(|f| f.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| request.current_name.clone())
            .unwrap_or_default();
        let filter = request
            .current_filter
            .or_else(|| (!request.filters.is_empty()).then_some(0));
        let standard = places::user_folders();
        let pinned = places::pinned(&standard);
        let mut dialog = Self {
            request,
            reply: Some(reply),
            folder,
            model: FileBrowserModel::new(),
            selected: Vec::new(),
            name,
            filter,
            notice: None,
            replacing: Vec::new(),
            query: String::new(),
            editing: None,
            new_folder: None,
            recent: false,
            grid: false,
            standard,
            pinned,
            devices: places::devices(),
        };
        dialog.read();
        dialog
    }

    fn title(&self) -> String {
        let request = &self.request;
        if !request.title.is_empty() {
            return request.title.clone();
        }
        match request.kind {
            Kind::Open if request.directory => tr("shell-choose-a-folder"),
            Kind::Open if request.multiple => tr("shell-open-files"),
            Kind::Open => tr("shell-open-a-file"),
            Kind::Save => tr("shell-save-as"),
            Kind::SaveFiles => tr("shell-choose-where-to-save"),
        }
        .to_owned()
    }

    fn picks_folders(&self) -> bool {
        self.request.directory || self.request.kind == Kind::SaveFiles
    }

    fn read(&mut self) {
        let table = FileTypeTable::host();
        let filter = self
            .filter
            .and_then(|i| self.request.filters.get(i))
            .cloned();
        let folders_only = self.picks_folders();
        let mut entries = Vec::new();
        self.notice = None;
        let listing: std::io::Result<Vec<(PathBuf, String)>> = if self.recent {
            Ok(places::recent()
                .into_iter()
                .filter_map(|path| {
                    Some((
                        path.clone(),
                        path.file_name()?.to_string_lossy().into_owned(),
                    ))
                })
                .collect())
        } else {
            std::fs::read_dir(&self.folder).map(|listing| {
                listing
                    .flatten()
                    .take(20_000)
                    .map(|item| (item.path(), item.file_name().to_string_lossy().into_owned()))
                    .collect()
            })
        };
        match listing {
            Ok(listing) => {
                for (path, name) in listing {
                    let Ok(metadata) = std::fs::metadata(&path) else {
                        continue;
                    };
                    let mut entry = if metadata.is_dir() {
                        FileEntry::directory(&path, name.clone())
                    } else {
                        FileEntry::file(&path, name.clone(), metadata.len())
                    };
                    if let Ok(modified) = metadata.modified() {
                        entry = entry.with_modified(modified);
                    }
                    entry.classify(table);
                    if !metadata.is_dir()
                        && (folders_only
                            || filter
                                .as_ref()
                                .is_some_and(|f| !f.accepts(&name, entry.mime.as_deref())))
                    {
                        continue;
                    }
                    entries.push(entry);
                }
            }
            Err(error) => {
                self.notice = Some(
                    tr!("shell-this-folder-can-t-be-opened", "arg0" => format!("{:}", error.kind())),
                );
            }
        }
        self.model.set_entries(entries);
        self.selected.clear();
        self.replacing.clear();
    }

    fn go(&mut self, folder: PathBuf) {
        if folder.is_dir() {
            self.folder = folder;
            self.recent = false;
            self.query.clear();
            self.model.set_query("");
            self.new_folder = None;
            self.read();
        }
    }

    fn go_typed(&mut self, typed: &str) -> bool {
        let Some(path) = places::resolve(typed, &self.folder, &home()) else {
            return false;
        };
        if path.is_dir() {
            self.go(path);
            return false;
        }
        let (Some(parent), Some(name)) = (path.parent().filter(|p| p.is_dir()), path.file_name())
        else {
            self.notice =
                Some(tr!("shell-doesn-t-exist", "arg0" => format!("{:}", path.display())));
            return false;
        };
        let name = name.to_string_lossy().into_owned();
        self.go(parent.to_path_buf());
        match self.request.kind {
            Kind::Save => {
                self.name = name;
                false
            }
            Kind::Open if path.is_file() && !self.picks_folders() => {
                self.selected = vec![path];
                true
            }
            _ => {
                self.notice = Some(tr!("shell-isn-t-here", "arg0" => name));
                false
            }
        }
    }

    fn create_folder(&mut self) {
        let Some(name) = self.new_folder.clone() else {
            return;
        };
        if let Some(problem) = places::folder_name_problem(&name, &self.folder) {
            self.notice = Some(problem.into());
            return;
        }
        let path = self.folder.join(name.trim());
        match std::fs::create_dir(&path) {
            Ok(()) => {
                // The new folder is where things go next.
                self.go(path);
            }
            Err(error) => {
                self.notice = Some(
                    tr!("shell-the-folder-could-not-be-made", "arg0" => format!("{:}", error.kind())),
                );
            }
        }
    }

    fn answer(&mut self, outcome: Outcome) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(outcome);
        }
    }

    fn choice(&self) -> Result<Vec<PathBuf>, String> {
        match self.request.kind {
            Kind::Open if self.request.directory => Ok(vec![
                self.selected
                    .first()
                    .cloned()
                    .unwrap_or_else(|| self.folder.clone()),
            ]),
            Kind::Open => {
                let files: Vec<PathBuf> = self
                    .selected
                    .iter()
                    .filter(|p| !p.is_dir())
                    .cloned()
                    .collect();
                if files.is_empty() {
                    Err(tr("shell-choose-a-file-first").into())
                } else {
                    Ok(files)
                }
            }
            Kind::Save => {
                let name = self.name.trim();
                if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                    return Err(tr("shell-type-a-file-name-without-slashes").into());
                }
                writable(&self.folder, &[self.folder.join(name)])
            }
            Kind::SaveFiles => writable(
                &self.folder,
                &self
                    .request
                    .files
                    .iter()
                    .map(|name| self.folder.join(name))
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

// Reject unwritable save destinations before returning them to the requesting application.
fn writable(folder: &Path, targets: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    use rustix::fs::{Access, access};
    if access(folder, Access::WRITE_OK).is_err() {
        return Err(tr("shell-you-can-t-save-in-this-folder-choose-another-one").into());
    }
    if let Some(locked) = targets
        .iter()
        .find(|t| t.exists() && access(t.as_path(), Access::WRITE_OK).is_err())
    {
        return Err(
            tr!("shell-is-read-only", "arg0" => locked.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        );
    }
    Ok(targets.to_vec())
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let light = enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light);
        (
            Self {
                dialogs: HashMap::new(),
                consents: HashMap::new(),
                closed: Default::default(),
                light,
            },
            Task::none(),
        )
    }
    fn tokens(&self) -> EnzTheme {
        if self.light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        }
    }
    fn finish(&mut self, window: window::Id, outcome: Outcome) -> Task<Message> {
        if let Some(mut dialog) = self.dialogs.remove(&window) {
            dialog.answer(outcome);
        }
        // A consent window that goes away without an answer refuses.
        if let Some(reply) = self.consents.remove(&window).and_then(|c| c.reply) {
            let _ = reply.send(false);
        }
        window::close(window)
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Portal(Incoming(event)) => {
                match event.lock().ok().and_then(|mut e| e.take()) {
                    Some(Event::Appearance { light }) => self.light = light,
                    Some(Event::Begin(request, reply)) => {
                        if self.closed.remove(&request.id) {
                            let _ = reply.send(Outcome::Cancelled);
                            return Task::none();
                        }
                        let (window, open) = window::open(window::Settings {
                            size: iced::Size::new(980.0, 640.0),
                            min_size: Some(iced::Size::new(420.0, 320.0)),
                            exit_on_close_request: false,
                            platform_specific: window::settings::PlatformSpecific {
                                application_id: "dev.enzui.FileChooser".into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        });
                        self.dialogs.insert(window, Dialog::new(request, reply));
                        return open.discard();
                    }
                    Some(Event::Access(request, reply)) => {
                        if self.closed.remove(&request.id) {
                            let _ = reply.send(false);
                            return Task::none();
                        }
                        let (window, open) = window::open(window::Settings {
                            size: iced::Size::new(460.0, 280.0),
                            resizable: false,
                            exit_on_close_request: false,
                            platform_specific: window::settings::PlatformSpecific {
                                application_id: "dev.enzui.PortalAccess".into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        });
                        self.consents.insert(
                            window,
                            Consent {
                                request,
                                reply: Some(reply),
                            },
                        );
                        return open.discard();
                    }
                    Some(Event::Close(id)) => {
                        let window = self
                            .dialogs
                            .iter()
                            .find(|(_, d)| d.request.id == id)
                            .map(|(w, _)| *w)
                            .or_else(|| {
                                self.consents
                                    .iter()
                                    .find(|(_, c)| c.request.id == id)
                                    .map(|(w, _)| *w)
                            });
                        match window {
                            Some(window) => return self.finish(window, Outcome::Cancelled),
                            // The request may still be on its way to this window.
                            None if self.closed.len() < 256 => {
                                self.closed.insert(id);
                            }
                            None => {}
                        }
                    }
                    None => {}
                }
            }
            Message::Grant(window, grant) => {
                if let Some(reply) = self.consents.get_mut(&window).and_then(|c| c.reply.take()) {
                    let _ = reply.send(grant);
                }
                return self.finish(window, Outcome::Cancelled);
            }
            // Cancelling the replace question returns to the chooser.
            Message::Cancel(window)
                if self
                    .dialogs
                    .get(&window)
                    .is_some_and(|d| !d.replacing.is_empty()) =>
            {
                if let Some(dialog) = self.dialogs.get_mut(&window) {
                    dialog.replacing.clear();
                }
            }
            Message::Closed(window) | Message::Cancel(window) => {
                return self.finish(window, Outcome::Cancelled);
            }
            Message::Key(window, key, modifiers) if self.dialogs.contains_key(&window) => {
                use iced::keyboard::{Key, key::Named};
                let dialog = &self.dialogs[&window];
                match key.as_ref() {
                    Key::Named(Named::Escape) if dialog.editing.is_some() => {
                        return self.update(Message::PathEdit(window, BreadcrumbEditEvent::Cancel));
                    }
                    Key::Named(Named::Escape) if dialog.new_folder.is_some() => {
                        return self.update(Message::NewFolder(window, None));
                    }
                    Key::Named(Named::Escape) => return self.finish(window, Outcome::Cancelled),
                    // Enter never grants a permission; it only accepts a file choice.
                    Key::Named(Named::Enter) => return self.update(Message::Accept(window)),
                    Key::Character("l") if modifiers.command() => {
                        return self.update(Message::PathEdit(window, BreadcrumbEditEvent::Begin));
                    }
                    Key::Character("f") if modifiers.command() => {
                        return iced::widget::operation::focus(iced::widget::Id::new(SEARCH));
                    }
                    Key::Character("n" | "N") if modifiers.command() && modifiers.shift() => {
                        return self.update(Message::NewFolder(window, Some(String::new())));
                    }
                    _ => {}
                }
            }
            Message::Key(
                window,
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                _,
            ) => return self.finish(window, Outcome::Cancelled),
            Message::Key(..) => {}
            Message::Accept(window) => {
                let Some(dialog) = self.dialogs.get_mut(&window) else {
                    return Task::none();
                };
                match dialog.choice() {
                    Err(notice) => dialog.notice = Some(notice),
                    Ok(paths) => {
                        // Typing a folder's name and pressing Save opens it.
                        if dialog.request.kind == Kind::Save && paths[0].is_dir() {
                            let folder = paths[0].clone();
                            dialog.name.clear();
                            dialog.go(folder);
                            return Task::none();
                        }
                        let existing: Vec<PathBuf> =
                            paths.iter().filter(|p| p.exists()).cloned().collect();
                        if dialog.request.kind != Kind::Open
                            && !existing.is_empty()
                            && dialog.replacing != existing
                        {
                            dialog.replacing = existing;
                            return Task::none();
                        }
                        let filter = dialog.filter;
                        return self.finish(window, Outcome::Chosen { paths, filter });
                    }
                }
            }
            Message::Replace(window) => {
                if let Some(dialog) = self.dialogs.get(&window)
                    && let Ok(paths) = dialog.choice()
                {
                    let filter = dialog.filter;
                    return self.finish(window, Outcome::Chosen { paths, filter });
                }
            }
            other => {
                let window = match &other {
                    Message::Go(w, _)
                    | Message::Up(w)
                    | Message::Activate(w, _)
                    | Message::Select(w, _)
                    | Message::Sort(w, _)
                    | Message::Name(w, _)
                    | Message::Filter(w, _)
                    | Message::Hidden(w)
                    | Message::Query(w, _)
                    | Message::PathEdit(w, _)
                    | Message::Crumb(w, _)
                    | Message::NewFolder(w, _)
                    | Message::CreateFolder(w)
                    | Message::Recent(w)
                    | Message::Grid(w) => *w,
                    _ => return Task::none(),
                };
                let Some(dialog) = self.dialogs.get_mut(&window) else {
                    return Task::none();
                };
                match other {
                    Message::Go(_, folder) => dialog.go(folder),
                    Message::Up(_) => {
                        if let Some(parent) = dialog.folder.parent().map(Path::to_path_buf) {
                            dialog.go(parent);
                        }
                    }
                    Message::Activate(_, path) if path.is_dir() => dialog.go(path),
                    Message::Activate(_, path) => {
                        dialog.selected = vec![path.clone()];
                        if dialog.request.kind == Kind::Save {
                            dialog.name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                        }
                        return self.update(Message::Accept(window));
                    }
                    Message::Select(_, path) => {
                        dialog.replacing.clear();
                        dialog.notice = None;
                        if dialog.request.multiple && !path.is_dir() {
                            if let Some(at) = dialog.selected.iter().position(|p| *p == path) {
                                dialog.selected.remove(at);
                            } else {
                                dialog.selected.push(path);
                            }
                        } else {
                            if dialog.request.kind == Kind::Save && !path.is_dir() {
                                dialog.name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                            }
                            dialog.selected = vec![path];
                        }
                    }
                    Message::Sort(_, key) => dialog.model.toggle_sort(key),
                    Message::Name(_, name) => {
                        dialog.name = name.chars().filter(|c| *c != '/').take(255).collect();
                        dialog.replacing.clear();
                        dialog.notice = None;
                    }
                    Message::Filter(_, filter) => {
                        dialog.filter = filter;
                        dialog.read();
                    }
                    Message::Hidden(_) => {
                        let show = !dialog.model.show_hidden();
                        dialog.model.set_show_hidden(show);
                    }
                    Message::Query(_, query) => {
                        dialog.query = query.chars().take(200).collect();
                        dialog.model.set_query(dialog.query.clone());
                        // A choice the search hides is not a choice.
                        let visible: Vec<PathBuf> = dialog
                            .model
                            .visible()
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect();
                        dialog.selected.retain(|path| visible.contains(path));
                    }
                    Message::Crumb(_, index) => {
                        let (_, paths) = places::trail(&dialog.folder, &home());
                        if let Some(path) = paths.get(index).cloned() {
                            dialog.go(path);
                        }
                    }
                    Message::PathEdit(_, event) => {
                        let focus = edit_focus_operation::<Message>(
                            &event,
                            iced::advanced::widget::Id::new(PATH_BAR),
                        )
                        .map_or_else(Task::none, iced::advanced::widget::operate);
                        match event {
                            BreadcrumbEditEvent::Begin => {
                                // The folder, ready for a name typed after it.
                                let folder = dialog.folder.display().to_string();
                                dialog.editing = Some(if dialog.recent {
                                    "~/".into()
                                } else if folder.ends_with('/') {
                                    folder
                                } else {
                                    folder + "/"
                                });
                            }
                            BreadcrumbEditEvent::Changed(text) => {
                                dialog.editing = Some(text.chars().take(4096).collect());
                            }
                            BreadcrumbEditEvent::Cancel => dialog.editing = None,
                            BreadcrumbEditEvent::Commit => {
                                let typed = dialog.editing.take().unwrap_or_default();
                                if dialog.go_typed(&typed) {
                                    return Task::batch([
                                        focus,
                                        self.update(Message::Accept(window)),
                                    ]);
                                }
                            }
                        }
                        return focus;
                    }
                    Message::NewFolder(_, name) => {
                        let opening = dialog.new_folder.is_none() && name.is_some();
                        dialog.new_folder =
                            name.map(|n| n.chars().filter(|c| *c != '/').take(255).collect());
                        dialog.notice = None;
                        if opening {
                            return iced::widget::operation::focus(iced::widget::Id::new(
                                NEW_FOLDER,
                            ));
                        }
                    }
                    Message::CreateFolder(_) => dialog.create_folder(),
                    Message::Recent(_) => {
                        dialog.recent = true;
                        dialog.query.clear();
                        dialog.model.set_query("");
                        dialog.read();
                    }
                    Message::Grid(_) => dialog.grid = !dialog.grid,
                    _ => {}
                }
            }
        }
        Task::none()
    }

    fn consent_view<'a>(
        &'a self,
        window: window::Id,
        consent: &'a Consent,
    ) -> Element<'a, Message> {
        let palette = self.tokens().palette;
        let request = &consent.request;
        let mut content = column![
            row![
                enzui_widgets::icon::enz_icon(CONSENT_ICON)
                    .size(22.0)
                    .into_element(),
                enz_text(if request.title.is_empty() {
                    format!("Allow {}?", request.app_id)
                } else {
                    request.title.clone()
                })
                .size(TextSize::Title3)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(10);
        for line in [&request.subtitle, &request.body] {
            if !line.is_empty() {
                content = content.push(
                    enz_text(line.clone())
                        .muted_on(palette.surface)
                        .into_element(),
                );
            }
        }
        if !request.app_id.is_empty() {
            content = content.push(
                enz_text(tr!("shell-app-2", "arg0" => request.app_id.clone()))
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .into_element(),
            );
        }
        content = content.push(Space::new().height(Length::Fill)).push(
            row![
                Space::new().width(Length::Fill),
                enz_button(request.deny_label.clone(), Message::Grant(window, false))
                    .kind(ButtonKind::Secondary)
                    .id("access-deny".into())
                    .into_element(),
                enz_button(request.grant_label.clone(), Message::Grant(window, true))
                    .id("access-grant".into())
                    .into_element(),
            ]
            .spacing(8),
        );
        container(content)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(palette.surface.into()),
                ..Default::default()
            })
            .into()
    }

    fn place_row<'a>(
        &self,
        label: String,
        icon: &'static str,
        message: Message,
        current: bool,
    ) -> Element<'a, Message> {
        let content = row![
            enzui_widgets::icon::enz_icon(icon)
                .size(16.0)
                .into_element(),
            enz_text(label.clone()).single_line_elided().into_element()
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        enzui_widgets::list_row_surface::enz_list_row_surface(content, message)
            .selectable(true)
            .selected(current)
            .label(label)
            .width(Length::Fill)
            .into_element()
    }

    fn sidebar<'a>(&self, window: window::Id, dialog: &'a Dialog) -> Element<'a, Message> {
        let palette = self.tokens().palette;
        let heading = |text: &'static str| -> Element<'a, Message> {
            container(
                enz_text(text)
                    .size(TextSize::Footnote)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .muted_on(palette.surface_alt)
                    .into_element(),
            )
            .padding(iced::Padding {
                top: 12.0,
                right: 10.0,
                bottom: 4.0,
                left: 10.0,
            })
            .into()
        };
        let mut sidebar = column![].spacing(2);
        // Recent files only make sense when a file is being opened.
        if dialog.request.kind == Kind::Open && !dialog.picks_folders() {
            sidebar = sidebar.push(self.place_row(
                tr("shell-recent").into(),
                RECENT_ICON,
                Message::Recent(window),
                dialog.recent,
            ));
        }
        for place in &dialog.standard {
            sidebar = sidebar.push(self.place_row(
                place.label.clone(),
                place.icon,
                Message::Go(window, place.path.clone()),
                !dialog.recent && dialog.folder == place.path,
            ));
        }
        if !dialog.pinned.is_empty() {
            sidebar = sidebar.push(heading(tr("shell-pinned")));
            for place in &dialog.pinned {
                sidebar = sidebar.push(self.place_row(
                    place.label.clone(),
                    place.icon,
                    Message::Go(window, place.path.clone()),
                    !dialog.recent && dialog.folder == place.path,
                ));
            }
        }
        sidebar = sidebar.push(heading(tr("shell-devices")));
        for place in &dialog.devices {
            sidebar = sidebar.push(self.place_row(
                place.label.clone(),
                place.icon,
                Message::Go(window, place.path.clone()),
                !dialog.recent && dialog.folder == place.path,
            ));
        }
        container(scrollable(container(sidebar).padding(8)).height(Length::Fill))
            .width(210)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(palette.surface_alt.into()),
                ..Default::default()
            })
            .into()
    }

    fn toolbar<'a>(&self, window: window::Id, dialog: &'a Dialog) -> Element<'a, Message> {
        let palette = self.tokens().palette;
        let home = home();
        let path: Element<'a, Message> = if dialog.recent && dialog.editing.is_none() {
            container(
                enz_text(tr("shell-recent-files"))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding([0, 8])
            .width(Length::Fill)
            .into()
        } else {
            let (labels, _) = places::trail(&dialog.folder, &home);
            enz_breadcrumb(labels, move |index| Message::Crumb(window, index))
                .id(PATH_BAR)
                .separator("›")
                .max_visible(4)
                .tokens(BreadcrumbTokens::from_palette(&palette))
                .editable(
                    dialog.editing.is_some(),
                    dialog.editing.clone().unwrap_or_default(),
                    move |event| Message::PathEdit(window, event),
                )
                .into_element()
        };
        let can_make_folders = !dialog.recent;
        let bar = row![
            enz_toolbar_icon("arrow-up", Message::Up(window))
                .aria_label(tr("shell-enclosing-folder"))
                .tooltip(tr("shell-enclosing-folder"))
                .enabled(!dialog.recent && dialog.folder.parent().is_some())
                .into_element(),
            container(path).width(Length::Fill),
            container(
                enz_search_field(&dialog.query, move |query| Message::Query(window, query))
                    .placeholder(if dialog.recent {
                        tr("shell-search-recent-files")
                    } else {
                        tr("shell-search-this-folder")
                    })
                    .on_clear(Message::Query(window, String::new()))
                    .id(iced::widget::Id::new(SEARCH))
                    .into_element(),
            )
            .width(Length::Fixed(220.0)),
            enz_toolbar_icon(
                "folder-plus",
                Message::NewFolder(window, Some(String::new()))
            )
            .aria_label(tr("shell-new-folder"))
            .tooltip(tr("shell-new-folder-ctrl-shift-n"))
            .enabled(can_make_folders)
            .into_element(),
            enz_toolbar_icon(
                if dialog.grid { "list" } else { "layout-grid" },
                Message::Grid(window)
            )
            .aria_label(if dialog.grid {
                tr("shell-show-as-a-list")
            } else {
                tr("shell-show-as-icons")
            })
            .tooltip(if dialog.grid {
                tr("shell-show-as-a-list")
            } else {
                tr("shell-show-as-icons")
            })
            .into_element(),
            enz_toolbar_icon(
                if dialog.model.show_hidden() {
                    "eye-off"
                } else {
                    "eye"
                },
                Message::Hidden(window)
            )
            .aria_label(if dialog.model.show_hidden() {
                tr("shell-hide-hidden-files")
            } else {
                tr("shell-show-hidden-files")
            })
            .tooltip(if dialog.model.show_hidden() {
                tr("shell-hide-hidden-files")
            } else {
                tr("shell-show-hidden-files")
            })
            .into_element(),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        // The path bar fills what it is given; the list gets the height.
        container(bar)
            .height(Length::Fixed(40.0))
            .align_y(Alignment::Center)
            .into()
    }

    fn preview<'a>(&self, dialog: &'a Dialog) -> Option<Element<'a, Message>> {
        let [path] = dialog.selected.as_slice() else {
            return None;
        };
        let entry = dialog.model.entry_at(path)?;
        let palette = self.tokens().palette;
        let image = !entry.is_navigable()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            && entry.size <= 64 << 20;
        let picture: Element<'a, Message> = if image {
            iced::widget::image(iced::widget::image::Handle::from_path(path))
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fixed(180.0))
                .into()
        } else {
            container(
                enzui_widgets::icon::enz_icon(entry.icon_name())
                    .size(72.0)
                    .into_element(),
            )
            .center_x(Length::Fill)
            .padding([24, 0])
            .into()
        };
        let mut details = column![
            picture,
            enz_text(entry.name.clone())
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .wrap(true)
                .into_element()
        ]
        .spacing(10);
        let mut facts: Vec<String> = Vec::new();
        if let Some(mime) = &entry.mime {
            facts.push(mime.clone());
        }
        if !entry.is_navigable() {
            facts.push(size_text(entry.size));
        }
        if let Some(modified) = entry.modified {
            facts.push(tr!("shell-modified", "arg0" => format!("{:}", chrono::DateTime::<chrono::Local>::from(modified).format("%-d %b %Y, %H:%M"))));
        }
        if dialog.recent
            && let Some(parent) = path.parent()
        {
            facts.push(tr!("shell-in", "arg0" => format!("{:}", parent.display())));
        }
        for fact in facts {
            details = details.push(
                enz_text(fact)
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .wrap(true)
                    .into_element(),
            );
        }
        Some(
            container(details)
                .width(Length::Fixed(240.0))
                .padding(iced::Padding {
                    top: 4.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 12.0,
                })
                .into(),
        )
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
        if let Some(consent) = self.consents.get(&window) {
            return self.consent_view(window, consent);
        }
        let Some(dialog) = self.dialogs.get(&window) else { return Space::new().into() };
        let palette = self.tokens().palette;
        let request = &dialog.request;
        let accept = request.accept_label.clone().unwrap_or_else(|| {
            match request.kind {
                Kind::Open if request.directory => tr("shell-choose"),
                Kind::Open => tr("shell-open"),
                Kind::Save | Kind::SaveFiles => tr("shell-save"),
            }
            .to_owned()
        });

        // Metadata columns give way first, then the preview, so names stay
        // readable in a chooser on a small or scaled display.
        let files = iced::widget::responsive(move |size| {
            let preview = if size.width >= 720.0 { self.preview(dialog) } else { None };
            let room = size.width - if preview.is_some() { 252.0 } else { 0.0 };
            let columns = if room >= 560.0 {
                DEFAULT_COLUMNS
            } else if room >= 360.0 {
                &[FileColumn::Modified][..]
            } else {
                &[]
            };
            let empty = if dialog.recent {
                tr("shell-no-recently-used-files")
            } else if !dialog.query.is_empty() {
                tr("shell-nothing-here-matches")
            } else if dialog.picks_folders() {
                tr("shell-no-folders-here")
            } else {
                tr("shell-no-matching-files-here")
            };
            let browser = enz_file_browser(&dialog.model)
                .view_mode(if dialog.grid { FileBrowserView::Grid } else { FileBrowserView::List })
                .columns(columns.to_vec())
                .selected(dialog.selected.clone())
                .on_activate(move |path| Message::Activate(window, path))
                .on_select(move |path| Message::Select(window, path))
                .on_sort(move |key| Message::Sort(window, key))
                .empty_title(empty)
                .into_element();
            let mut body = row![container(browser).width(Length::Fill).height(Length::Fill)];
            if let Some(preview) = preview {
                body = body.push(preview);
            }
            Element::from(body.height(Length::Fill))
        });

        let mut main = column![].spacing(10).width(Length::Fill).height(Length::Fill);
        if let Some(name) = &dialog.new_folder {
            main = main.push(
                row![
                    enzui_widgets::icon::enz_icon("folder-plus").size(18.0).into_element(),
                    enz_text_field("", enzui_shell::i18n::current().text("shell-new-folder-name"), name, move |name| Message::NewFolder(window, Some(name)))
                        .on_submit(Message::CreateFolder(window))
                        .id(NEW_FOLDER.into())
                        .into_element(),
                    enz_button(enzui_shell::i18n::tr("shell-create"), Message::CreateFolder(window)).id("chooser-create-folder".into()).enabled(places::folder_name_problem(name, &dialog.folder).is_none()).into_element(),
                    enz_toolbar_icon("x", Message::NewFolder(window, None)).aria_label(tr("shell-don-t-make-a-folder")).tooltip(tr("shell-don-t-make-a-folder")).into_element(),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        main = main.push(container(files).height(Length::Fill));

        // One footer row: what to save as and which files to show on the
        // left, the decision on the right.
        let mut footer = row![].spacing(12).align_y(Alignment::Center);
        if request.kind == Kind::Save {
            footer = footer.push(
                container(enz_text_field("", enzui_shell::i18n::current().text("shell-file-name"), &dialog.name, move |name| Message::Name(window, name)).on_submit(Message::Accept(window)).id("chooser-name".into()).into_element())
                    .width(Length::FillPortion(3)),
            );
        }
        if request.kind == Kind::SaveFiles && !request.files.is_empty() {
            footer = footer.push(enz_text(enzui_shell::i18n::count("portal-saves-files", request.files.len())).size(TextSize::Footnote).muted_on(palette.surface).into_element());
        }
        if !request.filters.is_empty() && !dialog.picks_folders() {
            let mut names: Vec<String> = request.filters.iter().map(|f| f.name.clone()).collect();
            // Apps often offer their own "All files"; one is enough.
            let count = request.filters.len();
            if !names.iter().any(|name| name.eq_ignore_ascii_case(enzui_shell::i18n::tr("shell-all-files"))) {
                names.push(tr("shell-all-files").to_owned());
            }
            let selected = Some(dialog.filter.filter(|i| *i < count).unwrap_or(count));
            footer = footer.push(
                enzui_widgets::combo_box::enz_combo_box("", names, None, move |name: String| {
                    let index = request.filters.iter().position(|f| f.name == name);
                    Message::Filter(window, index)
                })
                .selected_index(selected)
                .width(220)
                .id("chooser-file-type".into())
                .into_element(),
            );
        }
        footer = footer.push(Space::new().width(Length::Fill));
        if !dialog.replacing.is_empty() {
            let what = if dialog.replacing.len() == 1 {
                tr!("shell-replace-2", "arg0" => dialog.replacing[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            } else {
                tr!("shell-replace-existing-files", "arg0" => format!("{:}", dialog.replacing.len()))
            };
            footer = footer
                .push(enz_text(what).into_element())
                .push(enz_button(tr("shell-cancel"), Message::Cancel(window)).kind(ButtonKind::Secondary).into_element())
                .push(enz_button(tr("shell-replace"), Message::Replace(window)).id("chooser-replace".into()).into_element());
        } else {
            footer = footer
                .push(enz_button(tr("shell-cancel"), Message::Cancel(window)).kind(ButtonKind::Secondary).id("chooser-cancel".into()).into_element())
                .push(enz_button(accept, Message::Accept(window)).id("chooser-accept".into()).enabled(dialog.choice().is_ok()).into_element());
        }
        // Why Save is unavailable here; an open request simply waits for a pick.
        let problem = (request.kind != Kind::Open && !dialog.name.trim().is_empty() || request.kind == Kind::SaveFiles)
            .then(|| dialog.choice().err())
            .flatten();
        let mut bottom = column![].spacing(6);
        if let Some(notice) = dialog.notice.clone().or(problem) {
            bottom = bottom.push(enz_text(notice).size(TextSize::Footnote).into_element());
        }
        bottom = bottom.push(footer);

        // The window title names the request; the path leads the content.
        let content = column![self.toolbar(window, dialog), main, bottom].spacing(12).padding(iced::Padding { top: 14.0, right: 16.0, bottom: 14.0, left: 16.0 });
        container(row![self.sidebar(window, dialog), content.width(Length::Fill).height(Length::Fill)].height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(palette.surface.into()), ..Default::default() })
            .into()

        })
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(portal).map(Message::Portal),
            window::close_requests().map(Message::Closed),
            iced::event::listen_with(|event, status, window| match (event, status) {
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key, modifiers, ..
                    }),
                    iced::event::Status::Ignored,
                ) => Some(Message::Key(window, key, modifiers)),
                _ => None,
            }),
        ])
    }
}

const CONSENT_ICON: &str = "shield-question-mark";
const RECENT_ICON: &str = "clock";

fn portal() -> impl iced::futures::Stream<Item = Incoming> {
    iced::stream::channel(8, async move |mut output| {
        use iced::futures::SinkExt;
        let (events, mut incoming) = mpsc::unbounded_channel();
        let forward = async {
            while let Some(event) = incoming.recv().await {
                if output
                    .send(Incoming(Arc::new(Mutex::new(Some(event)))))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        };
        tokio::select! {
            result = desktop_portal::run(events) => {
                if let Err(error) = result {
                    eprintln!("enzui-portal: {error}");
                    std::process::exit(1);
                }
            }
            () = forward => {}
        }
    })
}

fn main() -> iced::Result {
    i18n::initialize();
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(App::new, App::update, App::view)
            .title(|app: &App, window| {
                match (app.dialogs.get(&window), app.consents.get(&window)) {
                    (Some(dialog), _) => dialog.title(),
                    (None, Some(consent)) => consent.request.title.clone(),
                    _ => tr("shell-files").to_owned(),
                }
            })
            .theme(|app: &App, _| app.tokens().iced_theme())
            .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
