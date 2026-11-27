use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const MAX_ELEMENTS: usize = 64;
pub const BUILTINS: [&str; 5] = [
    "builtin.applications",
    "builtin.desktops",
    "builtin.clock",
    "builtin.controls",
    "builtin.notifications",
];
/// The retired dock item: still a valid identity and placement anchor so
/// older layouts and plugins load, and removed by `without_legacy_dock`.
pub const LEGACY_DOCK: &str = "builtin.dock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Start,
    Middle,
    End,
}
impl Section {
    pub const ALL: [Self; 3] = [Self::Start, Self::Middle, Self::End];
    pub fn key(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
    pub fn parse(key: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|s| s.key() == key)
            .ok_or_else(|| tr("shell-unknown-bar-section").into())
    }
    fn index(self) -> usize {
        match self {
            Self::Start => 0,
            Self::Middle => 1,
            Self::End => 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    id: String,
    enabled: bool,
}
impl Item {
    pub fn new(id: impl AsRef<str>, enabled: bool) -> Result<Self, String> {
        let id = id.as_ref();
        validate_id(id)?;
        Ok(Self {
            id: id.to_owned(),
            enabled,
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarLayout {
    sections: [Vec<Item>; 3],
}
impl Default for BarLayout {
    fn default() -> Self {
        Self {
            sections: [
                vec![
                    Item::new(BUILTINS[0], true).expect("built-in bar identifiers are valid"),
                    Item::new(BUILTINS[1], true).expect("built-in bar identifiers are valid"),
                ],
                vec![Item::new(BUILTINS[2], true).expect("built-in bar identifiers are valid")],
                vec![Item::new(BUILTINS[3], true).expect("built-in bar identifiers are valid")],
            ],
        }
    }
}
fn validate_id(id: &str) -> Result<(), String> {
    if BUILTINS.contains(&id) || id == LEGACY_DOCK {
        return Ok(());
    }
    let slug = |s: &str, limit: usize| {
        s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && s.len() <= limit
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    };
    if id.len() <= 104
        && id
            .strip_prefix("plugin:")
            .and_then(|s| s.split_once('/'))
            .is_some_and(|(plugin, element)| slug(plugin, 64) && slug(element, 32))
    {
        Ok(())
    } else {
        Err(tr("shell-invalid-bar-element-identity").into())
    }
}
fn object_fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| tr("shell-expected-a-bar-layout-object"))?;
    if object.len() != allowed.len() || object.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(tr("shell-unexpected-or-missing-bar-layout-field").into());
    }
    Ok(())
}
impl BarLayout {
    #[must_use]
    pub fn without_legacy_dock(mut self) -> Self {
        self.forget(LEGACY_DOCK);
        self
    }

    pub fn items(&self, section: Section) -> &[Item] {
        &self.sections[section.index()]
    }
    pub fn section_of(&self, id: &str) -> Option<Section> {
        Section::ALL
            .into_iter()
            .find(|s| self.items(*s).iter().any(|i| i.id == id))
    }
    pub fn len(&self) -> usize {
        self.sections.iter().map(Vec::len).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn from_json(value: &Value) -> Result<Self, String> {
        object_fields(value, &["version", "start", "middle", "end"])?;
        if value["version"].as_u64() != Some(1) {
            return Err(tr("shell-unsupported-bar-layout-version").into());
        }
        let mut layout = Self {
            sections: std::array::from_fn(|_| Vec::new()),
        };
        let mut ids = BTreeSet::new();
        for section in Section::ALL {
            let rows = value[section.key()]
                .as_array()
                .ok_or_else(|| tr("shell-bar-section-must-be-an-array"))?;
            if rows.len() > MAX_ELEMENTS.saturating_sub(layout.len()) {
                return Err(tr("shell-too-many-bar-elements").into());
            }
            for row in rows {
                object_fields(row, &["id", "enabled"])?;
                let item = Item::new(
                    row["id"]
                        .as_str()
                        .ok_or_else(|| tr("shell-missing-bar-element-identity"))?,
                    row["enabled"]
                        .as_bool()
                        .ok_or_else(|| tr("shell-missing-bar-element-visibility"))?,
                )?;
                if !ids.insert(item.id.clone()) {
                    return Err(tr("shell-duplicate-bar-element-identity").into());
                }
                layout.sections[section.index()].push(item);
            }
        }
        Ok(layout)
    }
    pub fn to_json(&self) -> Value {
        let rows = |s| {
            self.items(s)
                .iter()
                .map(|i| json!({"id":i.id,"enabled":i.enabled}))
                .collect::<Vec<_>>()
        };
        json!({"version":1,"start":rows(Section::Start),"middle":rows(Section::Middle),"end":rows(Section::End)})
    }
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<bool, String> {
        let item = self
            .sections
            .iter_mut()
            .flatten()
            .find(|i| i.id == id)
            .ok_or_else(|| tr("shell-bar-element-is-no-longer-present"))?;
        let changed = item.enabled != enabled;
        item.enabled = enabled;
        Ok(changed)
    }
    /// Layouts lacking a notification item gain one before Control Center; custom layouts
    /// without it stay unchanged.
    #[must_use]
    pub fn with_notifications(mut self) -> Self {
        if self.section_of(BUILTINS[4]).is_none()
            && let Some(section) = self.section_of(BUILTINS[3])
        {
            let _ = self.add_default(BUILTINS[4], section, Some(BUILTINS[3]));
        }
        self
    }
    pub fn add_default(
        &mut self,
        id: &str,
        section: Section,
        before: Option<&str>,
    ) -> Result<bool, String> {
        validate_id(id)?;
        if self.section_of(id).is_some() {
            return Ok(false);
        }
        if self.len() == MAX_ELEMENTS {
            return Err(tr("shell-too-many-bar-elements").into());
        }
        let rows = &mut self.sections[section.index()];
        let index = before
            .and_then(|anchor| rows.iter().position(|i| i.id == anchor))
            .unwrap_or(rows.len());
        rows.insert(index, Item::new(id, true)?);
        Ok(true)
    }
    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.len();
        for rows in &mut self.sections {
            rows.retain(|i| i.id != id);
        }
        self.len() != before
    }
    pub fn move_before(
        &mut self,
        ids: &[&str],
        destination: Section,
        before: Option<&str>,
    ) -> Result<bool, String> {
        if ids.is_empty() || ids.len() > MAX_ELEMENTS {
            return Err(tr("shell-invalid-bar-move-selection").into());
        }
        let selected: BTreeSet<_> = ids.iter().copied().collect();
        if selected.len() != ids.len() || selected.iter().any(|id| self.section_of(id).is_none()) {
            return Err(tr("shell-stale-or-duplicate-bar-move-selection").into());
        }
        if let Some(anchor) = before {
            if self.section_of(anchor) != Some(destination) {
                return Err(tr("shell-bar-drop-anchor-is-no-longer-present").into());
            }
            if selected.contains(anchor) {
                return Ok(false);
            }
        }
        let old = self.clone();
        let moving: Vec<_> = self
            .sections
            .iter()
            .flatten()
            .filter(|i| selected.contains(i.id()))
            .cloned()
            .collect();
        for rows in &mut self.sections {
            rows.retain(|i| !selected.contains(i.id()));
        }
        let rows = &mut self.sections[destination.index()];
        let index = before
            .and_then(|id| rows.iter().position(|i| i.id == id))
            .unwrap_or(rows.len());
        rows.splice(index..index, moving);
        Ok(*self != old)
    }
    pub fn move_to_slot(
        &mut self,
        source: Section,
        destination: Section,
        ids: &[&str],
        slot: usize,
    ) -> Result<bool, String> {
        if ids.iter().any(|id| self.section_of(id) != Some(source))
            || slot > self.items(destination).len()
        {
            return Err(tr("shell-stale-bar-drop-coordinates").into());
        }
        let anchor = self.items(destination)[slot..]
            .iter()
            .find(|i| !ids.contains(&i.id()))
            .map(|i| i.id.clone());
        self.move_before(ids, destination, anchor.as_deref())
    }
}
