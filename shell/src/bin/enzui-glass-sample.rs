//! Compositor-glass sample with live material controls and an opaque fallback.

use enzui_shell::i18n;
use enzui_theme::{EnzTheme, MaterialStyle};
use enzui_widgets::{
    desktop_glass::{self, Backdrop, WindowGlass, enz_desktop_glass},
    enz_sidebar_row, enz_text,
    surface::enz_surface,
    toolbar_icon::enz_toolbar_icon,
    view_switcher::{SwitcherPage, enz_view_switcher},
};
use iced::widget::{column, container, mouse_area, row, space};
use iced::{Color, Element, Length, Size, Subscription, Task, window};

const STYLES: [(&str, Option<MaterialStyle>); 6] = [
    ("Regular", Some(MaterialStyle::Regular)),
    ("Lens", Some(MaterialStyle::Lens)),
    ("Subtle", Some(MaterialStyle::Subtle)),
    ("Frosted", Some(MaterialStyle::Frosted)),
    ("Clear", Some(MaterialStyle::Clear)),
    ("Solid", None),
];
const PLACES: [(&str, &str); 5] = [
    ("Home", "house"),
    ("Documents", "file-text"),
    ("Pictures", "image"),
    ("Music", "music"),
    ("Downloads", "download"),
];

#[derive(Debug, Clone)]
enum Message {
    Opened(window::Id),
    Glass(WindowGlass),
    Style(usize),
    Place(usize),
    Appearance,
    Drag,
    Window(window::Id, window::Event),
    Backdrop,
}

struct Sample {
    window: Option<window::Id>,
    glass: WindowGlass,
    style: usize,
    place: usize,
    light: bool,
}

impl Sample {
    fn new() -> (Self, Task<Message>) {
        let (_, open) = window::open(window::Settings {
            size: Size::new(960.0, 620.0),
            transparent: true,
            decorations: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "dev.enzui.GlassSample".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let style = std::env::args()
            .find_map(|arg| {
                STYLES
                    .iter()
                    .position(|(label, _)| label.eq_ignore_ascii_case(&arg))
            })
            .unwrap_or(0);
        let sample = Self {
            window: None,
            glass: WindowGlass::pending(),
            style,
            place: 1,
            light: std::env::args().any(|arg| arg == "light"),
        };
        (sample, open.map(Message::Opened))
    }

    fn tokens(&self) -> EnzTheme {
        if self.light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Opened(id) => {
                self.window = Some(id);
                desktop_glass::bind(id).map(Message::Glass)
            }
            Message::Glass(glass) => {
                if !glass.is_bound() {
                    eprintln!(
                        "glass-sample: compositor glass unavailable; using framework fallback"
                    );
                }
                self.glass = glass;
                Task::none()
            }
            Message::Style(index) => {
                self.style = index.min(STYLES.len() - 1);
                Task::none()
            }
            Message::Place(index) => {
                self.place = index.min(PLACES.len() - 1);
                Task::none()
            }
            Message::Appearance => {
                self.light = !self.light;
                Task::none()
            }
            Message::Drag => self.window.map_or_else(Task::none, window::drag),
            Message::Window(id, window::Event::CloseRequested) if Some(id) == self.window => {
                iced::exit()
            }
            Message::Window(..) | Message::Backdrop => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::events().map(|(id, event)| Message::Window(id, event)),
            desktop_glass::luminance_changes(&self.glass).map(|()| Message::Backdrop),
        ])
    }

    fn view(&self, _: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
        let tokens = self.tokens();
        let _defaults = enzui_theme::text_defaults::TextDefaultsScope::enter(&tokens);
        let p = tokens.palette;
        let mut places = column![enz_text(enzui_shell::i18n::tr("shell-places")).size_px(12.0).into_element()].spacing(2);
        for (index, (label, icon)) in PLACES.iter().enumerate() {
            places = places.push(enz_sidebar_row(*label, Message::Place(index)).icon(*icon).selected(index == self.place).into_element());
        }
        let pages = STYLES.iter().map(|(label, _)| SwitcherPage::new(*label, *label)).collect();
        let titlebar = mouse_area(
            container(
                row![
                    enz_text(enzui_shell::i18n::tr("shell-glass-sample")).size_px(14.0).into_element(),
                    space().width(Length::Fill),
                    enz_view_switcher(pages, self.style, Message::Style).into_element(),
                    enz_toolbar_icon(if self.light { "moon" } else { "sun" }, Message::Appearance)
                        .aria_label(if self.light { enzui_shell::i18n::tr("shell-dark-appearance") } else { enzui_shell::i18n::tr("shell-light-appearance") })
                        .into_element(),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            )
            .padding([10, 16])
            .width(Length::Fill),
        )
        .on_press(Message::Drag);
        let mut sidebar = enz_surface(places)
            .radius_corners([12.0, 0.0, 12.0, 0.0])
            .padding(14)
            .fill(p.surface_alt)
            .width(260)
            .height(Length::Fill);
        if let Some(style) = STYLES[self.style].1 {
            sidebar = sidebar.material(style).backdrop(Backdrop::Window);
        }
        let (title, _) = PLACES[self.place];
        let mut files = column![
            enz_text(title).size_px(22.0).into_element(),
            enz_text(enzui_shell::i18n::tr("shell-the-sidebar-is-glass-over-whatever-lies-behind-this-window-move-the-window-o"))
                .size_px(14.0)
                .into_element(),
        ]
        .spacing(12);
        for name in ["Quarterly report.pdf", "Holiday plan.odt", "Notes.md", "Budget 2027.ods", "Readme.txt"] {
            files = files.push(enz_sidebar_row(name, Message::Place(self.place)).icon("file").into_element());
        }
        let content = container(column![titlebar, container(files).padding([8, 24])])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(p.surface.into()),
                ..Default::default()
            });
        enz_desktop_glass(&self.glass, row![sidebar.into_element(), content]).into_element()

        })
    }
}

fn main() -> iced::Result {
    i18n::initialize();
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(Sample::new, Sample::update, Sample::view)
            .title(|_: &Sample, _| "Glass sample".to_owned())
            .theme(|sample: &Sample, _| sample.tokens().iced_theme())
            .style(|sample: &Sample, _| iced::theme::Style {
                background_color: Color::TRANSPARENT,
                text_color: sample.tokens().palette.text,
            })
            .subscription(Sample::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
