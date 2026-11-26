use super::manifest::{Manifest, Package, SettingKind};
use crate::dock::{Change, Preferences};
use enzui_shell::i18n::tr;
use serde_json::Value;
use std::collections::BTreeSet;

pub const DOCK_PLUGIN: &str = "app-dock";
pub fn package() -> Result<Package, String> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../examples/plugins/app-dock/manifest.json"
    ))
    .map_err(|_| tr("shell-invalid-app-dock-manifest"))?;
    value["program"] = include_str!("../../../examples/plugins/app-dock/main.py").into();
    Package::parse(&serde_json::to_vec(&value).map_err(|_| tr("shell-invalid-app-dock-package"))?)
}
pub fn valid_pins(value: &Value) -> bool {
    value.as_array().is_some_and(|pins| {
        pins.len() <= 32
            && pins
                .iter()
                .all(|id| id.as_str().is_some_and(crate::dock::valid_app_id))
            && pins
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                .len()
                == pins.len()
    })
}
pub(super) fn valid_folders(value: &Value) -> bool {
    value.as_array().is_some_and(|uris| {
        let values = uris.iter().filter_map(Value::as_str);
        values.clone().count() == uris.len() && crate::dock::valid_folders(values)
    })
}
pub fn preferences(manifest: &Manifest, config: &Value) -> Result<Preferences, String> {
    manifest.validate_config(config)?;
    let fields = [
        "visible",
        "position",
        "style",
        "visibility",
        "icon_size",
        "spacing",
        "radius",
        "highlight_radius",
        "margin",
        "show_running",
        "current_desktop",
        "pins",
    ];
    for key in fields {
        let spec = manifest
            .settings
            .iter()
            .find(|s| s.key == key)
            .ok_or_else(|| {
                tr("shell-application-surfaces-need-the-declared-dock-configuration-fields")
            })?;
        let kind = match key {
            "pins" => matches!(spec.kind, SettingKind::Applications { .. }),
            "visible" | "show_running" | "current_desktop" => {
                matches!(spec.kind, SettingKind::Toggle { .. })
            }
            "position" | "style" | "visibility" => matches!(spec.kind, SettingKind::Choice { .. }),
            _ => matches!(spec.kind, SettingKind::Integer { .. }),
        };
        if !kind {
            return Err(tr("shell-application-surface-setting-has-the-wrong-type").into());
        }
    }
    let mut value = config.clone();
    value
        .as_object_mut()
        .ok_or_else(|| tr("shell-invalid-plugin-configuration"))?
        .remove("enabled");
    value["enabled"] = value["visible"].clone();
    if value.get("hover_previews").is_none() {
        value["hover_previews"] = true.into();
    }
    Preferences::from_json(Some(&value))
}
pub fn edit(manifest: &Manifest, config: &Value, change: &Change) -> Result<Value, String> {
    let mut preferences = preferences(manifest, config)?;
    preferences.apply(change)?;
    let mut result = config.clone();
    let fields =
        serde_json::to_value(preferences).map_err(|_| tr("shell-invalid-dock-configuration"))?;
    for (key, value) in fields
        .as_object()
        .ok_or_else(|| tr("shell-invalid-serialized-application-preferences"))?
    {
        let key = if key == "enabled" { "visible" } else { key };
        if manifest.settings.iter().any(|spec| spec.key == key) {
            result[key] = value.clone();
        } else if key == "folders" && value != &serde_json::json!([]) {
            return Err(tr("shell-dock-invalid-folders").into());
        }
    }
    manifest.validate_config(&result)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_configuration_is_typed_and_pinning_preserves_other_plugin_options() {
        let package = package().unwrap();
        let manifest = package.manifest();
        let config = manifest.defaults();
        assert_eq!(preferences(manifest, &config).unwrap().icon_size, 24);
        let config = edit(
            manifest,
            &config,
            &Change::Pin("app:desktop/editor.desktop".into(), true),
        )
        .unwrap();
        assert_eq!(
            config["pins"],
            serde_json::json!(["app:desktop/editor.desktop"])
        );
        assert_eq!(config["visible"], true);
        assert!(edit(manifest, &config, &Change::IconSize(11)).is_err());
        assert!(!valid_pins(&serde_json::json!(["app:a", "app:a"])));
        assert!(!valid_pins(&serde_json::json!([
            "app:desktop/../a.desktop"
        ])));
    }
}
#[cfg(test)]
mod schema_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn applications_behaviour() {
        // Broker edits preserve extra typed settings and enforce complete schema.
        {
            let bundled = package().unwrap();
            let mut value: Value = serde_json::from_slice(bundled.bytes()).unwrap();
            value["settings"]
                .as_array_mut()
                .unwrap()
                .push(json!({"key":"extra","label":"Extra option","kind":"toggle","default":true}));
            let package = Package::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
            let config = edit(
                package.manifest(),
                &package.manifest().defaults(),
                &Change::IconSize(16),
            )
            .unwrap();
            assert_eq!(config["extra"], true);
            assert_eq!(config["icon_size"], 16);
            let mut incomplete = config;
            incomplete.as_object_mut().unwrap().remove("pins");
            assert!(preferences(package.manifest(), &incomplete).is_err());
        }
        // Choice labels are complete unique and do not replace stored values.
        {
            let bundled = package().unwrap();
            let mut value: Value = serde_json::from_slice(bundled.bytes()).unwrap();
            assert_eq!(bundled.manifest().defaults()["position"], "bottom");
            value["settings"][1]["labels"] =
                json!({"top":"Edge","bottom":"Edge","left":"Left","right":"Right"});
            assert!(Package::parse(&serde_json::to_vec(&value).unwrap()).is_err());
            value["settings"][1]["labels"] = json!({"top":"Top"});
            assert!(Package::parse(&serde_json::to_vec(&value).unwrap()).is_err());
            assert!(!valid_pins(&json!(
                (0..33).map(|i| format!("app:app-{i}")).collect::<Vec<_>>()
            )));
        }
    }
}
