//! Encrypted-drive passphrase dialog; prints the cleartext object and mount point after
//! success.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]

use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    icon::enz_icon,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Subscription, Task, window};

#[derive(Clone, Debug)]
enum Message {
    Passphrase(String),
    Submit,
    Done(Result<(String, std::path::PathBuf), String>),
    Cancel,
    Opened,
}

struct App {
    path: String,
    label: String,
    passphrase: String,
    error: Option<String>,
    checking: bool,
    light: bool,
}

impl App {
    fn new(path: String, label: String) -> (Self, Task<Message>) {
        let light = enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light);
        (
            Self {
                path,
                label,
                passphrase: String::new(),
                error: None,
                checking: false,
                light,
            },
            Task::done(Message::Opened),
        )
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
            Message::Opened => return iced::widget::operation::focus("drive-passphrase"),
            Message::Passphrase(text) => self.passphrase = text.chars().take(512).collect(),
            Message::Submit if !self.checking && !self.passphrase.is_empty() => {
                self.checking = true;
                self.error = None;
                let (path, passphrase) = (self.path.clone(), std::mem::take(&mut self.passphrase));
                return Task::perform(
                    async move { enzui_shell::drives::unlock(&path, &passphrase).await },
                    Message::Done,
                );
            }
            Message::Submit => {}
            Message::Done(Ok((cleartext, mount))) => {
                println!("{cleartext} {}", mount.display());
                return iced::exit();
            }
            Message::Done(Err(error)) => {
                self.checking = false;
                self.error = Some(error);
                return iced::widget::operation::focus("drive-passphrase");
            }
            Message::Cancel => return iced::exit(),
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        i18n::localizer().view(|| {
            let palette = self.tokens().palette;
            let mut field = enz_text_field(
                enzui_shell::i18n::current().text("shell-passphrase"),
                "",
                &self.passphrase,
                Message::Passphrase,
            )
            .secure(true)
            .on_submit(Message::Submit)
            .id("drive-passphrase".into())
            .enabled(!self.checking);
            if let Some(error) = &self.error {
                field = field.error(error.clone());
            }
            let ready = !self.checking && !self.passphrase.is_empty();
            let content = column![
                row![
                    enz_icon("hard-drive").size(22.0).into_element(),
                    enz_text(format!("Unlock {}", self.label))
                        .size(TextSize::Title3)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                enz_text(tr(
                    "shell-this-drive-is-encrypted-enter-its-passphrase-to-open-it"
                ))
                .muted_on(palette.surface)
                .into_element(),
                field.into_element(),
                Space::new().height(Length::Fill),
                row![
                    Space::new().width(Length::Fill),
                    enz_button(enzui_shell::i18n::tr("shell-cancel"), Message::Cancel)
                        .kind(ButtonKind::Secondary)
                        .id("drive-unlock-cancel".into())
                        .into_element(),
                    enz_button(
                        if self.checking {
                            enzui_shell::i18n::tr("shell-unlocking")
                        } else {
                            enzui_shell::i18n::tr("shell-unlock")
                        },
                        Message::Submit
                    )
                    .id("drive-unlock-submit".into())
                    .enabled(ready)
                    .into_element(),
                ]
                .spacing(8),
            ]
            .spacing(14);
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
        iced::keyboard::listen().filter_map(|event| match event {
            iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            } => Some(Message::Cancel),
            _ => None,
        })
    }
}

fn main() -> iced::Result {
    i18n::initialize();
    let mut args = std::env::args().skip(1);
    let (Some(path), label) = (args.next(), args.next()) else {
        eprintln!("usage: enzui-drive-unlock <udisks block object> [name]");
        std::process::exit(2);
    };
    if !enzui_shell::drives::block_object(&path) {
        eprintln!("enzui-drive-unlock: not a UDisks2 block device");
        std::process::exit(2);
    }
    let label: String = label
        .unwrap_or_else(|| "drive".into())
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    enzui_widgets::app_setup::run_with_services(
        iced::application(
            move || App::new(path.clone(), label.clone()),
            App::update,
            App::view,
        )
        .title(|app: &App| tr!("shell-unlock-3", "arg0" => app.label.clone()))
        .theme(|app: &App| app.tokens().iced_theme())
        .subscription(App::subscription)
        .window(window::Settings {
            size: iced::Size::new(420.0, 250.0),
            resizable: false,
            decorations: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "dev.enzui.DriveUnlock".into(),
                ..Default::default()
            },
            ..Default::default()
        }),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
