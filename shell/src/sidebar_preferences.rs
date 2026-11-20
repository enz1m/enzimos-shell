use enzui_shell::i18n::tr;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PinScope {
    #[default]
    Shared,
    Desktop,
}
impl PinScope {
    pub const ALL: [Self; 2] = [Self::Shared, Self::Desktop];
    pub fn key(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Desktop => "desktop",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Shared => tr("shell-shared-across-desktops"),
            Self::Desktop => tr("shell-only-on-the-desktop-where-pinned"),
        }
    }
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        match value {
            None => Ok(Self::default()),
            Some(value) => Self::ALL
                .into_iter()
                .find(|scope| value.as_str() == Some(scope.key()))
                .ok_or_else(|| tr("shell-invalid-sidebar-pin-scope").into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Fullscreen,
    Overlap,
    Reserved,
}
impl Visibility {
    pub const ALL: [Self; 3] = [Self::Fullscreen, Self::Overlap, Self::Reserved];
    pub fn key(self) -> &'static str {
        match self {
            Self::Fullscreen => "fullscreen",
            Self::Overlap => "overlap",
            Self::Reserved => "reserved",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Fullscreen => tr("shell-hide-for-fullscreen-apps"),
            Self::Overlap => tr("shell-hide-when-overlapped"),
            Self::Reserved => tr("shell-always-show-and-reserve-space"),
        }
    }
    fn parse(value: &Value) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|mode| value.as_str() == Some(mode.key()))
            .ok_or_else(|| tr("shell-invalid-sidebar-visibility-mode").into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub visibility: Visibility,
    pub fullscreen_padding: u16,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            visibility: Visibility::Fullscreen,
            fullscreen_padding: 8,
        }
    }
}
impl Preferences {
    pub const MAX_PADDING: u16 = 32;
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        let mut result = Self::default();
        if let Some(value) = value {
            let fields = value
                .as_object()
                .ok_or_else(|| tr("shell-invalid-sidebar-preferences"))?;
            if let Some(mode) = fields.get("visibility") {
                result.visibility = Visibility::parse(mode)?;
            }
            if let Some(padding) = fields.get("fullscreen_padding") {
                result.fullscreen_padding = padding
                    .as_u64()
                    .filter(|n| *n <= u64::from(Self::MAX_PADDING))
                    .ok_or_else(|| tr("shell-fullscreen-padding-must-be-between-0-and-32-pixels"))?
                    as u16;
            }
        }
        Ok(result)
    }
    pub fn set_padding(&mut self, value: u16) -> Result<(), String> {
        if value > Self::MAX_PADDING {
            return Err(tr("shell-fullscreen-padding-must-be-between-0-and-32-pixels").into());
        }
        self.fullscreen_padding = value;
        Ok(())
    }
    pub fn write_json(self, document: &mut Value) -> Result<(), String> {
        if self.fullscreen_padding > Self::MAX_PADDING {
            return Err(tr("shell-invalid-fullscreen-padding").into());
        }
        let root = document
            .as_object_mut()
            .ok_or_else(|| tr("shell-invalid-settings-document"))?;
        let value = root.entry("sidebar_behavior").or_insert_with(|| json!({}));
        let fields = value
            .as_object_mut()
            .ok_or_else(|| tr("shell-invalid-sidebar-preferences"))?;
        fields.insert("visibility".into(), self.visibility.key().into());
        fields.insert("fullscreen_padding".into(), self.fullscreen_padding.into());
        Ok(())
    }
    pub fn lua(self) -> Result<String, String> {
        if self.fullscreen_padding > Self::MAX_PADDING {
            return Err(tr("shell-invalid-fullscreen-padding").into());
        }
        Ok(format!(
            "{{visibility=\"{}\",fullscreen_padding={}}}",
            self.visibility.key(),
            self.fullscreen_padding
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_preserve_extensions_and_unrelated_preferences() {
        let mut document =
            json!({"shell_style":"frame","sidebar_behavior":{"peer":true},"icon_size":18});
        Preferences::default().write_json(&mut document).unwrap();
        assert_eq!(document["shell_style"], "frame");
        assert_eq!(document["icon_size"], 18);
        assert_eq!(document["sidebar_behavior"]["peer"], true);
        assert_eq!(document["sidebar_behavior"]["fullscreen_padding"], 8);
        assert!(Preferences::default().write_json(&mut Value::Null).is_err());
    }
}
