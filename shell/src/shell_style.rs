use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellStyle {
    Frame,
    #[default]
    Islands,
}
impl ShellStyle {
    pub fn key(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Islands => "islands",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Frame => tr("shell-frame"),
            Self::Islands => tr("shell-islands"),
        }
    }
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        match value {
            None => Ok(Self::default()),
            Some(value) => match value.as_str() {
                Some("frame") => Ok(Self::Frame),
                Some("islands") => Ok(Self::Islands),
                _ => Err(tr("shell-invalid-shell-style").into()),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Islands {
    pub height: u16,
    pub gap: u16,
    pub edge_gap: u16,
    pub side_gap: u16,
    pub radius: u16,
    pub rail: u16,
    pub padding: u16,
    pub joined_bar: bool,
}
impl Default for Islands {
    fn default() -> Self {
        Self {
            height: 32,
            gap: 8,
            edge_gap: 10,
            side_gap: 12,
            radius: 22,
            rail: 40,
            padding: 16,
            joined_bar: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IslandOption {
    Height,
    Gap,
    EdgeGap,
    SideGap,
    Radius,
    Rail,
    Padding,
}
impl IslandOption {
    pub const ALL: [Self; 7] = [
        Self::Height,
        Self::Gap,
        Self::EdgeGap,
        Self::SideGap,
        Self::Radius,
        Self::Rail,
        Self::Padding,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Height => "height",
            Self::Gap => "gap",
            Self::EdgeGap => "edge_gap",
            Self::SideGap => "side_gap",
            Self::Radius => "radius",
            Self::Rail => "rail",
            Self::Padding => "padding",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Height => tr("shell-island-height"),
            Self::Gap => tr("shell-space-between-islands"),
            Self::EdgeGap => tr("shell-distance-from-top-or-bottom"),
            Self::SideGap => tr("shell-side-inset"),
            Self::Radius => tr("shell-panel-corner-radius"),
            Self::Rail => tr("shell-floating-sidebar-width"),
            Self::Padding => tr("shell-island-side-padding"),
        }
    }
    pub fn bounds(self) -> (u16, u16) {
        match self {
            Self::Height => (32, 72),
            Self::Gap => (0, 32),
            Self::EdgeGap => (4, 48),
            Self::SideGap => (4, 48),
            Self::Radius => (0, 32),
            Self::Rail => (32, 64),
            Self::Padding => (0, 32),
        }
    }
}
impl Islands {
    pub fn get(self, key: IslandOption) -> u16 {
        match key {
            IslandOption::Height => self.height,
            IslandOption::Gap => self.gap,
            IslandOption::EdgeGap => self.edge_gap,
            IslandOption::SideGap => self.side_gap,
            IslandOption::Radius => self.radius,
            IslandOption::Rail => self.rail,
            IslandOption::Padding => self.padding,
        }
    }
    pub fn set(&mut self, key: IslandOption, value: u16) -> Result<(), String> {
        let (min, max) = key.bounds();
        if !(min..=max).contains(&value) {
            return Err(
                tr!("shell-must-be-px-2", "arg0" => key.label().to_owned(), "arg1" => format!("{:}", min), "arg2" => format!("{:}", max)),
            );
        }
        match key {
            IslandOption::Height => self.height = value,
            IslandOption::Gap => self.gap = value,
            IslandOption::EdgeGap => self.edge_gap = value,
            IslandOption::SideGap => self.side_gap = value,
            IslandOption::Radius => self.radius = value,
            IslandOption::Rail => self.rail = value,
            IslandOption::Padding => self.padding = value,
        }
        Ok(())
    }
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        let mut result = Self::default();
        if let Some(value) = value {
            let object = value
                .as_object()
                .ok_or_else(|| tr("shell-invalid-islands-preferences"))?;
            if let Some(value) = object.get("joined_bar") {
                result.joined_bar = value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-connected-bar-preference"))?;
            }
            for key in IslandOption::ALL {
                if let Some(value) = object.get(key.key()) {
                    result.set(
                        key,
                        value
                            .as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .ok_or_else(|| tr("shell-invalid-islands-size"))?,
                    )?;
                }
            }
        }
        Ok(result)
    }
    pub fn update_json(value: &mut Value, key: IslandOption, setting: u16) -> Result<(), String> {
        let mut checked = Self::from_json(value.get("islands"))?;
        checked.set(key, setting)?;
        if value.get("islands").is_none() {
            value["islands"] = json!({});
        }
        value["islands"][key.key()] = setting.into();
        Ok(())
    }
    pub fn update_joined_bar(value: &mut Value, joined: bool) -> Result<(), String> {
        Self::from_json(value.get("islands"))?;
        if value.get("islands").is_none() {
            value["islands"] = json!({});
        }
        value["islands"]["joined_bar"] = joined.into();
        Ok(())
    }
    pub fn lua(self) -> String {
        let fields = IslandOption::ALL
            .into_iter()
            .map(|key| format!("{}={}", key.key(), self.get(key)))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{{fields},joined_bar={}}}", self.joined_bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn island_fields_validate_before_mutation_and_preserve_peer_extensions() {
        let mut doc = json!({"islands":{"future":true},"frame_width":7});
        for key in IslandOption::ALL {
            let (min, max) = key.bounds();
            for value in [min, max] {
                Islands::update_json(&mut doc, key, value).unwrap();
                assert_eq!(
                    Islands::from_json(doc.get("islands")).unwrap().get(key),
                    value
                );
                assert_eq!(doc["islands"]["future"], true);
                assert_eq!(doc["frame_width"], 7);
            }
            let before = doc.clone();
            assert!(Islands::update_json(&mut doc, key, max + 1).is_err());
            assert_eq!(doc, before);
        }
        for bad in [
            json!([]),
            json!({"gap":-1}),
            json!({"height":3}),
            json!({"edge_gap":true}),
            json!({"padding":2.5}),
        ] {
            assert!(Islands::from_json(Some(&bad)).is_err());
        }
    }
}
