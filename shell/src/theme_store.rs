//! Shared-theme persistence preserving parsed studio overrides.
//! Unreadable or invalid documents disable saving to prevent data loss.

use crate::theme_presets::{self, Choices};
use enzui_core::theme_file::{MAX_THEME_FILE_BYTES, ThemeDocument, ThemePaths, save_theme_atomic};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::path::PathBuf;

/// What Settings knows about the theme file right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    /// No theme file exists yet; the built-in defaults are in effect.
    Absent,
    Loaded {
        overrides: usize,
        studio_tokens: usize,
    },
    /// A file exists but could not be used. Settings must not overwrite it.
    Unusable(String),
}

/// The component families the Buttons setting shapes together.
pub const BUTTON_SHAPES: [&str; 3] = ["shape.button", "shape.button_group", "shape.search_field"];

/// The theme file as Settings holds it between edits.
#[derive(Debug, Clone)]
pub struct Store {
    path: Option<PathBuf>,
    document: ThemeDocument,
    load: Load,
    choices: Choices,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            path: None,
            document: ThemeDocument::empty(),
            load: Load::Absent,
            choices: Choices::default(),
        }
    }
}

impl Store {
    /// Loads theme choices; unreadable or invalid files become Load::Unusable and cannot be
    /// overwritten.
    pub fn read(paths: &ThemePaths) -> Self {
        let path = paths.user_path();
        let Some(target) = path.clone() else {
            return Self::default();
        };
        let metadata = match std::fs::metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::from_load(
                    path,
                    enzui_core::theme_file::ThemeLoadOp::new(paths, 0).execute(),
                );
            }
            Err(error) => {
                return Self {
                    path,
                    load: Load::Unusable(
                        tr!("shell-could-not-read-the-theme-file-2", "arg0" => format!("{:}", error)),
                    ),
                    ..Self::default()
                };
            }
        };
        if !metadata.is_file() {
            return Self {
                path,
                load: Load::Unusable(tr("shell-the-theme-path-is-not-a-regular-file").into()),
                ..Self::default()
            };
        }
        if metadata.len() > MAX_THEME_FILE_BYTES {
            return Self {
                path,
                load: Load::Unusable(
                    tr!("shell-the-theme-file-is-bytes-past-the-byte-limit", "arg0" => format!("{:}", metadata.len()), "arg1" => format!("{:}", MAX_THEME_FILE_BYTES)),
                ),
                ..Self::default()
            };
        }
        Self::from_load(
            path,
            enzui_core::theme_file::ThemeLoadOp::new(paths, 0).execute(),
        )
    }

    fn from_load(path: Option<PathBuf>, loaded: enzui_core::theme_file::ThemeLoad) -> Self {
        match loaded.outcome {
            Ok(document) => {
                let choices = theme_presets::detect(&document);
                let load = Load::Loaded {
                    overrides: document.override_count(),
                    studio_tokens: unmanaged_count(&document),
                };
                Self {
                    path,
                    document,
                    load,
                    choices,
                }
            }
            Err(enzui_core::theme_file::ThemeLoadError::Absent) => Self {
                path,
                ..Self::default()
            },
            Err(error) => Self {
                path,
                load: Load::Unusable(
                    tr!("shell-the-theme-file-could-not-be-parsed", "arg0" => format!("{:}", error)),
                ),
                ..Self::default()
            },
        }
    }

    pub fn choices(&self) -> Choices {
        self.choices
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Whether a save would be safe: never true for a file that failed to load.
    pub fn writable(&self) -> bool {
        self.path.is_some() && !matches!(self.load, Load::Unusable(_))
    }

    /// How many overrides in the file Settings does not manage.
    pub fn studio_tokens(&self) -> usize {
        match self.load {
            Load::Loaded { studio_tokens, .. } => studio_tokens,
            _ => 0,
        }
    }

    /// Apply `choices` in memory and return the TOML that would be written.
    pub fn stage(&mut self, choices: Choices) -> String {
        self.stage_from(self.choices, choices)
    }

    /// Stages the user’s selected changes against the displayed baseline.
    pub fn stage_from(&mut self, before: Choices, choices: Choices) -> String {
        theme_presets::apply_changes(&mut self.document, before, choices);
        if choices.corners != before.corners {
            let mode = enzui_theme::EnzThemeMode::Dark;
            if choices.corners == theme_presets::Corners::Rounded {
                self.document.clear("window.radius", mode);
            } else {
                self.document.set(
                    "window.radius",
                    mode,
                    enzui_core::theme_file::registry::TokenValue::Number(f32::from(
                        choices.corners.window_radius(),
                    )),
                );
            }
        }
        self.choices = choices;
        self.document.to_toml()
    }

    /// Change shared window controls while preserving unrelated theme overrides.
    pub fn stage_window_buttons(&mut self, buttons: enzui_theme::WindowButtons) -> bool {
        use enzui_core::theme_file::registry::TokenValue;
        if buttons == enzui_theme::WindowButtons::Desktop {
            self.document
                .clear("window.buttons", enzui_theme::EnzThemeMode::Dark);
            true
        } else {
            self.document.set(
                "window.buttons",
                enzui_theme::EnzThemeMode::Dark,
                TokenValue::Enum(buttons.name().to_owned()),
            )
        }
    }

    /// Set what every application's window frame is made of; `App` (each
    /// application chooses) is the default and clears the key.
    pub fn stage_window_frame(&mut self, frame: enzui_theme::WindowFrame) -> bool {
        use enzui_core::theme_file::registry::TokenValue;
        let mode = enzui_theme::EnzThemeMode::Dark;
        if frame == enzui_theme::WindowFrame::App {
            self.document.clear("window.frame", mode);
            true
        } else {
            self.document.set(
                "window.frame",
                mode,
                TokenValue::Enum(frame.name().to_owned()),
            )
        }
    }

    /// Set the corner step of buttons, button groups and search fields;
    /// `Full` (pills) is the default and clears the keys.
    pub fn stage_button_shape(&mut self, step: enzui_theme::RadiusStep) -> bool {
        use enzui_core::theme_file::registry::TokenValue;
        let mode = enzui_theme::EnzThemeMode::Dark;
        BUTTON_SHAPES.iter().all(|path| {
            if step == enzui_theme::RadiusStep::Full {
                self.document.clear(path, mode);
                true
            } else {
                self.document
                    .set(path, mode, TokenValue::Enum(step.name().to_owned()))
            }
        })
    }

    /// Set the shared keyboard focus-ring width, restoring the default at two pixels.
    pub fn stage_focus_ring(&mut self, width: f32) -> bool {
        use enzui_core::theme_file::registry::TokenValue;
        if width == 2.0 {
            self.document
                .clear("stroke.focus_ring", enzui_theme::EnzThemeMode::Dark);
            true
        } else {
            self.document.set(
                "stroke.focus_ring",
                enzui_theme::EnzThemeMode::Dark,
                TokenValue::Number(width),
            )
        }
    }

    /// Set the window corner radius every application draws, restoring the
    /// theme's default when `radius` is it.
    pub fn stage_window_radius(&mut self, radius: u16) -> bool {
        use enzui_core::theme_file::registry::TokenValue;
        let mode = enzui_theme::EnzThemeMode::Dark;
        if f32::from(radius) == enzui_core::theme_file::default_theme(mode).window_radius {
            self.document.clear("window.radius", mode);
            true
        } else {
            self.document
                .set("window.radius", mode, TokenValue::Number(f32::from(radius)))
        }
    }

    /// Whether the shared theme asks every enzUI app to reduce motion.
    pub fn reduced_motion(&self) -> bool {
        enzui_core::theme_file::document_reduced_motion(&self.document).unwrap_or(false)
    }

    /// Ask (or stop asking) every enzUI app to reduce motion, in memory.
    pub fn stage_reduced_motion(&mut self, reduced: bool) -> bool {
        use enzui_core::theme_file::registry::TokenValue;
        if reduced {
            self.document.set(
                "motion.reduced_motion",
                enzui_theme::EnzThemeMode::Dark,
                TokenValue::Bool(true),
            )
        } else {
            self.document
                .clear("motion.reduced_motion", enzui_theme::EnzThemeMode::Dark);
            true
        }
    }

    /// Write the staged document to the user's theme path.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err(tr("shell-no-writable-configuration-directory").into());
        };
        if !self.writable() {
            return Err(
                tr("shell-the-existing-theme-file-was-not-readable-so-it-was-left-alone").into(),
            );
        }
        save_theme_atomic(path, &self.document.to_toml()).map_err(
            |error| tr!("shell-could-not-write-the-theme-file", "arg0" => format!("{:}", error)),
        )
    }

    /// The document as it currently stands, for previewing.
    pub fn document(&self) -> &ThemeDocument {
        &self.document
    }

    /// Take the parsed document, for a reader that does not edit the file.
    pub fn into_document(self) -> ThemeDocument {
        self.document
    }
}

fn unmanaged_count(document: &ThemeDocument) -> usize {
    use enzui_theme::EnzThemeMode;
    let mut managed = 0;
    for path in theme_presets::managed_paths() {
        for mode in [EnzThemeMode::Dark, EnzThemeMode::Light] {
            if document.get(path, mode).is_some() {
                managed += 1;
            }
        }
    }
    document.override_count().saturating_sub(managed)
}
