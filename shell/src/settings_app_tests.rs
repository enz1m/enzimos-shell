use super::*;
use enzui_shell::settings::{Metric, QuickAction};

#[test]
fn stale_reads_and_queued_edits_do_not_replace_current_settings() {
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
    }
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

#[test]
fn settings_pages_publish_routable_controls() {
    use enzui_core::collect::SemanticsCollector;
    use iced::advanced::{
        layout::{Limits, Measure as _},
        renderer,
        widget::Tree,
    };
    use iced::{Rectangle, Size};

    let directory = tempfile::tempdir().unwrap();
    let store = Store::at(directory.path().join("settings.json")).unwrap();
    let mut app = SettingsApp::model(store);
    app.layout_preview = true;
    app.wallpaper.supported = true;
    app.system.platform = Some(enzui_shell::platform::Platform::everything());
    let renderer = iced::futures::executor::block_on(<iced::Renderer as renderer::Headless>::new(
        renderer::Settings::default(),
        Some("tiny-skia"),
    ))
    .unwrap();
    let size = Size::new(900.0, 100_000.0);
    let mut failures = Vec::new();
    for category in enzui_shell::settings_catalog::Category::ALL {
        app.page = category;
        let mut page = app.settings_page();
        let widget = page.as_widget_mut();
        let mut tree = Tree::new(&*widget);
        tree.diff(&mut *widget);
        let node = widget.measure_snapshot(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
        let mut semantics = SemanticsCollector::new();
        widget.operate(
            &mut tree,
            iced::advanced::Layout::of(&node),
            &Rectangle::with_size(size),
            &renderer,
            &mut semantics,
        );
        let anonymous: Vec<_> = semantics
            .collected()
            .iter()
            .filter(|node| node.id.is_none() && node.semantics.advertises_action())
            .map(|node| (node.semantics.role, node.semantics.label.clone()))
            .collect();
        let duplicates = semantics.duplicate_ids();
        if !anonymous.is_empty() || !duplicates.is_empty() {
            failures.push(format!("{category:?}: {anonymous:?} {duplicates:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "Settings pages published unroutable controls:\n{}",
        failures.join("\n")
    );
}
