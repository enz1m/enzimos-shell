use super::super::{Message as AppMessage, Page, SettingsApp};
use super::{
    gallery::{parse_gallery, parse_live_scenes, run_child},
    preview::{LivePreviewResult, Message, PreviewVisual, gallery_folder, message, parse_color},
};
use enzui_shell::i18n::tr;
use enzui_shell::{
    desktop_preferences::{WallpaperChange, WallpaperKind},
    settings::Change,
};
use enzui_widgets::color_picker::Hsva;
use iced::Task;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

impl SettingsApp {
    pub(crate) fn read_wallpaper_gallery(&mut self) -> Task<AppMessage> {
        if self.page != Page::Appearance
            || self.closing
            || !self.wallpaper.supported
            || self.wallpaper.gallery_loading
        {
            return Task::none();
        }
        let Some(root) = std::env::var_os("ENZUI_DESKTOP_ROOT")
            .map(PathBuf::from)
            .filter(|root| root.is_absolute())
        else {
            return Task::none();
        };
        let folder = gallery_folder();
        let seen = (
            folder.clone(),
            std::fs::metadata(&folder)
                .and_then(|info| info.modified())
                .ok(),
        );
        if self.wallpaper.gallery_seen.as_ref() == Some(&seen)
            && self.wallpaper.gallery_error.is_none()
        {
            return Task::none();
        }
        self.wallpaper.gallery_reading = Some(seen);
        let cache = self.store.path().with_file_name("wallpaper-thumbnails");
        let input = serde_json::to_vec(&serde_json::json!({"folder":folder,"cache":cache}))
            .unwrap_or_default();
        self.wallpaper.gallery_generation = self.wallpaper.gallery_generation.wrapping_add(1);
        let generation = self.wallpaper.gallery_generation;
        self.wallpaper.gallery_loading = true;
        self.wallpaper.gallery_error = None;
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        self.wallpaper.gallery_cancel = Some(cancel);
        let live = self.read_live_scenes(&root, receiver.clone());
        let gallery = Task::perform(
            async move {
                let (ok, output) = run_child(
                    tokio::process::Command::new(root.join("scripts/wallpaper-gallery")),
                    input,
                    receiver,
                    Duration::from_secs(20),
                    65536,
                )
                .await?;
                if !ok {
                    return Err(tr(
                        "shell-cannot-load-wallpapers-check-the-folder-and-try-refresh",
                    )
                    .into());
                }
                parse_gallery(&output, &cache)
            },
            move |result| message(Message::GalleryLoaded(generation, result)),
        );
        Task::batch([gallery, live])
    }
    fn read_live_scenes(
        &mut self,
        root: &Path,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Task<AppMessage> {
        let Some(binary) = enzui_shell::wallpaper_snapshot::binary() else {
            return Task::none();
        };
        let scenes = root.join("enzwall/scenes");
        self.wallpaper.live_generation = self.wallpaper.live_generation.wrapping_add(1);
        let generation = self.wallpaper.live_generation;
        self.wallpaper.live_loading = true;
        self.wallpaper.live_error = None;
        Task::perform(
            async move {
                let mut command = tokio::process::Command::new(binary);
                command.arg("list").arg(&scenes);
                let (ok, output) =
                    run_child(command, Vec::new(), cancel, Duration::from_secs(10), 65536).await?;
                if !ok {
                    return Err(tr("shell-cannot-list-live-wallpapers").into());
                }
                parse_live_scenes(&output, &scenes)
            },
            move |result| message(Message::LiveLoaded(generation, result)),
        )
    }
    pub(crate) fn wallpaper_status(&self) -> Task<AppMessage> {
        let path = self.store.path().with_file_name("wallpaper-status.json");
        Task::perform(
            async move {
                let file = tokio::fs::File::open(path).await.ok()?;
                let mut data = Vec::new();
                file.take(8193).read_to_end(&mut data).await.ok()?;
                if data.len() > 8192 {
                    return None;
                }
                let value: serde_json::Value = serde_json::from_slice(&data).ok()?;
                Some((
                    value.get("request")?.clone(),
                    value.get("error")?.as_str().map(str::to_owned),
                ))
            },
            |result| message(Message::Status(result)),
        )
    }
    pub(crate) fn wallpaper_import(&mut self) -> Task<AppMessage> {
        if self.closing
            || self.wallpaper.busy
            || (!self.wallpaper.supported
                && matches!(self.image_purpose, super::super::ImagePurpose::Desktop))
        {
            return Task::none();
        }
        let Some(root) = std::env::var_os("ENZUI_DESKTOP_ROOT")
            .map(PathBuf::from)
            .filter(|root| root.is_absolute())
        else {
            self.error = Some(tr("shell-image-selection-requires-the-managed-desktop").into());
            return Task::none();
        };
        let source = self.wallpaper.path.trim().to_owned();
        let Some(parent) = self.store.path().parent() else {
            return Task::none();
        };
        let cache = parent.join("wallpapers");
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        self.wallpaper.cancel = Some(cancel);
        self.wallpaper.busy = true;
        self.error = None;
        let input = serde_json::to_vec(&serde_json::json!({"source":source,"cache":cache,"expected_sha256":self.wallpaper.expected_digest.take()})).unwrap_or_default();
        Task::perform(
            async move {
                let (success, output) = run_child(
                    tokio::process::Command::new(root.join("scripts/import-wallpaper")),
                    input,
                    receiver,
                    Duration::from_secs(10),
                    8192,
                )
                .await?;
                let result: serde_json::Value = serde_json::from_slice(&output)
                    .map_err(|_| tr("shell-cannot-import-this-image"))?;
                if !success {
                    return Err(result
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| tr("shell-cannot-import-this-image"))
                        .to_owned());
                }
                let path = result
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| tr("shell-image-import-did-not-return-a-path"))?;
                if Path::new(path).parent() != Some(cache.as_path()) || !path.ends_with(".png") {
                    return Err(tr("shell-image-importer-returned-an-unexpected-location").into());
                }
                Ok(path.to_owned())
            },
            |result| message(Message::Imported(result)),
        )
    }
    pub(crate) fn refresh_live_wallpaper_preview(&mut self) -> Task<AppMessage> {
        if self
            .wallpaper
            .view_kind
            .unwrap_or(self.settings.wallpaper.kind)
            != WallpaperKind::Live
        {
            return Task::none();
        }
        self.warm_wallpaper_preview()
    }

    pub(crate) fn warm_wallpaper_preview(&mut self) -> Task<AppMessage> {
        if self.page != Page::Appearance || self.closing || !self.wallpaper.supported {
            return Task::none();
        }
        let before = self.wallpaper.displayed(&self.settings.wallpaper);
        self.wallpaper
            .refresh_preview(&self.settings.wallpaper, before, false)
    }

    pub(crate) fn wallpaper_message(&mut self, event: Message) -> Task<AppMessage> {
        let preference = self.settings.wallpaper.clone();
        let before = self.wallpaper.displayed(&preference);
        let live_color = matches!(&event, Message::Color(_) | Message::Picked(_));
        let mut apply = None;
        match event {
            Message::Color(value) if !self.closing => {
                self.wallpaper.color = value.chars().take(7).collect();
                if let Some(mut color) = parse_color(&self.wallpaper.color) {
                    if color.saturation == 0.0 {
                        color.hue = self.wallpaper.hsva.hue;
                    }
                    self.wallpaper.hsva = color;
                }
                self.wallpaper.color_dirty = true;
            }
            Message::Picked(value) if !self.closing => {
                self.wallpaper.hsva = Hsva {
                    alpha: 1.0,
                    ..value.sanitised()
                };
                self.wallpaper.color = self.wallpaper.hsva.to_hex();
                self.wallpaper.color_dirty = true;
            }
            Message::CancelColor if !self.closing => {
                self.wallpaper.color.clone_from(&preference.color);
                self.wallpaper.hsva = parse_color(&preference.color).unwrap_or_default();
                self.wallpaper.color_dirty = false;
            }
            Message::Kind(kind) if !self.closing && !self.wallpaper.busy => {
                self.menu = None;
                self.wallpaper.view_kind = Some(kind);
                if kind == WallpaperKind::Default {
                    apply = Some(WallpaperChange::Kind(kind));
                }
                if kind == WallpaperKind::Image && preference.image.is_some() {
                    apply = Some(WallpaperChange::Kind(kind));
                }
                if kind == WallpaperKind::Live && preference.scene.is_some() {
                    apply = Some(WallpaperChange::Kind(kind));
                }
            }
            Message::Fit(fit) if !self.closing => apply = Some(WallpaperChange::Fit(fit)),
            Message::ApplyColor if !self.closing && !self.wallpaper.busy => {
                if parse_color(&self.wallpaper.color).is_none() {
                    self.error =
                        Some(tr("shell-choose-a-color-or-enter-six-hexadecimal-digits").into());
                } else {
                    apply = Some(WallpaperChange::Solid(self.wallpaper.color.clone()));
                }
            }
            Message::Browse => return self.start_wallpaper_picker(),
            Message::Retry if !self.closing => {
                self.wallpaper.preview_failed = None;
                self.wallpaper.live_preview.forget();
                apply = Some(WallpaperChange::Retry);
            }
            Message::RefreshGallery => {
                self.wallpaper.gallery_seen = None;
                return self.read_wallpaper_gallery();
            }
            Message::SelectGallery(generation, id)
                if !self.closing
                    && !self.wallpaper.busy
                    && generation == self.wallpaper.gallery_generation =>
            {
                if let Some(entry) = self
                    .wallpaper
                    .gallery
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                {
                    self.image_purpose = super::super::ImagePurpose::Desktop;
                    self.wallpaper.path = entry.source.clone();
                    self.wallpaper.expected_digest = Some(entry.digest.clone());
                    self.wallpaper.import_choice = Some(id.clone());
                    self.wallpaper.gallery_choice = Some(id);
                    let preview = self.wallpaper.refresh_preview(&preference, before, false);
                    return Task::batch([preview, self.wallpaper_import()]);
                }
            }
            Message::SelectLive(id)
                if !self.closing
                    && !self.wallpaper.busy
                    && self.wallpaper.live.iter().any(|scene| scene.id == id) =>
            {
                self.wallpaper.view_kind = Some(WallpaperKind::Live);
                apply = Some(WallpaperChange::Live(id));
            }
            Message::Motion(motion) if !self.closing => {
                apply = Some(WallpaperChange::Motion(motion));
            }
            Message::PauseIdle(pause) if !self.closing => {
                apply = Some(WallpaperChange::PauseIdle(pause));
            }
            Message::HideClock(hide) if !self.closing => {
                apply = Some(WallpaperChange::HideClock(hide));
            }
            Message::HideDate(hide) if !self.closing => {
                apply = Some(WallpaperChange::HideDate(hide));
            }
            Message::LiveLoaded(generation, result)
                if generation == self.wallpaper.live_generation =>
            {
                self.wallpaper.live_loading = false;
                match result {
                    Ok(scenes) => self.wallpaper.live = scenes,
                    Err(error) => self.wallpaper.live_error = Some(error),
                }
            }
            Message::GalleryLoaded(generation, result)
                if generation == self.wallpaper.gallery_generation =>
            {
                self.wallpaper.gallery_loading = false;
                self.wallpaper.gallery_cancel = None;
                if !self.closing {
                    match result {
                        Ok(gallery) => {
                            self.wallpaper.gallery = gallery;
                            self.wallpaper.gallery_seen = self.wallpaper.gallery_reading.take();
                        }
                        Err(error) => self.wallpaper.gallery_error = Some(error),
                    }
                }
            }
            Message::Dropped(drop) => {
                if self.page != Page::Appearance
                    || self.closing
                    || self.wallpaper.busy
                    || self.wallpaper.native_drop.is_some()
                    || !self.wallpaper.supported
                    || drop.paths().len() != 1
                    || drop
                        .action()
                        .is_some_and(|action| action != enzui_widgets::file_drop::DragAction::Copy)
                {
                    return Task::none();
                }
                let Some(path) = drop.paths()[0].to_str() else {
                    return Task::none();
                };
                self.image_purpose = super::super::ImagePurpose::Desktop;
                path.clone_into(&mut self.wallpaper.path);
                self.wallpaper.expected_digest = None;
                self.wallpaper.import_choice = None;
                self.wallpaper.native_saved_lock = false;
                self.wallpaper.native_drop = Some(drop);
                let task = self.wallpaper_import();
                if !self.wallpaper.busy {
                    self.wallpaper.native_drop = None;
                }
                return task;
            }
            Message::Imported(result) => {
                self.wallpaper.busy = false;
                if result.is_err() || self.closing {
                    self.wallpaper.native_drop = None;
                    self.wallpaper.native_saved_path = None;
                }
                let purpose = std::mem::take(&mut self.image_purpose);
                if !matches!(purpose, super::super::ImagePurpose::Desktop) {
                    self.wallpaper.cancel = None;
                    if self.closing {
                        return if !self.wallpaper.pending()
                            && !self.busy
                            && !self.plugins.busy()
                            && !self.users_pending()
                        {
                            iced::exit()
                        } else {
                            Task::none()
                        };
                    }
                    return match result {
                        Ok(path) => match purpose {
                            super::super::ImagePurpose::Lock => {
                                if self.wallpaper.native_drop.is_some() {
                                    self.wallpaper.native_saved_lock = true;
                                    self.wallpaper.native_saved_path = Some(path.clone());
                                }
                                self.update(AppMessage::Change(enzui_shell::settings::Change::Session(
                                    enzui_shell::session_preferences::SessionChange::LockBackground(Some(path)),
                                )))
                            }
                            super::super::ImagePurpose::Avatar(uid) => {
                                self.users_message(super::super::users::Message::Apply(
                                    enzui_shell::user_accounts::Action::Avatar(uid, path),
                                ))
                            }
                            _ => Task::none(),
                        },
                        Err(error) => {
                            self.error = Some(error);
                            Task::none()
                        }
                    };
                }
                self.wallpaper.cancel = None;
                let choice = self.wallpaper.import_choice.take();
                if !self.closing {
                    match result {
                        Ok(path) => {
                            if self.wallpaper.native_drop.is_some() {
                                self.wallpaper.native_saved_path = Some(path.clone());
                            }
                            self.wallpaper.gallery_choice = choice;
                            self.wallpaper.view_kind = Some(WallpaperKind::Image);
                            apply = Some(WallpaperChange::Image(path));
                        }
                        Err(error) => {
                            // Undo optimistic selection only if the failed import still owns
                            // it.
                            if self.wallpaper.gallery_choice == choice {
                                self.wallpaper.gallery_choice = None;
                            }
                            self.error = Some(error);
                        }
                    }
                }
            }
            Message::LivePreviewLoaded(key, result) => {
                let current = (self.page == Page::Appearance && !self.closing)
                    .then(|| self.wallpaper.current_live_preview_key(&preference))
                    .flatten();
                if !self.wallpaper.live_preview.finish(&key, current.as_ref()) {
                    return self.warm_wallpaper_preview();
                }
                match result {
                    LivePreviewResult::Snapshot(path, handle) => {
                        self.wallpaper
                            .live_preview
                            .remember(key, PreviewVisual::Image(path.clone()));
                        self.wallpaper.preview_epoch = self.wallpaper.preview_epoch.wrapping_add(1);
                        return self.wallpaper.warm_preview_handle(
                            self.wallpaper.preview_epoch,
                            path,
                            handle,
                        );
                    }
                    LivePreviewResult::Fallback(path) => {
                        let visual = path.map_or_else(
                            || PreviewVisual::Color(self.wallpaper.hsva.to_color()),
                            PreviewVisual::Image,
                        );
                        self.wallpaper.live_preview.remember(key, visual);
                    }
                }
            }
            Message::PreviewLoaded(epoch, path, result) => {
                if self.wallpaper.preview_pending.as_ref() != Some(&(epoch, path.clone())) {
                    return Task::none();
                }
                self.wallpaper.preview_pending = None;
                match result {
                    Ok(handle) => {
                        return self.wallpaper.warm_preview_handle(epoch, path, handle);
                    }
                    Err(error) => {
                        self.wallpaper.preview_failed = Some(path);
                        self.error = Some(error);
                        return Task::none();
                    }
                }
            }
            Message::PreviewWarm(epoch, path) => {
                if self.wallpaper.preview_warming.as_ref() != Some(&(epoch, path)) {
                    return Task::none();
                }
                self.wallpaper.preview_warming = None;
                return self.warm_wallpaper_preview();
            }
            Message::Status(status) => {
                let wanted = serde_json::to_value(&preference).unwrap_or_default();
                let (matched, error) = match status {
                    Some((request, error)) => (request == wanted, error),
                    None => (false, None),
                };
                self.wallpaper.status = self.wallpaper.settle(matched, error, Instant::now());
            }
            _ => {}
        }
        if let Some(change) = apply {
            self.menu = None;
            let task = self.update(AppMessage::Change(Change::Wallpaper(change)));
            return Task::batch([
                task,
                self.wallpaper
                    .refresh_preview(&self.settings.wallpaper, before, live_color),
            ]);
        }
        let preview = if self.page == Page::Appearance && !self.closing {
            self.wallpaper
                .refresh_preview(&self.settings.wallpaper, before, live_color)
        } else {
            Task::none()
        };
        if self.closing
            && !self.wallpaper.pending()
            && !self.busy
            && !self.plugins.busy()
            && !self.users_pending()
        {
            return iced::exit();
        }
        preview
    }
    pub(crate) fn sync_wallpaper_drafts(&mut self) {
        let theme = self.tokens();
        if let Some(picker) = &mut self.picker {
            picker.theme = theme;
        }
        if self.settings.wallpaper.color == self.wallpaper.color {
            self.wallpaper.color_dirty = false;
        }
        if !self.wallpaper.color_dirty {
            self.wallpaper.color = self.settings.wallpaper.color.clone();
            if let Some(color) = parse_color(&self.wallpaper.color)
                && color.to_color() != self.wallpaper.hsva.to_color()
            {
                self.wallpaper.hsva = color;
            }
        }
    }
}
