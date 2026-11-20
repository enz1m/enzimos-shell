use super::super::Message as AppMessage;
use super::gallery::{Gallery, LiveScene};
use enzui_shell::desktop_preferences::{
    Wallpaper as WallpaperPreference, WallpaperFit, WallpaperKind,
};
use enzui_shell::i18n::tr;
use enzui_widgets::{
    color_picker::Hsva,
    image::{EnzImageFit, enz_image_handle},
};
use iced::{
    Element, Length, Task,
    widget::{Space, container},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) const WALLPAPER_SETTLE: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PreviewVisual {
    Image(String),
    Color(iced::Color),
}

#[derive(Default)]
pub(crate) struct State {
    pub path: String,
    pub(crate) color: String,
    pub(crate) hsva: Hsva,
    pub(crate) color_dirty: bool,
    pub(crate) view_kind: Option<WallpaperKind>,
    pub busy: bool,
    pub(crate) native_drop: Option<enzui_widgets::file_drop::FileDrop>,
    pub(crate) native_saved_path: Option<String>,
    pub(crate) native_saved_lock: bool,
    pub supported: bool,
    pub preview_started: Option<Instant>,
    pub(crate) preview_previous: Option<PreviewVisual>,
    pub(crate) preview_showing: Option<PreviewVisual>,
    pub(crate) preview_handles: Vec<(String, iced::widget::image::Handle)>,
    pub(crate) preview_epoch: u64,
    pub(crate) preview_pending: Option<(u64, String)>,
    pub(crate) preview_failed: Option<String>,
    // Warm decoded textures on the GPU before fading away from the previous picture.
    pub(crate) preview_warming: Option<(u64, String)>,
    // Alternate crossfade slots; settling synchronizes the inactive slot without reversing the
    // fade.
    pub(crate) preview_slot: bool,
    pub(crate) cancel: Option<tokio::sync::watch::Sender<bool>>,
    pub(crate) status: Option<String>,
    pub(crate) gallery: Gallery,
    pub(crate) gallery_generation: u64,
    pub(crate) gallery_loading: bool,
    pub(crate) gallery_cancel: Option<tokio::sync::watch::Sender<bool>>,
    pub(crate) gallery_error: Option<String>,
    pub(crate) gallery_choice: Option<String>,
    // Cache gallery directory timestamps to avoid rereading unchanged contents.
    pub(crate) gallery_seen: Option<(PathBuf, Option<std::time::SystemTime>)>,
    pub(crate) gallery_reading: Option<(PathBuf, Option<std::time::SystemTime>)>,
    pub(crate) live: Vec<LiveScene>,
    pub(crate) live_generation: u64,
    pub(crate) live_loading: bool,
    pub(crate) live_error: Option<String>,
    pub(crate) import_choice: Option<String>,
    pub(crate) expected_digest: Option<String>,
    // When the service first reported a request older than the current
    // preference, so a slow change is reported but a normal one is not.
    pub(crate) pending_since: Option<Instant>,
}

impl State {
    pub(crate) fn preview_visual(&self, preference: &WallpaperPreference) -> PreviewVisual {
        let kind = self.view_kind.unwrap_or(preference.kind);
        let default = || {
            std::env::var_os("ENZUI_DESKTOP_ROOT").map(|root| {
                PathBuf::from(root)
                    .join("assets/wallpaper.png")
                    .to_string_lossy()
                    .into_owned()
            })
        };
        let path = match kind {
            WallpaperKind::Default => default(),
            // The gallery thumbnail is available before the asynchronous source import finishes.
            WallpaperKind::Image => self
                .gallery_choice
                .as_ref()
                .and_then(|id| self.gallery.entries.iter().find(|entry| &entry.id == id))
                .map(|entry| entry.thumbnail.clone())
                .or_else(|| preference.image.clone())
                .or_else(default),
            WallpaperKind::Color => None,
            WallpaperKind::Live => preference
                .scene
                .as_ref()
                .and_then(|id| self.live.iter().find(|scene| &scene.id == id))
                .or_else(|| self.live.first())
                .and_then(|scene| scene.preview.clone()),
        };
        path.map_or_else(
            || PreviewVisual::Color(self.hsva.to_color()),
            PreviewVisual::Image,
        )
    }

    pub(crate) fn handle(&self, path: &str) -> Option<iced::widget::image::Handle> {
        self.preview_handles
            .iter()
            .find(|(cached, _)| cached == path)
            .map(|(_, handle)| handle.clone())
    }

    pub(crate) fn displayed(&self, preference: &WallpaperPreference) -> PreviewVisual {
        // A picture not decoded yet (the imported copy of a chosen one, say)
        // keeps the last decoded picture on screen rather than a blank.
        let undecoded = || {
            self.preview_handles.last().map_or_else(
                || PreviewVisual::Color(self.hsva.to_color()),
                |(path, _)| PreviewVisual::Image(path.clone()),
            )
        };
        let showing = self
            .preview_showing
            .clone()
            .unwrap_or_else(|| self.preview_visual(preference));
        match showing {
            PreviewVisual::Image(path) if self.handle(&path).is_none() => undecoded(),
            visual => visual,
        }
    }

    pub(crate) fn remember_handle(&mut self, path: String, handle: iced::widget::image::Handle) {
        self.preview_handles.retain(|(cached, _)| cached != &path);
        self.preview_handles.push((path, handle));
        if self.preview_handles.len() > 4 {
            self.preview_handles.remove(0);
        }
    }

    pub(crate) fn transition_preview(
        &mut self,
        before: PreviewVisual,
        after: PreviewVisual,
        live_color: bool,
    ) -> Task<AppMessage> {
        if let PreviewVisual::Image(path) = &after
            && self.handle(path).is_none()
        {
            if self
                .preview_pending
                .as_ref()
                .is_some_and(|(_, pending)| pending == path)
            {
                return Task::none();
            }
            if self.preview_failed.as_deref() == Some(path) {
                return Task::none();
            }
            self.preview_epoch = self.preview_epoch.wrapping_add(1);
            let epoch = self.preview_epoch;
            self.preview_pending = Some((epoch, path.clone()));
            self.preview_previous = None;
            self.preview_started = None;
            self.preview_showing = Some(before);
            let decode = path.clone();
            let reply = path.clone();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || load_preview(&decode))
                        .await
                        .map_err(|_| tr("shell-wallpaper-preview-worker-stopped").to_owned())?
                },
                move |result| message(Message::PreviewLoaded(epoch, reply, result)),
            );
        }
        if self.preview_pending.is_some() {
            self.preview_epoch = self.preview_epoch.wrapping_add(1);
            self.preview_pending = None;
        }
        self.preview_failed = None;
        if before != after {
            self.preview_previous = (!live_color).then_some(before);
            self.preview_started = (!live_color).then(Instant::now);
            self.preview_showing = Some(after);
            if !live_color {
                self.preview_slot = !self.preview_slot;
            }
        }
        Task::none()
    }

    pub(crate) fn settle_preview(&mut self) {
        if self
            .preview_started
            .is_some_and(|start| start.elapsed() >= Duration::from_millis(580))
        {
            self.preview_previous = None;
            self.preview_started = None;
        }
    }

    // The colour picker is built only in the colour view, so a search result
    // for the background colour has to open that view to reveal the control.
    pub(crate) fn reveal_color_editor(&mut self) {
        self.view_kind = Some(WallpaperKind::Color);
    }
    pub(crate) fn for_environment() -> Self {
        let supported = (|| {
            use std::io::Read;
            if !enzui_shell::desktop_session::mode().managed()
                || !PathBuf::from(std::env::var_os("ENZUI_DESKTOP_ROOT")?).is_absolute()
            {
                return None;
            }
            let path = PathBuf::from(std::env::var_os("ENZUI_HYPR_RUNROOT")?).join("launch.json");
            let mut data = Vec::new();
            std::fs::File::open(path)
                .ok()?
                .take(65537)
                .read_to_end(&mut data)
                .ok()?;
            if data.len() > 65536 {
                return None;
            }
            let value: serde_json::Value = serde_json::from_slice(&data).ok()?;
            Some(value["wallpaper_version"] == 1)
        })()
        .unwrap_or(false);
        Self {
            supported,
            ..Self::default()
        }
    }
    pub(crate) fn pending(&self) -> bool {
        self.busy || self.gallery_loading
    }
    pub(crate) fn can_choose_source(&self, closing: bool, picking: bool) -> bool {
        !closing && !picking
    }
    pub(crate) fn cancel(&self) {
        for cancel in [&self.cancel, &self.gallery_cancel].into_iter().flatten() {
            let _ = cancel.send(true);
        }
    }
    // Delays a mismatch notice until WALLPAPER_SETTLE; current-choice errors remain immediate.
    pub(crate) fn settle(
        &mut self,
        matched: bool,
        error: Option<String>,
        now: Instant,
    ) -> Option<String> {
        if matched {
            self.pending_since = None;
            return error;
        }
        if !self.supported {
            self.pending_since = None;
            return None;
        }
        let since = *self.pending_since.get_or_insert(now);
        (now.duration_since(since) >= WALLPAPER_SETTLE)
            .then(|| tr("shell-waiting-for-the-wallpaper-service").to_owned())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Color(String),
    Picked(Hsva),
    CancelColor,
    Kind(WallpaperKind),
    Fit(WallpaperFit),
    ApplyColor,
    Browse,
    Retry,
    RefreshGallery,
    SelectGallery(u64, String),
    SelectLive(String),
    Motion(enzui_shell::desktop_preferences::WallpaperMotion),
    PauseIdle(bool),
    HideClock(bool),
    HideDate(bool),
    LiveLoaded(u64, Result<Vec<LiveScene>, String>),
    GalleryLoaded(u64, Result<Gallery, String>),
    Dropped(enzui_widgets::file_drop::FileDrop),
    Imported(Result<String, String>),
    Status(Option<(serde_json::Value, Option<String>)>),
    PreviewLoaded(u64, String, Result<iced::widget::image::Handle, String>),
    // The decoded preview has been drawn unseen for a moment: fade to it.
    PreviewWarm(u64, String),
}

pub(crate) fn message(value: Message) -> AppMessage {
    AppMessage::Wallpaper(value)
}

pub(crate) fn gallery_folder() -> PathBuf {
    std::env::var_os("ENZUI_WALLPAPERS_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Pictures/Wallpapers")
        })
}
pub(crate) fn parse_color(text: &str) -> Option<Hsva> {
    if text.len() != 7 || !text.starts_with('#') {
        return None;
    }
    let value = u32::from_str_radix(text.strip_prefix('#')?, 16).ok()?;
    Some(Hsva::from_color(iced::Color::from_rgb8(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    )))
}
pub(crate) fn load_preview(path: &str) -> Result<iced::widget::image::Handle, String> {
    let image = enzui_shell::profile_image::decode(std::path::Path::new(path))?;
    let image = raster::DynamicImage::ImageRgba8(image)
        .resize_to_fill(1040, 585, raster::imageops::FilterType::Triangle)
        .to_rgba8();
    Ok(iced::widget::image::Handle::from_rgba(
        1040,
        585,
        image.into_raw(),
    ))
}

pub(crate) fn preview_element(
    visual: PreviewVisual,
    handle: Option<iced::widget::image::Handle>,
    width: f32,
    height: f32,
    placeholder: iced::Color,
) -> Element<'static, AppMessage> {
    match (visual, handle) {
        (PreviewVisual::Image(_), Some(handle)) => enz_image_handle(handle)
            .alt_text(tr("shell-wallpaper-preview"))
            .fit(EnzImageFit::Cover)
            .radius(14.0)
            .placeholder_background(placeholder)
            .width(Length::Fixed(width))
            .height(Length::Fixed(height))
            .into_element(),
        (PreviewVisual::Image(_), None) => preview_element(
            PreviewVisual::Color(placeholder),
            None,
            width,
            height,
            placeholder,
        ),
        (PreviewVisual::Color(color), _) => container(Space::new())
            .width(width)
            .height(height)
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: iced::Border {
                    radius: 14.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    }
}
