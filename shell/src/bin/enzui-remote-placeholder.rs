//! Displays a remote application frame and forwards input over its authenticated connection.
use enzui_shell::i18n::{self, tr};
use enzui_widgets::{
    enz_button, enz_card, enz_text,
    image::{EnzImageFit, enz_image_handle},
};
use iced::widget::{column, container, image, row, stack};
use iced::{Alignment, Color, Element, Length, Subscription, Task, window};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

#[derive(Clone)]
struct Target {
    socket: PathBuf,
    token: String,
    pid: i32,
}
#[derive(Debug)]
struct Connection {
    stream: Option<BufReader<UnixStream>>,
}
#[derive(Debug, Clone)]
struct Prepared {
    connection: Arc<Mutex<Connection>>,
    pixels: image::Handle,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
enum Message {
    Prepared(Result<Prepared, String>),
    Opened(window::Id),
    Tick,
    Restore,
    Status(Result<Value, String>),
    Window(window::Id, window::Event),
}

struct Placeholder {
    prepared: Option<Prepared>,
    window: Option<window::Id>,
    busy: bool,
    restoring: bool,
}

impl Connection {
    async fn request(&mut self, request: Value) -> Result<Value, String> {
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| tr("shell-control-connection-is-closed"))?;
            let mut encoded =
                serde_json::to_vec(&request).map_err(|_| tr("shell-invalid-request"))?;
            encoded.push(b'\n');
            stream
                .get_mut()
                .write_all(&encoded)
                .await
                .map_err(|_| tr("shell-control-write-failed"))?;
            let mut line = Vec::new();
            loop {
                let buffer = stream
                    .fill_buf()
                    .await
                    .map_err(|_| tr("shell-control-read-failed"))?;
                if buffer.is_empty() {
                    return Err(tr("shell-desktop-controller-closed").to_owned());
                }
                let end = buffer.iter().position(|byte| *byte == b'\n');
                let length = end.map_or(buffer.len(), |offset| offset + 1);
                if line.len() + length > 16_384 {
                    return Err(tr("shell-control-reply-exceeds-bound").into());
                }
                line.extend_from_slice(&buffer[..length]);
                stream.consume(length);
                if end.is_some() {
                    break;
                }
            }
            let reply: Value =
                serde_json::from_slice(&line).map_err(|_| tr("shell-invalid-control-reply"))?;
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                return Err(tr("shell-desktop-rejected-placeholder-control").into());
            }
            Ok(reply)
        })
        .await
        .unwrap_or_else(|_| Err(tr("shell-desktop-controller-timed-out").into()));
        if result.is_err() {
            self.stream.take();
        }
        result
    }
}

async fn prepare(target: Target) -> Result<Prepared, String> {
    let stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(target.socket))
        .await
        .map_err(|_| tr("shell-desktop-connection-timed-out"))?
        .map_err(|_| tr("shell-desktop-control-is-unavailable"))?;
    if stream
        .peer_cred()
        .map_err(|_| "Cannot verify desktop peer")?
        .pid()
        != Some(target.pid)
    {
        return Err(tr("shell-desktop-controller-identity-changed").into());
    }
    let mut connection = Connection {
        stream: Some(BufReader::new(stream)),
    };
    let description = connection
        .request(json!({"v":1,"op":"ui_attach","token":target.token}))
        .await?;
    let width = dimension(&description, "desktop_width")?;
    let height = dimension(&description, "desktop_height")?;
    let path = PathBuf::from(
        description
            .get("snapshot_path")
            .and_then(Value::as_str)
            .ok_or_else(|| tr("shell-snapshot-path-is-missing"))?,
    );
    if !path.is_absolute() {
        return Err(tr("shell-snapshot-path-must-be-absolute").into());
    }
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| tr("shell-snapshot-is-unavailable"))?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err(tr("shell-snapshot-exceeds-its-bound").into());
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| tr("shell-snapshot-read-failed"))?;
    let pixels = tokio::task::spawn_blocking(move || decode_snapshot(bytes))
        .await
        .map_err(|_| tr("shell-snapshot-decode-failed"))??;
    Ok(Prepared {
        connection: Arc::new(Mutex::new(connection)),
        pixels,
        width,
        height,
    })
}

fn dimension(value: &Value, name: &str) -> Result<f32, String> {
    let number = value
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| tr("shell-missing-desktop-geometry"))?;
    if !number.is_finite() || !(1.0..=8192.0).contains(&number) {
        return Err(tr("shell-invalid-desktop-geometry").into());
    }
    Ok(number as f32)
}

fn decode_snapshot(bytes: Vec<u8>) -> Result<image::Handle, String> {
    if bytes.len() < 24 || bytes[..8] != *b"\x89PNG\r\n\x1a\n" {
        return Err(tr("shell-snapshot-is-not-png").into());
    }
    let width = u32::from_be_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| tr("shell-invalid-png-header"))?,
    );
    let height = u32::from_be_bytes(
        bytes[20..24]
            .try_into()
            .map_err(|_| tr("shell-invalid-png-header"))?,
    );
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 32 * 1024 * 1024
    {
        return Err(tr("shell-snapshot-dimensions-exceed-their-bound").into());
    }
    let decoded = raster::load_from_memory_with_format(&bytes, raster::ImageFormat::Png)
        .map_err(|_| tr("shell-invalid-snapshot-png"))?
        .into_rgba8();
    Ok(image::Handle::from_rgba(
        decoded.width(),
        decoded.height(),
        decoded.into_raw(),
    ))
}

impl Placeholder {
    fn new(target: Target) -> (Self, Task<Message>) {
        (
            Self {
                prepared: None,
                window: None,
                busy: false,
                restoring: false,
            },
            Task::perform(prepare(target), Message::Prepared),
        )
    }

    fn request(&mut self, restore: bool) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let Some(prepared) = &self.prepared else {
            return Task::none();
        };
        self.busy = true;
        let connection = std::sync::Arc::clone(&prepared.connection);
        Task::perform(
            async move {
                connection
                    .lock()
                    .await
                    .request(json!({"v":1,"op":if restore {"restore"} else {"status"}}))
                    .await
            },
            Message::Status,
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Prepared(Ok(prepared)) => {
                let settings = window::Settings {
                    size: iced::Size::new(prepared.width, prepared.height),
                    resizable: true,
                    decorations: false,
                    transparent: false,
                    exit_on_close_request: false,
                    platform_specific: window::settings::PlatformSpecific {
                        application_id: "dev.enzui.RemotePlaceholder".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                };
                self.prepared = Some(prepared);
                let (id, open) = window::open(settings);
                self.window = Some(id);
                open.map(Message::Opened)
            }
            Message::Prepared(Err(error)) | Message::Status(Err(error)) => {
                eprintln!("remote-placeholder: {error}");
                iced::exit()
            }
            Message::Opened(id) => {
                self.window = Some(id);
                iced::widget::operation::focus_next()
            }
            Message::Restore => {
                self.restoring = true;
                self.request(true)
            }
            Message::Tick => self.request(self.restoring),
            Message::Status(Ok(status)) => {
                self.busy = false;
                match status.get("phase").and_then(Value::as_str) {
                    Some("restored" | "closed" | "failed") => iced::exit(),
                    Some("restoring") => {
                        self.restoring = true;
                        Task::none()
                    }
                    _ => Task::none(),
                }
            }
            Message::Window(id, window::Event::CloseRequested) if Some(id) == self.window => {
                self.restoring = true;
                self.request(true)
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.window => iced::exit(),
            Message::Window(_, _) => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(200)).map(|_| Message::Tick),
            window::events().map(|(id, event)| Message::Window(id, event)),
        ])
    }

    fn view(&self, _: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
            let Some(prepared) = &self.prepared else {
                return container("").into();
            };
            let background = enz_image_handle(prepared.pixels.clone())
                .fit(EnzImageFit::Fill)
                .width(Length::Fill)
                .height(Length::Fill)
                .radius(0.0)
                .into_element();
            let title = if self.restoring {
                tr("shell-returning-to-desktop")
            } else {
                tr("shell-controlled-by-phone")
            };
            let content = column![
                enz_text(title).size_px(24.0).into_element(),
                enz_text(tr(
                    "shell-this-is-the-same-running-app-its-unsaved-work-stays-open"
                ))
                .size_px(14.0)
                .into_element(),
            ]
            .spacing(16)
            .align_x(Alignment::Center);
            let card = enz_card(content)
                .padding(28)
                .width_length(Length::Fill)
                .into_element();
            let mut heading = row![
                enz_text(title)
                    .size_px(14.0)
                    .width(Length::Fill)
                    .into_element()
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            if !self.restoring {
                heading = heading
                    .push(enz_button(tr("shell-resume-here"), Message::Restore).into_element());
            }
            let heading = container(heading)
                .padding(12)
                .width(Length::Fill)
                .style(|_| container::Style {
                    background: Some(Color::from_rgb8(32, 33, 36).into()),
                    ..Default::default()
                });
            let center = container(container(card).width(Length::Fill.max(420.0)))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .padding(20);
            let overlay = container(column![heading, center])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba(0.04, 0.045, 0.055, 0.68).into()),
                    ..Default::default()
                });
            stack![background, overlay].into()
        })
    }
}

fn main() -> iced::Result {
    i18n::initialize();
    let target = (|| {
        let socket = PathBuf::from(
            std::env::var_os("ENZUI_REMOTE_SOCKET")
                .ok_or_else(|| tr("shell-missing-control-endpoint"))?,
        );
        if !socket.is_absolute() {
            return Err(tr("shell-control-endpoint-must-be-absolute"));
        }
        let token = std::env::var("ENZUI_REMOTE_TOKEN")
            .map_err(|_| tr("shell-missing-placeholder-capability"))?;
        if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(tr("shell-invalid-placeholder-capability"));
        }
        let pid = std::env::var("ENZUI_REMOTE_COMPOSITOR_PID")
            .map_err(|_| tr("shell-missing-desktop-identity"))?
            .parse::<i32>()
            .map_err(|_| tr("shell-invalid-desktop-identity"))?;
        if pid <= 1 {
            return Err(tr("shell-invalid-desktop-identity"));
        }
        Ok(Target { socket, token, pid })
    })()
    .unwrap_or_else(|error: &str| {
        eprintln!("remote-placeholder: {error}");
        std::process::exit(2);
    });
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(
            move || Placeholder::new(target.clone()),
            Placeholder::update,
            Placeholder::view,
        )
        .title(|_: &Placeholder, _| tr("shell-controlled-by-phone").to_owned())
        .theme(|_: &Placeholder, _| iced::Theme::Dark)
        .subscription(Placeholder::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enzui_remote_placeholder_behaviour() {
        // Rejects unbounded or missing geometry.
        {
            assert_eq!(dimension(&json!({"w":1024}), "w").unwrap(), 1024.0);
            for bad in [
                json!({}),
                json!({"w":0}),
                json!({"w":9000}),
                json!({"w":"1024"}),
            ] {
                assert!(dimension(&bad, "w").is_err());
            }
        }
        // Rejects snapshot bombs before decode.
        {
            assert!(decode_snapshot(vec![]).is_err());
            let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            data.extend_from_slice(&8192u32.to_be_bytes());
            data.extend_from_slice(&8192u32.to_be_bytes());
            assert!(decode_snapshot(data).unwrap_err().contains("bound"));
        }
    }
}
