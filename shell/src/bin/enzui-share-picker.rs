//! Screen-sharing picker started by the screen-cast portal.
//!
//! The reply protocol is described in `enzui_shell::share_picker`. Closing the
//! window, Escape or Cancel exits without a reply, which the portal treats as a
//! refusal.

use enzui_shell::arm_delay::ArmGate;
use enzui_shell::i18n::{self, tr};
use enzui_shell::share_picker::{self, Output, Region, Request, Selection, SharedWindow};
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    checkbox::enz_checkbox,
    icon::enz_icon,
    inline_view_switcher::{InlineSwitcherPage, enz_inline_view_switcher},
    list_row_surface::enz_list_row_surface,
    scroll_view::enz_scroll_view,
    stepper::enz_stepper_u32,
    text::{TextSize, enz_text},
};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Subscription, Task, window};
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::path::PathBuf;

const APP_ID: &str = "dev.enzui.SharePicker";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Screens,
    Windows,
    Region,
}

const TABS: [Tab; 3] = [Tab::Screens, Tab::Windows, Tab::Region];

#[derive(Clone, Copy, Debug)]
enum Field {
    X,
    Y,
    Width,
    Height,
}

#[derive(Clone, Debug)]
enum Message {
    Tab(usize),
    Screen(String),
    Window(u32),
    RegionOutput(String),
    RegionField(Field, u32),
    Draw,
    Drawn(Option<(String, Region)>),
    Remember(bool),
    AfterRestart(bool),
    Share,
    Cancel,
    Key(iced::keyboard::Key),
    Shown(window::Id),
    Focused(window::Id),
    Armed,
    Closed(window::Id),
}

struct App {
    request: Request,
    app_name: Option<String>,
    app_icon: Option<PathBuf>,
    outputs: Vec<Output>,
    window_names: HashMap<String, String>,
    window_icons: HashMap<String, Option<PathBuf>>,
    tab: Tab,
    screen: Option<String>,
    window: Option<u32>,
    region_output: Option<String>,
    region: Region,
    drawing: bool,
    remember: bool,
    after_restart: bool,
    gate: ArmGate,
    window_id: Option<window::Id>,
    light: bool,
    reply: Reply,
}

/// The original standard output, kept for the reply while fd 1 points at stderr
/// so that library output cannot corrupt the line the portal parses.
struct Reply(Option<std::fs::File>);

impl Reply {
    fn take_stdout() -> Self {
        let saved = std::io::stdout().as_fd().try_clone_to_owned().ok();
        if saved.is_some() && rustix::stdio::dup2_stdout(std::io::stderr()).is_err() {
            eprintln!("enzui-share-picker: standard output stays shared");
        }
        Self(saved.map(std::fs::File::from))
    }

    fn send(&mut self, line: &str) -> ! {
        let written = self.0.as_mut().is_some_and(|out| {
            out.write_all(line.as_bytes())
                .and_then(|()| out.flush())
                .is_ok()
        });
        std::process::exit(if written { 0 } else { 1 })
    }
}

fn slurp_available() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("slurp").is_file()))
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let request = Request::new(&args, |key| std::env::var(key).ok());
        let outputs = share_picker::outputs().unwrap_or_else(|error| {
            eprintln!("enzui-share-picker: outputs: {error}");
            Vec::new()
        });
        let app_name = (!request.app_id.is_empty()).then(|| {
            enzui_shell::search::catalogue::name_for_app_id(&request.app_id)
                .unwrap_or_else(|| request.app_id.clone())
        });
        let app_icon = (!request.app_id.is_empty())
            .then(|| enzui_shell::search::catalogue::icon_for_app_id(&request.app_id))
            .flatten();
        let window_names = share_picker::app_names(&request.windows);
        let window_icons = request
            .windows
            .iter()
            .map(|w| {
                (
                    w.class.clone(),
                    enzui_shell::search::catalogue::icon_for_app_id(&w.class),
                )
            })
            .collect();
        let light = enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light);
        let first = outputs.first().map(|o| (o.name.clone(), o.full_region()));
        let (id, open) = window::open(window::Settings {
            size: iced::Size::new(560.0, 600.0),
            resizable: false,
            decorations: false,
            exit_on_close_request: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: APP_ID.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let app = Self {
            remember: request.remember_by_default,
            screen: (outputs.len() == 1).then(|| outputs[0].name.clone()),
            region_output: first.as_ref().map(|(name, _)| name.clone()),
            region: first.map_or(
                Region {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                |(_, r)| r,
            ),
            request,
            app_name,
            app_icon,
            outputs,
            window_names,
            window_icons,
            tab: Tab::Screens,
            window: None,
            drawing: false,
            after_restart: false,
            gate: ArmGate::default(),
            window_id: Some(id),
            light,
            reply: Reply(None),
        };
        (app, open.map(Message::Shown))
    }

    fn tokens(&self) -> EnzTheme {
        if self.light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        }
    }

    fn output(&self, name: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.name == name)
    }

    fn selection(&self) -> Option<Selection> {
        match self.tab {
            Tab::Screens => self.screen.clone().map(Selection::Screen),
            Tab::Windows => self.window.map(Selection::Window),
            Tab::Region => {
                let name = self.region_output.clone()?;
                let region = self.output(&name)?.clamp(self.region);
                Some(Selection::Region(name, region))
            }
        }
    }

    fn can_share(&self) -> bool {
        self.gate.armed() && !self.drawing && self.selection().is_some()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tab(index) => self.tab = TABS.get(index).copied().unwrap_or(Tab::Screens),
            Message::Screen(name) => self.screen = Some(name),
            Message::Window(handle) => self.window = Some(handle),
            Message::RegionOutput(name) => {
                if let Some(output) = self.output(&name) {
                    self.region = output.full_region();
                    self.region_output = Some(name);
                }
            }
            Message::RegionField(field, value) => {
                let value = i32::try_from(value).unwrap_or(i32::MAX);
                match field {
                    Field::X => self.region.x = value,
                    Field::Y => self.region.y = value,
                    Field::Width => self.region.width = value,
                    Field::Height => self.region.height = value,
                }
                if let Some(output) = self.region_output.as_deref().and_then(|n| self.output(n)) {
                    self.region = output.clamp(self.region);
                }
            }
            Message::Draw => {
                self.drawing = true;
                let outputs = self.outputs.clone();
                return Task::perform(
                    async move {
                        let output = tokio::process::Command::new("slurp")
                            .args(["-f", "%o %x %y %w %h"])
                            .output()
                            .await
                            .ok()?;
                        share_picker::parse_slurp(
                            &String::from_utf8_lossy(&output.stdout),
                            &outputs,
                        )
                    },
                    Message::Drawn,
                );
            }
            Message::Drawn(drawn) => {
                self.drawing = false;
                if let Some((name, region)) = drawn {
                    self.region_output = Some(name);
                    self.region = region;
                }
            }
            Message::Remember(checked) => self.remember = checked,
            Message::AfterRestart(checked) => {
                self.after_restart = checked;
                if checked {
                    self.remember = true;
                }
            }
            Message::Share => {
                if self.can_share()
                    && let Some(selection) = self.selection()
                {
                    let line = share_picker::reply(
                        &selection,
                        self.remember,
                        self.after_restart,
                        self.request.offer_persist,
                    );
                    self.reply.send(&line);
                }
            }
            Message::Cancel => std::process::exit(0),
            Message::Closed(id) if Some(id) == self.window_id => std::process::exit(0),
            Message::Closed(_) => {}
            Message::Key(iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)) => {
                std::process::exit(0)
            }
            Message::Key(iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter)) => {
                return self.update(Message::Share);
            }
            Message::Key(_) => {}
            Message::Shown(id) if Some(id) == self.window_id => {
                self.gate.shown(std::time::Instant::now());
                return ArmGate::wake(Message::Armed);
            }
            Message::Focused(id) if Some(id) == self.window_id => {
                if self.gate.focused(std::time::Instant::now()) {
                    return ArmGate::wake(Message::Armed);
                }
            }
            Message::Shown(_) | Message::Focused(_) | Message::Armed => {}
        }
        Task::none()
    }

    fn icon(path: Option<&PathBuf>, fallback: &str, size: f32) -> Element<'static, Message> {
        match path {
            Some(path)
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("svg")) =>
            {
                iced::widget::svg(iced::widget::svg::Handle::from_path(path))
                    .width(size)
                    .height(size)
                    .into()
            }
            Some(path) => iced::widget::image(path.clone())
                .width(size)
                .height(size)
                .into(),
            None => enz_icon(fallback).size(size).into_element(),
        }
    }

    fn source_row<'a>(
        &self,
        icon: Element<'a, Message>,
        title: String,
        detail: String,
        message: Message,
        selected: bool,
    ) -> Element<'a, Message> {
        let palette = self.tokens().palette;
        let text = column![
            enz_text(title.clone()).single_line_elided().into_element(),
            enz_text(detail)
                .size(TextSize::Footnote)
                .muted_on(palette.surface)
                .single_line_elided()
                .into_element(),
        ]
        .spacing(2);
        enz_list_row_surface(
            row![icon, text].spacing(12).align_y(Alignment::Center),
            message,
        )
        .selectable(true)
        .selected(selected)
        .label(title)
        .content_height(true)
        .width(Length::Fill)
        .into_element()
    }

    fn screen_detail(output: &Output) -> String {
        let size = tr!(
            "shell-share-picker-screen-size",
            "width" => output.width.to_string(),
            "height" => output.height.to_string(),
            "scale" => i18n::number(format!("{}", output.scale())),
        );
        if Self::screen_title(output) == output.name {
            size
        } else {
            tr!("shell-share-picker-screen-detail", "name" => output.name.clone(), "size" => size)
        }
    }

    fn screen_title(output: &Output) -> String {
        if output.description.trim().is_empty() {
            output.name.clone()
        } else {
            output.description.clone()
        }
    }

    fn empty(&self, text: &'static str) -> Element<'_, Message> {
        let palette = self.tokens().palette;
        container(enz_text(tr(text)).muted_on(palette.surface).into_element())
            .padding(12)
            .into()
    }

    fn screens(&self) -> Element<'_, Message> {
        if self.outputs.is_empty() {
            return self.empty("shell-share-picker-no-screens");
        }
        column(self.outputs.iter().map(|output| {
            self.source_row(
                enz_icon("monitor").size(28.0).into_element(),
                Self::screen_title(output),
                Self::screen_detail(output),
                Message::Screen(output.name.clone()),
                self.screen.as_deref() == Some(output.name.as_str()),
            )
        }))
        .spacing(4)
        .into()
    }

    fn windows(&self) -> Element<'_, Message> {
        if self.request.windows.is_empty() {
            return self.empty("shell-share-picker-no-windows");
        }
        column(self.request.windows.iter().map(|window: &SharedWindow| {
            let app = self
                .window_names
                .get(&window.class)
                .cloned()
                .unwrap_or_else(|| window.class.clone());
            let title = if window.title.trim().is_empty() {
                app.clone()
            } else {
                window.title.clone()
            };
            self.source_row(
                Self::icon(
                    self.window_icons
                        .get(&window.class)
                        .and_then(Option::as_ref),
                    "app-window",
                    28.0,
                ),
                title,
                app,
                Message::Window(window.handle),
                self.window == Some(window.handle),
            )
        }))
        .spacing(4)
        .into()
    }

    fn region_view(&self) -> Element<'_, Message> {
        if self.outputs.is_empty() {
            return self.empty("shell-share-picker-no-screens");
        }
        let palette = self.tokens().palette;
        let mut content = column(self.outputs.iter().map(|output| {
            self.source_row(
                enz_icon("monitor").size(28.0).into_element(),
                Self::screen_title(output),
                Self::screen_detail(output),
                Message::RegionOutput(output.name.clone()),
                self.region_output.as_deref() == Some(output.name.as_str()),
            )
        }))
        .spacing(4);
        let bounds = self
            .region_output
            .as_deref()
            .and_then(|n| self.output(n))
            .map_or(self.region, Output::full_region);
        let positive = |v: i32| u32::try_from(v.max(0)).unwrap_or(0);
        let field = |label: &'static str,
                     id: &'static str,
                     field: Field,
                     value: i32,
                     min: u32,
                     max: i32| {
            enz_stepper_u32(positive(value), move |v| Message::RegionField(field, v))
                .min(min)
                .max(positive(max).max(min))
                .label(tr(label))
                .id(iced::widget::Id::new(id))
                .into_element()
        };
        content = content.push(
            enz_text(tr("shell-share-picker-region-hint"))
                .size(TextSize::Footnote)
                .muted_on(palette.surface)
                .into_element(),
        );
        content = content.push(
            row![
                field(
                    "shell-share-picker-region-x",
                    "share-region-x",
                    Field::X,
                    self.region.x,
                    0,
                    bounds.width - 1
                ),
                field(
                    "shell-share-picker-region-y",
                    "share-region-y",
                    Field::Y,
                    self.region.y,
                    0,
                    bounds.height - 1
                ),
                field(
                    "shell-share-picker-region-width",
                    "share-region-width",
                    Field::Width,
                    self.region.width,
                    1,
                    bounds.width
                ),
                field(
                    "shell-share-picker-region-height",
                    "share-region-height",
                    Field::Height,
                    self.region.height,
                    1,
                    bounds.height
                ),
            ]
            .spacing(8),
        );
        if slurp_available() {
            content = content.push(
                enz_button(tr("shell-share-picker-region-draw"), Message::Draw)
                    .kind(ButtonKind::Secondary)
                    .enabled(!self.drawing)
                    .id("share-region-draw".into())
                    .into_element(),
            );
        }
        content.spacing(10).into()
    }

    fn view(&self, _: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
            let palette = self.tokens().palette;
            let heading = match &self.app_name {
                Some(name) => tr!("shell-share-picker-title-app", "app" => name.clone()),
                None => tr("shell-share-picker-title").to_owned(),
            };
            let header = row![
                Self::icon(self.app_icon.as_ref(), "screen-share", 40.0),
                column![
                    enz_text(heading)
                        .size(TextSize::Title3)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                    enz_text(tr("shell-share-picker-subtitle"))
                        .size(TextSize::Footnote)
                        .muted_on(palette.surface)
                        .into_element(),
                ]
                .spacing(2),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            let pages = vec![
                InlineSwitcherPage::new("screens", tr("shell-share-picker-screens"))
                    .with_icon("monitor"),
                InlineSwitcherPage::new("windows", tr("shell-share-picker-windows"))
                    .with_icon("app-window"),
                InlineSwitcherPage::new("region", tr("shell-share-picker-region"))
                    .with_icon("crop"),
            ];
            let selected = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
            let tabs = enz_inline_view_switcher(pages, selected, Message::Tab)
                .aria_label(tr("shell-share-picker-what"))
                .tab_id_prefix("share-picker-tab")
                .stretch(true);
            let page = match self.tab {
                Tab::Screens => self.screens(),
                Tab::Windows => self.windows(),
                Tab::Region => self.region_view(),
            };
            let persistable = self.tab != Tab::Windows;
            let keeps_after_restart =
                self.request.offer_persist && self.after_restart && persistable;
            let mut grants = column![
                enz_checkbox(
                    self.remember,
                    tr("shell-share-picker-remember"),
                    Message::Remember
                )
                .description(tr("shell-share-picker-remember-detail"))
                .enabled(!keeps_after_restart)
                .id("share-remember".into())
                .into_element(),
            ]
            .spacing(8);
            if self.request.offer_persist {
                grants = grants.push(
                    enz_checkbox(
                        keeps_after_restart,
                        tr("shell-share-picker-after-restart"),
                        Message::AfterRestart,
                    )
                    .description(tr(if persistable {
                        "shell-share-picker-after-restart-detail"
                    } else {
                        "shell-share-picker-after-restart-windows"
                    }))
                    .enabled(persistable)
                    .id("share-after-restart".into())
                    .into_element(),
                );
            }
            let buttons = row![
                Space::new().width(Length::Fill),
                enz_button(tr("shell-cancel"), Message::Cancel)
                    .kind(ButtonKind::Secondary)
                    .id("share-cancel".into())
                    .into_element(),
                enz_button(tr("shell-share-picker-share"), Message::Share)
                    .id("share-confirm".into())
                    .enabled(self.can_share())
                    .into_element(),
            ]
            .spacing(8);
            let content = column![
                header,
                tabs,
                enz_scroll_view(container(page).padding(iced::Padding {
                    right: 8.0,
                    ..Default::default()
                }))
                .height(Length::Fill)
                .into_element(),
                grants,
                buttons,
            ]
            .spacing(16);
            container(content)
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(palette.surface.into()),
                    ..Default::default()
                })
                .into()
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::close_requests().map(Message::Closed),
            window::events().filter_map(|(id, event)| {
                matches!(event, window::Event::Focused).then_some(Message::Focused(id))
            }),
            iced::event::listen_with_interest(
                iced::event::Interest::KEYBOARD,
                |event, status, _| match (event, status) {
                    (
                        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }),
                        iced::event::Status::Ignored,
                    ) => Some(Message::Key(key)),
                    _ => None,
                },
            ),
        ])
    }
}

fn main() -> iced::Result {
    let reply = Reply::take_stdout();
    i18n::initialize();
    let reply = std::cell::Cell::new(Some(reply));
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(
            move || {
                let (mut app, task) = App::new();
                if let Some(reply) = reply.take() {
                    app.reply = reply;
                }
                (app, task)
            },
            App::update,
            App::view,
        )
        .title(|_: &App, _| tr("shell-share-picker-window-title").to_owned())
        .theme(|app: &App, _| app.tokens().iced_theme())
        .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
