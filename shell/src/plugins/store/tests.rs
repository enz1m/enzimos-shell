use super::super::manifest::{Package, Permission};
use super::*;
use crate::settings;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self},
    path::Path,
};

fn fixture(id: &str, version: &str) -> Package {
    Package::parse(&serde_json::to_vec(&json!({"format":1,"id":id,"name":"Fixture","version":version,"description":"Disposable package","permissions":["shell.bar"],"elements":[{"id":"status","section":"end","before":null}],"settings":[{"key":"count","label":"Count","kind":"integer","default":4,"min":1,"max":10}],"program":"print('fixture')"})).unwrap()).unwrap()
}
fn attached(root: &Path) -> (Store, settings::Store) {
    let settings = settings::Store::at(root.join("custom-settings.json")).unwrap();
    (Store::for_settings(settings.clone()).unwrap(), settings)
}

#[test]
fn store_behaviour() {
    // Stale remove cannot delete a same content reinstall.
    {
        let temp = tempfile::tempdir().unwrap();
        let (store, _) = attached(temp.path());
        let package = fixture("fixture", "1");
        let old = store
            .install(&package, None, BTreeSet::from([Permission::Bar]), false)
            .unwrap();
        let current = store
            .install(
                &package,
                Some(old.digest()),
                BTreeSet::from([Permission::Bar]),
                false,
            )
            .unwrap();
        assert_ne!(old.activation(), current.activation());
        assert!(
            store
                .remove(old.id(), old.digest(), old.activation())
                .is_err()
        );
        assert_eq!(
            store.registry().unwrap().entries[0].activation(),
            current.activation()
        );
        store
            .remove(current.id(), current.digest(), current.activation())
            .unwrap();
    }
    // Admission rejects overflow without publishing package or registry.
    {
        let temp = tempfile::tempdir().unwrap();
        let (store, settings) = attached(temp.path());
        for i in 0..8 {
            let mut value: Value =
                serde_json::from_slice(fixture(&format!("fixture-{i}"), "1").bytes()).unwrap();
            value["elements"] = Value::Array(
                (0..if i == 7 { 3 } else { 8 })
                    .map(|n| json!({"id":format!("entry-{n}"),"section":"end","before":null}))
                    .collect(),
            );
            let package = Package::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
            store
                .install(&package, None, BTreeSet::from([Permission::Bar]), false)
                .unwrap();
            store.reconcile_layout(false).unwrap();
        }
        assert_eq!(settings.load().unwrap().layout.len(), 64);
        let state = fs::read(store.root.join("state.json")).unwrap();
        let saved = fs::read(settings.path()).unwrap();
        let overflow = fixture("overflow", "1");
        assert!(
            store
                .install(&overflow, None, BTreeSet::from([Permission::Bar]), false)
                .is_err()
        );
        assert_eq!(fs::read(store.root.join("state.json")).unwrap(), state);
        assert_eq!(fs::read(settings.path()).unwrap(), saved);
        assert!(!store.package_path("overflow", overflow.digest()).exists());
        let first = store.registry().unwrap().entries[0].clone();
        store
            .remove(first.id(), first.digest(), first.activation())
            .unwrap();
        store
            .install(&overflow, None, BTreeSet::from([Permission::Bar]), false)
            .unwrap();
        assert_eq!(store.reconcile_layout(false).unwrap().layout.len(), 57);
    }
    // Install update and config are content bound and preserve other records.
    {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("plugins")).unwrap();
        let first = fixture("first", "1");
        let second = fixture("second", "1");
        let a = store
            .install(&first, None, BTreeSet::from([Permission::Bar]), false)
            .unwrap();
        store
            .install(&second, None, BTreeSet::new(), false)
            .unwrap();
        store
            .configure(a.id(), a.digest(), "count", json!(7))
            .unwrap();
        assert!(
            store
                .install(&fixture("first", "2"), None, BTreeSet::new(), true)
                .is_err()
        );
        let updated = store
            .install(
                &fixture("first", "2"),
                Some(a.digest()),
                BTreeSet::new(),
                true,
            )
            .unwrap();
        assert_eq!(updated.config()["count"], 7);
        assert_eq!(store.registry().unwrap().entries.len(), 2);
        assert!(
            store
                .configure(a.id(), a.digest(), "count", json!(8))
                .is_err()
        );
        assert_eq!(store.package(&updated).unwrap().digest(), updated.digest());
    }
    // Symlink and corrupt package inputs do not gain authority.
    {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("plugins")).unwrap();
        let package = fixture("first", "1");
        let record = store
            .install(&package, None, BTreeSet::new(), false)
            .unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"unchanged").unwrap();
        let path = store.package_path(record.id(), record.digest());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(store.package(&record).is_err());
        assert!(Store::read_package(&path).is_err());
        store
            .remove(record.id(), record.digest(), record.activation())
            .unwrap();
        assert_eq!(fs::read(outside).unwrap(), b"unchanged");
        assert!(store.registry().unwrap().entries.is_empty());
    }
    // An older bundled plugin moves to the shipped one with its grants,
    // settings and state; a newer one, or one asking for other permissions,
    // stays.
    //
    // Sabotage: return true from `older` for equal versions, or drop the
    // permissions check in `refresh_bundled`.
    {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("plugins")).unwrap();
        let grants = BTreeSet::from([Permission::Bar]);
        let old = store
            .install(&fixture("clipboard", "1.3.1"), None, grants.clone(), true)
            .unwrap();
        store
            .configure(old.id(), old.digest(), "count", json!(7))
            .unwrap();
        assert!(
            !store
                .refresh_bundled(&fixture("clipboard", "1.3.1"))
                .unwrap()
        );
        assert!(
            store
                .refresh_bundled(&fixture("clipboard", "1.3.2"))
                .unwrap()
        );
        let record = store.registry().unwrap().entries[0].clone();
        assert_ne!(record.digest(), old.digest());
        assert_eq!(store.package(&record).unwrap().manifest().version, "1.3.2");
        assert!(record.enabled);
        assert_eq!(record.grants, grants);
        assert_eq!(record.config["count"], 7);
        assert!(
            !store
                .refresh_bundled(&fixture("clipboard", "1.3.0"))
                .unwrap()
        );
        let mut wider: Value = serde_json::from_slice(fixture("clipboard", "9").bytes()).unwrap();
        wider["permissions"] = json!(["shell.bar", "shell.panels"]);
        let wider = Package::parse(&serde_json::to_vec(&wider).unwrap()).unwrap();
        assert!(!store.refresh_bundled(&wider).unwrap());
        assert!(!store.refresh_bundled(&fixture("other", "2")).unwrap());
    }
}

#[test]
fn dock_folder_pins_survive_the_registry_round_trip() {
    use super::super::applications;
    use crate::dock::Change;

    let temp = tempfile::tempdir().unwrap();
    let store = Store::at(temp.path().join("plugins")).unwrap();
    let package = applications::package().unwrap();
    let record = store
        .install(&package, None, package.manifest().permissions.clone(), true)
        .unwrap();
    let uri = crate::desktop_portal::uri(&temp.path().join("My Folder"));
    let save = |change| {
        store.configure_active(
            record.id(),
            record.digest(),
            record.activation(),
            Permission::Applications,
            |manifest, config| applications::edit(manifest, config, &change),
        )
    };

    assert!(save(Change::PinFolders(vec!["file://remote/share".into()])).is_err());
    save(Change::PinFolders(vec![uri.clone(), uri.clone()])).unwrap();
    assert_eq!(
        store.registry().unwrap().entries[0].config()["folders"],
        json!([&uri])
    );
    save(Change::UnpinFolder(uri)).unwrap();
    assert_eq!(
        store.registry().unwrap().entries[0].config()["folders"],
        json!([])
    );
}
