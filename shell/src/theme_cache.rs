use enzui_core::theme_file::ThemeDocument;
use enzui_shell::chrome_style::ChromeStyle;
use enzui_shell::font_settings::Fonts;
use enzui_shell::plugins::theme::Colors;
use enzui_theme::EnzTheme;
use iced::Color;
use std::cell::RefCell;
use std::sync::Arc;

// Every input of the shell's base theme. The document is compared by
// identity, so it is replaced, never edited in place.
pub(crate) struct Inputs<'a> {
    pub document: &'a Arc<ThemeDocument>,
    pub light: bool,
    pub scheme: Option<&'a str>,
    pub chrome: ChromeStyle,
    pub fonts: &'a Fonts,
    pub text_scale: f32,
}

// The inputs a plugin's theme claim and the animation rate add on top of
// the base theme.
#[derive(Clone, Copy)]
pub(crate) struct Overlay<'a> {
    pub plugin: Option<(bool, &'a Colors)>,
    pub animation_rate: u16,
}

#[derive(Clone)]
pub(crate) struct Base {
    pub chrome: Option<Color>,
    pub theme: EnzTheme,
}

struct Key {
    document: Arc<ThemeDocument>,
    light: bool,
    scheme: Option<String>,
    chrome: ChromeStyle,
    fonts: Fonts,
    text_scale: f32,
}

impl Inputs<'_> {
    fn matches(&self, key: &Key) -> bool {
        let Self {
            document,
            light,
            scheme,
            chrome,
            fonts,
            text_scale,
        } = self;
        Arc::ptr_eq(document, &key.document)
            && *light == key.light
            && *scheme == key.scheme.as_deref()
            && *chrome == key.chrome
            && **fonts == key.fonts
            && *text_scale == key.text_scale
    }

    fn key(&self) -> Key {
        let Self {
            document,
            light,
            scheme,
            chrome,
            fonts,
            text_scale,
        } = self;
        Key {
            document: Arc::clone(document),
            light: *light,
            scheme: scheme.map(str::to_owned),
            chrome: *chrome,
            fonts: (*fonts).clone(),
            text_scale: *text_scale,
        }
    }
}

struct OverlayKey {
    plugin: Option<(bool, Colors)>,
    animation_rate: u16,
}

impl Overlay<'_> {
    fn matches(&self, key: &OverlayKey) -> bool {
        let Self {
            plugin,
            animation_rate,
        } = self;
        *animation_rate == key.animation_rate
            && match (plugin, &key.plugin) {
                (None, None) => true,
                (Some((authority, colors)), Some((kept_authority, kept_colors))) => {
                    authority == kept_authority && *colors == kept_colors
                }
                _ => false,
            }
    }

    fn key(&self) -> OverlayKey {
        OverlayKey {
            plugin: self
                .plugin
                .map(|(authority, colors)| (authority, colors.clone())),
            animation_rate: self.animation_rate,
        }
    }
}

struct Entry {
    key: Key,
    base: Base,
    tokens: Option<(OverlayKey, EnzTheme)>,
}

// The shell's last resolved theme, rebuilt only when one of its inputs
// differs from those it was built from.
#[derive(Default)]
pub(crate) struct ThemeCache(RefCell<Option<Entry>>);

impl ThemeCache {
    pub(crate) fn base(&self, inputs: &Inputs<'_>, build: impl FnOnce() -> Base) -> Base {
        if let Some(entry) = self.0.borrow().as_ref()
            && inputs.matches(&entry.key)
        {
            return entry.base.clone();
        }
        let base = build();
        *self.0.borrow_mut() = Some(Entry {
            key: inputs.key(),
            base: base.clone(),
            tokens: None,
        });
        base
    }

    pub(crate) fn chrome(
        &self,
        inputs: &Inputs<'_>,
        build: impl FnOnce() -> Base,
    ) -> Option<Color> {
        if let Some(entry) = self.0.borrow().as_ref()
            && inputs.matches(&entry.key)
        {
            return entry.base.chrome;
        }
        self.base(inputs, build).chrome
    }

    pub(crate) fn tokens(
        &self,
        inputs: &Inputs<'_>,
        overlay: Overlay<'_>,
        build: impl FnOnce() -> Base,
        finish: impl FnOnce(&EnzTheme) -> EnzTheme,
    ) -> EnzTheme {
        if let Some(entry) = self.0.borrow().as_ref()
            && inputs.matches(&entry.key)
            && let Some((key, tokens)) = &entry.tokens
            && overlay.matches(key)
        {
            return tokens.clone();
        }
        let base = self.base(inputs, build).theme;
        let tokens = finish(&base);
        if let Some(entry) = self.0.borrow_mut().as_mut() {
            entry.tokens = Some((overlay.key(), tokens.clone()));
        }
        tokens
    }
}
