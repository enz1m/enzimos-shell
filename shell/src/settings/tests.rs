use std::sync::Arc;

use super::*;

#[test]
fn settings_behaviour() {
    // Invalid preferences are not silently overwritten.
    {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            path: temp.path().join("settings.json"),
        };
        for invalid in [
            "[]",
            "{",
            r#"{"light":0}"#,
            r#"{"bar":"left"}"#,
            r#"{"sidebar":null}"#,
        ] {
            std::fs::write(&store.path, invalid).unwrap();
            assert!(store.update(Change::Light(true)).is_err());
            assert_eq!(std::fs::read_to_string(&store.path).unwrap(), invalid);
        }
        std::fs::write(&store.path, " ".repeat(16385)).unwrap();
        assert!(store.load().is_err());
    }
    // Layout preferences migrate and preserve unrelated settings.
    {
        use crate::bar_layout::{BarLayout, Section};
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("settings.json")).unwrap();
        std::fs::write(store.path(), r#"{"light":true,"future":42}"#).unwrap();
        assert_eq!(
            *store.load().unwrap().layout,
            BarLayout::default().with_notifications()
        );
        let mut layout = BarLayout::default().with_notifications();
        layout
            .move_before(&["builtin.clock"], Section::End, None)
            .unwrap();
        layout.set_enabled("builtin.desktops", false).unwrap();
        layout
            .add_default(
                "plugin:clipboard/history",
                Section::End,
                Some("builtin.controls"),
            )
            .unwrap();
        store
            .update(Change::Layout {
                expected: store.load().unwrap().layout,
                value: Arc::new(layout.clone()),
            })
            .unwrap();
        store.update(Change::Metric(Metric::Bar, 48)).unwrap();
        let saved = store.load().unwrap();
        assert_eq!(*saved.layout, layout);
        assert!(saved.light);
        assert_eq!(store.document().unwrap()["future"], 42);
        let mut bad = store.document().unwrap();
        bad["bar_layout"]["version"] = serde_json::json!(99);
        let bytes = serde_json::to_vec(&bad).unwrap();
        std::fs::write(store.path(), &bytes).unwrap();
        assert!(store.update(Change::Light(false)).is_err());
        assert_eq!(std::fs::read(store.path()).unwrap(), bytes);
    }
    {
        use crate::shortcut_map::{AssistantShortcut, CustomShortcut, CustomTarget, important};
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("settings.json")).unwrap();
        std::fs::write(
            store.path(),
            r#"{"quick_search":{"shortcut":"alt_space","future":42},"compositor":{"keyboard_layouts":"us","shortcuts":{"lock":"SUPER + slash"}}}"#,
        ).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.compositor.shortcuts["search"], "ALT + SPACE");
        assert_eq!(loaded.compositor.shortcuts["shortcuts"], "");
        assert!(
            important(&loaded.compositor)
                .iter()
                .any(|binding| binding.id == "search" && binding.chord == "ALT + SPACE")
        );

        let id = "custom-a".to_owned();
        let mut custom = CustomShortcut {
            name: "Open the guide".into(),
            target: CustomTarget::Action("shortcuts".into()),
            chord: "SUPER + slash".into(),
        };
        assert!(
            store
                .update(Change::CustomShortcut(id.clone(), Some(custom.clone())))
                .is_err()
        );
        custom.chord = "SUPER + J".into();
        store
            .update(Change::CustomShortcut(id.clone(), Some(custom.clone())))
            .unwrap();
        assert_eq!(
            store.load().unwrap().compositor.custom_shortcuts[&id],
            custom
        );
        let send = AssistantShortcut::Send;
        for conflicting in ["SHIFT + Return", "SUPER + slash", "SUPER + J"] {
            assert!(
                store
                    .update(Change::AssistantShortcut(send, Some(conflicting.into())))
                    .is_err()
            );
        }
        store
            .update(Change::AssistantShortcut(
                send,
                Some("CTRL + Return".into()),
            ))
            .unwrap();
        assert_eq!(
            send.chord(&store.load().unwrap().assistant_shortcuts),
            "CTRL + Return"
        );
        store.update(Change::AssistantShortcut(send, None)).unwrap();
        assert!(store.load().unwrap().assistant_shortcuts.is_empty());

        store
            .update(Change::Shortcut("search".into(), None))
            .unwrap();
        assert!(
            !store
                .load()
                .unwrap()
                .compositor
                .shortcuts
                .contains_key("search")
        );
        assert_eq!(store.document().unwrap()["quick_search"]["future"], 42);
        store.update(Change::Shortcut(String::new(), None)).unwrap();
        let compositor = store.load().unwrap().compositor;
        assert!(compositor.shortcuts.is_empty() && compositor.custom_shortcuts.is_empty());
    }
    // Concurrent history save and clear leave no old record.
    {
        use crate::notifications::preferences::Change as NotificationChange;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("settings.json")).unwrap();
        for _ in 0..8 {
            let snapshot = history_fixture(&store);
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    barrier.wait();
                    store.save_notification_history(&snapshot, 3).unwrap();
                });
                scope.spawn(|| {
                    barrier.wait();
                    store
                        .update(Change::Notification(NotificationChange::ClearHistory))
                        .unwrap();
                });
            });
            assert!(store.read_notification_history(3).unwrap().is_empty());
            assert!(!temp.path().join("settings.json.notifications").exists());
        }
    }
    // Concurrent disjoint changes do not clobber each other.
    {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            path: temp.path().join("settings.json"),
        };
        std::thread::scope(|scope| {
            for change in [
                Change::Light(true),
                Change::Bar(BarEdge::Bottom),
                Change::Sidebar(SidebarEdge::Right),
                Change::Tiling(true),
            ] {
                let store = &store;
                scope.spawn(move || store.update(change).unwrap());
            }
        });
        assert_eq!(
            store.load().unwrap(),
            Settings {
                light: true,
                bar: BarEdge::Bottom,
                sidebar: SidebarEdge::Right,
                tiling: Some(true),
                ..Default::default()
            }
        );
    }
    {
        use QuickAction::*;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("settings.json")).unwrap();
        std::fs::write(
            store.path(),
            r#"{"control_center_layout":{"version":1,"columns":6,
                "order":["network","bluetooth","appearance","phone","tiling","volume","brightness",
                         "dnd","screenshot","lock","power","night_light","power_mode","keyboard",
                         "drives","game_mode","dictation"]}}"#,
        )
        .unwrap();
        let selected = |settings: &Settings| {
            settings
                .controls
                .order()
                .iter()
                .copied()
                .filter(|&action| settings.shows(action))
                .collect::<Vec<_>>()
        };
        let defaults = store.load().unwrap();
        assert_eq!(
            selected(&defaults),
            [
                Network, Bluetooth, Appearance, Phone, Tiling, Volume, Brightness, Dnd, Screenshot,
                NightLight, PowerMode, Keyboard, Drives, GameMode, Dictation,
            ]
        );
        store
            .update(Change::QuickAction(NightLight, false))
            .unwrap();
        let saved = store.load().unwrap();
        assert_eq!(saved.controls, defaults.controls);
        assert_eq!(
            selected(&saved),
            selected(&defaults)
                .into_iter()
                .filter(|&action| action != NightLight)
                .collect::<Vec<_>>()
        );
        std::fs::write(store.path(), r#"{"quick_actions":["network"]}"#).unwrap();
        assert_eq!(selected(&store.load().unwrap()), [Network]);
    }
    {
        use crate::font_settings::{FontChange, Fonts, Slot, Weight, migrate_legacy};
        use enzui_core::theme_file::{ThemeDocument, ThemePaths};
        use enzui_theme::{EnzThemeMode, FontFamily, FontRole};
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("settings.json")).unwrap();
        let paths = ThemePaths::new(
            Some(temp.path().to_path_buf()),
            Vec::<std::path::PathBuf>::new(),
            None::<std::path::PathBuf>,
        );
        let shared = paths.user_path().unwrap();
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        let app = shared.with_file_name("dev.enzui.Test.toml");
        let app_bytes = "version = 1\n[fonts]\nui = \"Private Font\"\n";
        std::fs::write(&app, app_bytes).unwrap();
        let old = "version = 1\n# Keep this comment\n[fonts]\nui = \"Old Interface\"\ndocument = \"Old Prose\"\nmonospace = \"Old Mono\"\n[typography]\nweight_regular = 300\nweight_medium = 400\nweight_semibold = 500\nweight_bold = 600\nweight_extrabold = 700\n[future]\nchoice = 42\n";
        std::fs::write(&shared, old).unwrap();
        std::fs::write(store.path(), r#"{"future":42}"#).unwrap();
        migrate_legacy(&store, &paths).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.fonts.interface.as_deref(), Some("Old Interface"));
        assert_eq!(loaded.fonts.document.as_deref(), Some("Old Prose"));
        assert_eq!(loaded.fonts.monospace.as_deref(), Some("Old Mono"));
        assert_eq!(loaded.fonts.weight, Weight::Light);
        let cleaned = std::fs::read_to_string(&shared).unwrap();
        let document = ThemeDocument::parse(&cleaned).unwrap();
        assert!(document.get("fonts.ui", EnzThemeMode::Dark).is_none());
        assert!(
            document
                .get("typography.weight_regular", EnzThemeMode::Dark)
                .is_none()
        );
        assert!(cleaned.contains("Keep this comment") && cleaned.contains("choice = 42"));
        assert_eq!(std::fs::read_to_string(&app).unwrap(), app_bytes);
        let mut model = loaded;
        let change = FontChange::Family(Slot::Interface, Some("New Interface".into()));
        model.preview(Change::Font(change.clone())).unwrap();
        let saved = store.update(Change::Font(change)).unwrap();
        assert_eq!(saved.fonts, model.fonts);
        assert_eq!(store.document().unwrap()["future"], 42);
        assert_eq!(std::fs::read_to_string(&shared).unwrap(), cleaned);
        let mut theme = enzui_theme::EnzTheme::dark();
        saved
            .fonts
            .apply_to_theme(&mut theme, &document, EnzThemeMode::Dark);
        assert_eq!(
            theme.font_catalog.family(FontRole::Ui),
            &FontFamily::Named("New Interface".into())
        );
        assert_eq!(theme.typography.weight_regular, 300);
        for slot in Slot::ALL {
            store
                .update(Change::Font(FontChange::Family(slot, None)))
                .unwrap();
        }
        store
            .update(Change::Font(FontChange::Weight(Weight::Regular)))
            .unwrap();
        // Old files cannot resurrect a choice explicitly reset in the new store.
        std::fs::write(&shared, old).unwrap();
        migrate_legacy(&store, &paths).unwrap();
        assert_eq!(store.load().unwrap().fonts, Fonts::default());
        assert_eq!(std::fs::read_to_string(&app).unwrap(), app_bytes);
        assert!(
            store
                .update(Change::Font(FontChange::Family(
                    Slot::Interface,
                    Some("bad\nfont".into())
                )))
                .is_err()
        );
        let before = std::fs::read(store.path()).unwrap();
        std::fs::write(&shared, "not a theme = [").unwrap();
        assert!(migrate_legacy(&store, &paths).is_err());
        assert_eq!(std::fs::read(store.path()).unwrap(), before);
        std::fs::write(store.path(), "{").unwrap();
        std::fs::write(&shared, old).unwrap();
        assert!(migrate_legacy(&store, &paths).is_err());
        assert_eq!(
            std::fs::read_to_string(&shared).unwrap(),
            old,
            "a refused settings write must leave the legacy choices intact"
        );
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), "{");
        let linked_store = Store::at(temp.path().join("linked-settings.json")).unwrap();
        let target = temp.path().join("linked-theme.toml");
        std::fs::write(&target, old).unwrap();
        std::fs::remove_file(&shared).unwrap();
        std::os::unix::fs::symlink(&target, &shared).unwrap();
        migrate_legacy(&linked_store, &paths).unwrap();
        assert_eq!(
            linked_store.load().unwrap().fonts.interface.as_deref(),
            Some("Old Interface")
        );
        assert!(
            std::fs::symlink_metadata(&shared)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), old);
        let modified = std::fs::metadata(linked_store.path())
            .unwrap()
            .modified()
            .unwrap();
        migrate_legacy(&linked_store, &paths).unwrap();
        assert_eq!(
            std::fs::metadata(linked_store.path())
                .unwrap()
                .modified()
                .unwrap(),
            modified,
            "an imported linked theme must not cause repeated settings writes"
        );
    }
}

fn history_fixture(store: &Store) -> crate::notifications::server::Snapshot {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        store.path().parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    use crate::notifications::{
        model::{Archived, CloseReason},
        preferences::Change as NotificationChange,
        server::Snapshot,
    };
    let settings = store
        .update(Change::Notification(NotificationChange::History(true)))
        .unwrap();
    let mut snapshot = Snapshot::empty(1, settings.notifications.policy.clone());
    snapshot.history.push(Archived {
        id: 1,
        principal: format!("exe:{:064x}", 1),
        source: "Fixture".into(),
        summary: "Disposable history".into(),
        body: "Fixture body".into(),
        image: None,
        received_ms: 1,
        closed_ms: 2,
        reason: CloseReason::Dismissed,
    });
    snapshot
}
