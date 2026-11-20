use super::*;
use enzui_shell::settings::{Metric, QuickAction};

#[test]
fn settings_app_behaviour() {
    // Every catalog setting has a reveal target and navigation does not edit.
    {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().join("settings.json")).unwrap();
        let mut app = SettingsApp::model(store.clone());
        let background_generation = app.generation;
        let background_snapshot = store.load().unwrap();
        store.update(Change::Metric(Metric::Bar, 47)).unwrap();
        let refresh = app.refresh_settings_read();
        let _ = app.update(Message::Loaded(
            background_generation,
            Ok(background_snapshot),
        ));
        assert!(
            app.reading,
            "a late background result cleared the window's pending read"
        );
        let tasks = enzui_agent::center::task_host::TaskHost::spawn().unwrap();
        tasks.submit(refresh).unwrap();
        let loaded = tasks
            .poll(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let _ = app.update(loaded);
        assert!(
            !app.reading,
            "window navigation remained blocked after its snapshot arrived"
        );
        assert_eq!(
            app.settings.metrics.bar, 47,
            "presentation did not load current persisted settings"
        );
        app.layout_preview = true;
        app.wallpaper.supported = true;
        app.system.platform = Some(enzui_shell::platform::Platform::everything());
        let before = app.settings.clone();
        for entry in enzui_shell::settings_catalog::ENTRIES {
            app.filter = "previous search".into();
            let _ = app.update(Message::Reveal(entry.route));
            assert_eq!(app.page, entry.category());
            assert_eq!(app.highlighted, Some(entry.route));
            assert!(app.reveal_pending);
            assert!(app.filter.is_empty());
            app.built_routes.borrow_mut().clear();
            let _ = app.settings_page();
            assert!(
                app.built_routes.borrow().contains(entry.route),
                "Missing {}",
                entry.route
            );
            assert_eq!(app.settings, before);
        }
        // A sidebar jump lands on the section's heading only where the section
        // carries an anchor; without one it lands on the first row and the
        // heading is scrolled off.
        let mut missing = Vec::new();
        for category in enzui_shell::settings_catalog::Category::ALL {
            app.page = category;
            app.built_sections.borrow_mut().clear();
            let _ = app.settings_page();
            for section in app.sections(category) {
                // These cards are drawn once the system has answered; this
                // window has no system snapshot.
                let waits = matches!(
                    section.route,
                    "phone/sharing" | "power/memory" | "updates/generations"
                ) || section.route.starts_with("backups/");
                if !waits && !app.built_sections.borrow().contains(section.route) {
                    missing.push(section.route);
                }
            }
        }
        assert!(
            missing.is_empty(),
            "Sections without an anchor: {missing:?}"
        );
    }
    // Rapid edits keep the latest value and close waits for the queue.
    {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::at(temp.path().join("settings.json")).unwrap();
        let (mut app, _) = SettingsApp::new(store.clone());
        let opening_read = app.generation;
        let _ = app.update(Message::Change(Change::Metric(Metric::Bar, 40)));
        for value in 41..=60 {
            let _ = app.update(Message::Change(Change::Metric(Metric::Bar, value)));
        }
        let _ = app.update(Message::Loaded(opening_read, store.load()));
        assert!(
            !app.reading,
            "an edit made during a read left navigation waiting for that read"
        );
        assert_eq!(app.settings.metrics.bar, 60);
        let _ = app.update(Message::Change(Change::QuickAction(
            QuickAction::Screenshot,
            false,
        )));
        assert_eq!(app.pending.len(), 2);
        assert_eq!(app.settings.metrics.bar, 60);
        assert!(!app.settings.shows(QuickAction::Screenshot));
        let _ = app.update(Message::Close);
        assert!(app.closing);
        let mut stored = Settings::default();
        stored.metrics.bar = 40;
        let _ = app.update(Message::Saved(Ok(stored.clone())));
        assert!(app.busy);
        assert_eq!(app.settings.metrics.bar, 60);
        assert_eq!(app.pending.len(), 1);
        stored.metrics.bar = 60;
        let _ = app.update(Message::Saved(Ok(stored.clone())));
        assert!(app.busy);
        assert!(app.pending.is_empty());
        stored
            .preview(Change::QuickAction(QuickAction::Screenshot, false))
            .unwrap();
        let _ = app.update(Message::Saved(Ok(stored)));
        assert!(!app.busy);
        assert!(app.closing);

        use enzui_shell::font_settings::{FontChange, Slot, Weight};
        let store = Store::at(temp.path().join("fonts.json")).unwrap();
        let mut app = SettingsApp::model(store);
        let _ = app.update(Message::Change(Change::Font(FontChange::Family(
            Slot::Interface,
            Some("First UI".into()),
        ))));
        for (slot, family) in [
            (Slot::Document, "Prose"),
            (Slot::Monospace, "Code"),
            (Slot::Interface, "Final UI"),
        ] {
            let _ = app.update(Message::Change(Change::Font(FontChange::Family(
                slot,
                Some(family.into()),
            ))));
        }
        for weight in [Weight::Heavy, Weight::Light] {
            let _ = app.update(Message::Change(Change::Font(FontChange::Weight(weight))));
        }
        assert_eq!(
            app.pending.len(),
            4,
            "different font roles and weight must survive queued edits"
        );
        assert_eq!(app.settings.fonts.interface.as_deref(), Some("Final UI"));
        assert_eq!(app.settings.fonts.document.as_deref(), Some("Prose"));
        assert_eq!(app.settings.fonts.monospace.as_deref(), Some("Code"));
        assert_eq!(app.settings.fonts.weight, Weight::Light);
    }
}
