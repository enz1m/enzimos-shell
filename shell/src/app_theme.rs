//! An application's own style: its theme file, `<app-id>.toml` beside the
//! shared `theme.toml`, which layers over the shared theme in that
//! application only (see the framework's theme-file guide).
//!
//! Settings writes three things here, each of which can also be left to the
//! shared theme: the mode, the accent and the corner style. Keys a user wrote
//! by hand stay as they are.

use crate::theme_presets::{Corners, corner_tokens};
use enzui_core::app_data::AppId;
use enzui_core::theme_file::registry::TokenValue;
use enzui_core::theme_file::{ModePreference, ThemeDocument, ThemePaths, save_theme_atomic};
use enzui_theme::EnzThemeMode;
use std::path::PathBuf;

/// An application that can be styled on its own: its name and the id its
/// windows and desktop entry carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub name: &'static str,
    pub id: &'static str,
}

/// Every installed-by-default application that reads the theme file.
pub fn targets() -> Vec<Target> {
    std::iter::once(Target {
        name: "Settings",
        id: "dev.enzui.Settings",
    })
    .chain(crate::catalog::APPS.iter().filter_map(|app| {
        crate::catalog::desktop_id(app.id).map(|id| Target { name: app.name, id })
    }))
    .collect()
}

/// What an application's file decides; `None` follows the shared theme.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    pub mode: Option<ModePreference>,
    pub accent: Option<iced::Color>,
    pub corners: Option<Corners>,
}

/// One change to an application's style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Mode(Option<ModePreference>),
    Accent(Option<iced::Color>),
    Corners(Option<Corners>),
}

/// An application's theme file as Settings holds it between edits.
#[derive(Debug, Clone)]
pub struct File {
    path: Option<PathBuf>,
    document: ThemeDocument,
    /// Why the file is left alone: it exists but cannot be read.
    unusable: Option<String>,
}

impl File {
    /// The user's theme file for `app`, empty when there is none yet.
    pub fn read(paths: &ThemePaths, app: &str) -> Self {
        let path = AppId::new(app).ok().and_then(|id| paths.app_user_path(&id));
        let read = path.as_ref().map(std::fs::read_to_string);
        let (document, unusable) = match read {
            None => (ThemeDocument::empty(), None),
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                (ThemeDocument::empty(), None)
            }
            Some(Err(error)) => (ThemeDocument::empty(), Some(error.to_string())),
            Some(Ok(text)) => match ThemeDocument::parse(&text) {
                Ok(document) => (document, None),
                Err(error) => (ThemeDocument::empty(), Some(error.to_string())),
            },
        };
        Self {
            path,
            document,
            unusable,
        }
    }

    /// Why Settings will not write this file, when it will not.
    pub fn unusable(&self) -> Option<&str> {
        self.unusable.as_deref()
    }

    /// What the file decides now.
    pub fn style(&self) -> Style {
        let number = |path| match self.document.get(path, EnzThemeMode::Dark) {
            Some(TokenValue::Number(value)) => Some(*value),
            _ => None,
        };
        Style {
            mode: self
                .document
                .declares_mode()
                .then(|| self.document.mode_preference()),
            accent: match self.document.get("palette.accent", EnzThemeMode::Dark) {
                Some(TokenValue::Color(color)) => Some(*color),
                _ => None,
            },
            corners: number("radius.md").map(Corners::nearest),
        }
    }

    /// Apply `change`; false when the file is left alone or refused it.
    pub fn apply(&mut self, change: Change) -> bool {
        if self.unusable.is_some() {
            return false;
        }
        let document = &mut self.document;
        match change {
            Change::Mode(Some(mode)) => document.set_mode_preference(mode),
            Change::Mode(None) => document.clear_mode_preference(),
            Change::Accent(accent) => {
                for mode in [EnzThemeMode::Light, EnzThemeMode::Dark] {
                    match accent {
                        Some(color) => {
                            if !document.set("palette.accent", mode, TokenValue::Color(color)) {
                                return false;
                            }
                        }
                        None => {
                            let _ = document.clear("palette.accent", mode);
                        }
                    }
                }
            }
            Change::Corners(corners) => {
                for (path, value) in corner_tokens(corners.unwrap_or_default()) {
                    match corners {
                        Some(_) => {
                            if !document.set(path, EnzThemeMode::Dark, TokenValue::Number(value)) {
                                return false;
                            }
                        }
                        None => {
                            let _ = document.clear(path, EnzThemeMode::Dark);
                        }
                    }
                }
            }
        }
        true
    }

    /// Write the file. An application that decides nothing has no file, so
    /// it follows the shared theme entirely.
    ///
    /// # Errors
    ///
    /// The file could not be written or removed.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("no configuration folder".into());
        };
        if self.unusable.is_some() {
            return Err("the file was left untouched".into());
        }
        if self.document.override_count() == 0 && !self.document.declares_mode() {
            return match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(error.to_string())
                }
                _ => Ok(()),
            };
        }
        save_theme_atomic(path, &self.text()).map_err(|error| error.to_string())
    }

    /// The file's text. A file Settings starts says what it is: the
    /// framework's canonical header describes the shared theme.
    fn text(&self) -> String {
        let text = self.document.to_toml();
        if !text.starts_with("# enzUI shared theme.") {
            return text;
        }
        let body: String = text
            .lines()
            .skip_while(|line| line.starts_with('#'))
            .map(|line| format!("{line}\n"))
            .collect();
        format!(
            "# This application's own style, layered over theme.toml beside it.\n\
             # Whatever is omitted follows the shared theme.\n{body}"
        )
    }

    /// Follow the shared theme entirely again.
    ///
    /// # Errors
    ///
    /// The file could not be removed.
    pub fn reset(&mut self) -> Result<(), String> {
        self.document = ThemeDocument::empty();
        self.unusable = None;
        self.save()
    }
}
