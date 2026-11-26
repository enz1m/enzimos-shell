use crate::bar_layout::{Item, Section};
use enzui_shell::i18n::tr;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};

pub const MAX_PACKAGE_BYTES: usize = 1024 * 1024;
pub const MAX_PROGRAM_BYTES: usize = 512 * 1024;
pub const MAX_ELEMENTS: usize = 8;
pub const MAX_SETTINGS: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    Bar,
    Panels,
    Theme,
    Layout,
    Behavior,
    ClipboardRead,
    ClipboardWrite,
    PersistentStorage,
    NotificationRead,
    Applications,
}
impl Permission {
    pub const ALL: [Self; 10] = [
        Self::Bar,
        Self::Panels,
        Self::Theme,
        Self::Layout,
        Self::Behavior,
        Self::ClipboardRead,
        Self::ClipboardWrite,
        Self::PersistentStorage,
        Self::NotificationRead,
        Self::Applications,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Bar => "shell.bar",
            Self::Panels => "shell.panels",
            Self::Theme => "shell.theme",
            Self::Layout => "shell.layout",
            Self::Behavior => "shell.behavior",
            Self::ClipboardRead => "clipboard.read",
            Self::ClipboardWrite => "clipboard.write",
            Self::PersistentStorage => "storage.persistent",
            Self::NotificationRead => "notifications.read",
            Self::Applications => "applications.surface",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Bar => tr("shell-add-bar-elements"),
            Self::Panels => tr("shell-show-plugin-panels"),
            Self::Theme => tr("shell-change-shell-appearance"),
            Self::Layout => tr("shell-change-shell-element-placement-and-visibility"),
            Self::Behavior => tr("shell-open-shell-views-after-a-user-action"),
            Self::ClipboardRead => tr("shell-read-copied-text-and-images-in-this-desktop"),
            Self::ClipboardWrite => tr("shell-restore-clipboard-content-after-a-user-action"),
            Self::PersistentStorage => tr("shell-keep-private-plugin-data-on-disk"),
            Self::NotificationRead => tr("shell-read-notifications-in-this-desktop"),
            Self::Applications => {
                tr("shell-show-application-launchers-window-controls-and-hover-previews")
            }
        }
    }
    pub(super) fn parse(value: &Value) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|p| value.as_str() == Some(p.key()))
            .ok_or_else(|| tr("shell-unsupported-plugin-permission").into())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementSpec {
    pub id: String,
    pub section: Section,
    pub before: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingKind {
    DockFolders,
    Applications {
        default: Vec<String>,
    },
    Toggle {
        default: bool,
    },
    Integer {
        default: i64,
        min: i64,
        max: i64,
    },
    Choice {
        default: String,
        choices: Vec<String>,
        labels: std::collections::BTreeMap<String, String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    pub kind: SettingKind,
}
impl SettingSpec {
    pub fn default_value(&self) -> Value {
        match &self.kind {
            SettingKind::DockFolders => serde_json::json!([]),
            SettingKind::Applications { default } => serde_json::json!(default),
            SettingKind::Toggle { default } => (*default).into(),
            SettingKind::Integer { default, .. } => (*default).into(),
            SettingKind::Choice { default, .. } => default.clone().into(),
        }
    }
    pub fn accepts(&self, value: &Value) -> bool {
        match &self.kind {
            SettingKind::DockFolders => super::applications::valid_folders(value),
            SettingKind::Applications { .. } => super::applications::valid_pins(value),
            SettingKind::Toggle { .. } => value.is_boolean(),
            SettingKind::Integer { min, max, .. } => {
                value.as_i64().is_some_and(|n| (*min..=*max).contains(&n))
            }
            SettingKind::Choice { choices, .. } => value
                .as_str()
                .is_some_and(|v| choices.iter().any(|c| c == v)),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub permissions: BTreeSet<Permission>,
    pub elements: Vec<ElementSpec>,
    pub settings: Vec<SettingSpec>,
}
impl Manifest {
    pub fn element_id(&self, local: &str) -> Option<String> {
        self.elements
            .iter()
            .any(|e| e.id == local)
            .then(|| format!("plugin:{}/{local}", self.id))
    }
    pub fn defaults(&self) -> Value {
        self.settings
            .iter()
            .map(|s| (s.key.clone(), s.default_value()))
            .collect::<serde_json::Map<_, _>>()
            .into()
    }
    pub fn validate_config(&self, value: &Value) -> Result<(), String> {
        let object = value
            .as_object()
            .ok_or_else(|| tr("shell-plugin-settings-must-be-an-object"))?;
        if object.len() != self.settings.len()
            || self
                .settings
                .iter()
                .any(|s| object.get(&s.key).is_none_or(|v| !s.accepts(v)))
        {
            return Err(tr("shell-plugin-settings-do-not-match-the-installed-schema").into());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Package {
    manifest: Manifest,
    digest: [u8; 32],
    bytes: Arc<[u8]>,
    program: Arc<str>,
}
impl Package {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PACKAGE_BYTES {
            return Err(tr("shell-plugin-package-is-too-large").into());
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| tr("shell-invalid-plugin-package-json"))?;
        fields(
            &value,
            &[
                "format",
                "id",
                "name",
                "version",
                "description",
                "permissions",
                "elements",
                "settings",
                "program",
            ],
        )?;
        if value["format"].as_u64() != Some(1) {
            return Err(tr("shell-unsupported-plugin-package-format").into());
        }
        let id = value["id"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-plugin-identity"))?;
        Item::new(format!("plugin:{id}/entry"), true)?;
        let name = label(&value["name"], 64)?;
        let version = value["version"]
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 32
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
            })
            .ok_or_else(|| tr("shell-invalid-plugin-version"))?;
        let description = label(&value["description"], 512)?;
        let mut permissions = BTreeSet::new();
        for permission in array(&value["permissions"], Permission::ALL.len())? {
            if !permissions.insert(Permission::parse(permission)?) {
                return Err(tr("shell-duplicate-plugin-permission").into());
            }
        }
        let mut elements = Vec::new();
        let mut ids = BTreeSet::new();
        for element in array(&value["elements"], MAX_ELEMENTS)? {
            fields(element, &["id", "section", "before"])?;
            let local = element["id"]
                .as_str()
                .ok_or_else(|| tr("shell-missing-plugin-element-identity"))?;
            Item::new(format!("plugin:{id}/{local}"), true)?;
            if !ids.insert(local) {
                return Err(tr("shell-duplicate-plugin-element-identity").into());
            }
            let section = Section::parse(
                element["section"]
                    .as_str()
                    .ok_or_else(|| tr("shell-missing-plugin-element-section"))?,
            )?;
            let before = if element["before"].is_null() {
                None
            } else {
                let anchor = element["before"]
                    .as_str()
                    .ok_or_else(|| tr("shell-invalid-plugin-placement-anchor"))?;
                Item::new(anchor, true)?;
                Some(anchor.to_owned())
            };
            elements.push(ElementSpec {
                id: local.to_owned(),
                section,
                before,
            });
        }
        if !elements.is_empty() && !permissions.contains(&Permission::Bar) {
            return Err(tr("shell-bar-elements-require-an-explicit-permission-declaration").into());
        }
        let mut settings = Vec::new();
        let mut keys = BTreeSet::new();
        for setting in array(&value["settings"], MAX_SETTINGS)? {
            let key = setting["key"]
                .as_str()
                .ok_or_else(|| tr("shell-missing-plugin-setting-identity"))?;
            Item::new(format!("plugin:{id}/{key}"), true)?;
            if !keys.insert(key) {
                return Err(tr("shell-duplicate-plugin-setting-identity").into());
            }
            let name = label(&setting["label"], 80)?;
            let kind = match setting["kind"].as_str() {
                Some("toggle") => {
                    fields(setting, &["key", "label", "kind", "default"])?;
                    SettingKind::Toggle {
                        default: setting["default"]
                            .as_bool()
                            .ok_or_else(|| tr("shell-invalid-plugin-toggle-default"))?,
                    }
                }
                Some("integer") => {
                    fields(setting, &["key", "label", "kind", "default", "min", "max"])?;
                    let min = setting["min"]
                        .as_i64()
                        .ok_or_else(|| tr("shell-invalid-plugin-setting-minimum"))?;
                    let max = setting["max"]
                        .as_i64()
                        .ok_or_else(|| tr("shell-invalid-plugin-setting-maximum"))?;
                    let default = setting["default"]
                        .as_i64()
                        .ok_or_else(|| tr("shell-invalid-plugin-integer-default"))?;
                    if min < -1_000_000
                        || max > 1_000_000
                        || min > max
                        || !(min..=max).contains(&default)
                    {
                        return Err(tr("shell-invalid-plugin-setting-range").into());
                    }
                    SettingKind::Integer { default, min, max }
                }
                Some("dock-folders") if id == super::applications::DOCK_PLUGIN => {
                    fields(setting, &["key", "label", "kind", "default"])?;
                    if setting["default"] != serde_json::json!([]) {
                        return Err(tr("shell-dock-invalid-folders").into());
                    }
                    SettingKind::DockFolders
                }
                Some("applications") => {
                    fields(setting, &["key", "label", "kind", "default"])?;
                    if !super::applications::valid_pins(&setting["default"]) {
                        return Err(tr("shell-invalid-application-favorites-default").into());
                    }
                    SettingKind::Applications {
                        default: serde_json::from_value(setting["default"].clone())
                            .map_err(|_| tr("shell-invalid-application-favorites"))?,
                    }
                }
                Some("choice") => {
                    if setting.get("labels").is_some() {
                        fields(
                            setting,
                            &["key", "label", "kind", "default", "choices", "labels"],
                        )?;
                    } else {
                        fields(setting, &["key", "label", "kind", "default", "choices"])?;
                    }
                    let choices: Vec<String> = array(&setting["choices"], 16)?
                        .iter()
                        .map(|v| label(v, 64))
                        .collect::<Result<_, _>>()?;
                    let default = label(&setting["default"], 64)?;
                    if choices.is_empty()
                        || choices.iter().collect::<BTreeSet<_>>().len() != choices.len()
                        || !choices.contains(&default)
                    {
                        return Err(tr("shell-invalid-plugin-choices").into());
                    }
                    let mut labels = std::collections::BTreeMap::new();
                    if let Some(value) = setting.get("labels") {
                        let object = value
                            .as_object()
                            .ok_or_else(|| tr("shell-invalid-choice-labels"))?;
                        if object.len() != choices.len() {
                            return Err(tr("shell-choice-labels-must-name-every-choice").into());
                        }
                        for choice in &choices {
                            labels.insert(
                                choice.clone(),
                                label(
                                    object
                                        .get(choice)
                                        .ok_or_else(|| tr("shell-missing-choice-label"))?,
                                    64,
                                )?,
                            );
                        }
                        if labels.values().collect::<BTreeSet<_>>().len() != choices.len() {
                            return Err(tr("shell-choice-labels-must-be-unique").into());
                        }
                    }
                    SettingKind::Choice {
                        default,
                        choices,
                        labels,
                    }
                }
                _ => {
                    return Err(tr("shell-unsupported-plugin-setting-kind").into());
                }
            };
            settings.push(SettingSpec {
                key: key.to_owned(),
                label: name,
                kind,
            });
        }
        let program = value["program"]
            .as_str()
            .filter(|p| !p.trim().is_empty() && p.len() <= MAX_PROGRAM_BYTES && !p.contains('\0'))
            .ok_or_else(|| tr("shell-invalid-plugin-entry-program"))?;
        Ok(Self {
            manifest: Manifest {
                id: id.to_owned(),
                name,
                version: version.to_owned(),
                description,
                permissions,
                elements,
                settings,
            },
            digest: Sha256::digest(bytes).into(),
            bytes: bytes.into(),
            program: program.into(),
        })
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn program(&self) -> &str {
        &self.program
    }
}
fn fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| tr("shell-expected-a-plugin-object"))?;
    if object.len() != allowed.len() || object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(tr("shell-unexpected-or-missing-plugin-field").into());
    }
    Ok(())
}
fn array(value: &Value, max: usize) -> Result<&Vec<Value>, String> {
    value
        .as_array()
        .filter(|a| a.len() <= max)
        .ok_or_else(|| tr("shell-invalid-or-excessive-plugin-array").into())
}
fn label(value: &Value, limit: usize) -> Result<String, String> {
    let text = value
        .as_str()
        .ok_or_else(|| tr("shell-missing-plugin-label"))?;
    if text.trim().is_empty()
        || text.chars().count() > limit
        || text.chars().any(|c| {
            c.is_control()
                || matches!(c as u32, 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfeff)
        })
    {
        return Err(tr("shell-invalid-plugin-label").into());
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Regenerate with ENZUI_REGENERATE_PLUGIN_SCHEMA=1 after a plugin API change.

    fn fixture() -> Value {
        json!({"format":1,"id":"fixture","name":"Fixture","version":"1.0.0","description":"Disposable plugin","permissions":["shell.bar","shell.panels"],"elements":[{"id":"status","section":"end","before":"builtin.controls"}],"settings":[{"key":"count","label":"Count","kind":"integer","default":5,"min":1,"max":10},{"key":"paused","label":"Paused","kind":"toggle","default":false}],"program":"print('fixture')"})
    }
    fn parse(value: &Value) -> Result<Package, String> {
        Package::parse(&serde_json::to_vec(value).map_err(|error| error.to_string())?)
    }
    #[test]
    fn manifest_behaviour() {
        // Bounded package keeps source identity and explicit capabilities.
        {
            let value = fixture();
            let package = parse(&value).unwrap();
            assert_eq!(
                package.manifest.element_id("status").as_deref(),
                Some("plugin:fixture/status")
            );
            assert!(package.manifest.element_id("other").is_none());
            assert!(
                !package
                    .manifest
                    .permissions
                    .contains(&Permission::ClipboardRead)
            );
            assert_eq!(package.program(), "print('fixture')");
            assert_eq!(
                package.digest,
                Package::parse(package.bytes()).unwrap().digest
            );
            let mut changed = value;
            changed["program"] = json!("print('changed')");
            assert_ne!(package.digest, parse(&changed).unwrap().digest);
        }
        // Unknown permissions fields and unsafe names fail closed.
        {
            for (key, value) in [
                ("permissions", json!(["shell.bar", "host.exec"])),
                ("id", json!("../escape")),
                ("format", json!(2)),
                ("name", json!("Hidden\u{202e}name")),
                ("program", json!("bad\u{0}")),
                ("extra", json!(true)),
            ] {
                let mut v = fixture();
                v[key] = value;
                assert!(parse(&v).is_err(), "{key}");
            }
            let mut v = fixture();
            v["permissions"] = json!(["shell.panels"]);
            assert!(parse(&v).is_err());
            v["permissions"] = json!(["shell.bar", "shell.bar"]);
            assert!(parse(&v).is_err());
        }
    }
}
