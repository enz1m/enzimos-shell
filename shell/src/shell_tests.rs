use super::*;

fn controls_shell() -> Shell {
    let compositor = Compositor::new(PathBuf::from("/usr/bin/true"), "fixture".into()).unwrap();
    let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
    shell.control_center = Some(window::Id::unique());
    shell.control_started = true;
    shell.control_size = iced::Size::new(296.0, 180.0);
    shell.control_attachment = enzui_shell::reveal::Attachment::RightFrame;
    shell.control_anchor = Some(iced::Rectangle {
        x: 1350.0,
        y: 4.0,
        width: 80.0,
        height: 28.0,
    });
    shell
}
#[test]
fn expanding_controls_keeps_the_reserved_window_size() {
    use enzui_shell::{control_layout::Shape, settings::QuickAction};

    let mut shell = controls_shell();
    shell.control_visible = true;
    shell.system.network.ethernet_present = true;
    shell.system.bluetooth = Some(true);
    let layout = std::sync::Arc::make_mut(&mut shell.desired_settings.controls);
    layout.set_shape(QuickAction::Network, Shape::Triple);
    layout.set_shape(QuickAction::Bluetooth, Shape::Wide);
    let reserve = |shell: &Shell| {
        shell
            .control_content()
            .reserved_height(376.0, 44.0, 16.0, 0.0)
    };
    let expected = reserve(&shell);
    let size = shell.control_size;
    for (action, open) in [
        (QuickAction::Network, true),
        (QuickAction::Network, false),
        (QuickAction::Bluetooth, true),
        (QuickAction::Bluetooth, false),
    ] {
        let _ = shell.update_chrome(Message::ControlDetails(Some(action)));
        assert_eq!(shell.control_details_open, open);
        assert!(!shell.control_placing);
        assert_eq!(shell.control_size, size);
        assert_eq!(reserve(&shell), expected);
    }

    shell.control_details = None;
    let id = shell.control_center.unwrap();
    let key = shell.control_body_key();
    let measured = iced::Size::new(shell.control_body_width(), 300.0);
    shell.control_body = Some((key, measured));
    shell.control_body_pending = Some((id, key));
    let _ = shell.update_chrome(Message::ControlBodyMeasured(id, key, Some(measured)));
    assert!(!shell.control_placing);
}

#[test]
fn control_disclosures_switch_without_activating_the_other_tiles_primary_action() {
    use enzui_core::collect::SemanticsCollector;
    use enzui_shell::{
        control_layout,
        settings::{Change, QuickAction},
    };
    use iced::advanced::{
        layout::{Limits, Measure as _},
        renderer,
        shell::{Bus, Shell as EventShell, Waker},
        widget::{Id, Tree},
    };
    use iced::{Event, Point, Rectangle, Size, mouse};
    use std::sync::Arc;

    let mut shell = controls_shell();
    shell.control_visible = true;
    shell.system.network.ethernet_present = true;
    shell.system.bluetooth = Some(true);
    shell.system.system_writes = true;
    shell.desired_settings.quick_actions = 0;
    let mut controls = control_layout::Layout::default();
    for action in [QuickAction::Network, QuickAction::Bluetooth] {
        shell
            .desired_settings
            .preview(Change::QuickAction(action, true))
            .unwrap();
        controls.set_shape(action, control_layout::Shape::Triple);
    }
    shell.desired_settings.controls = Arc::new(controls);
    let _ = shell.update_chrome(Message::ControlDetails(Some(QuickAction::Network)));

    let renderer = iced::futures::executor::block_on(<iced::Renderer as renderer::Headless>::new(
        renderer::Settings::default(),
        Some("tiny-skia"),
    ))
    .unwrap();
    let size = Size::new(260.0, 800.0);
    let viewport = Rectangle::with_size(size);
    for target in [QuickAction::Bluetooth, QuickAction::Network] {
        let message = {
            let mut controls = control_center::controls(&shell);
            let widget = controls.as_widget_mut();
            let mut tree = Tree::new(&*widget);
            tree.diff(&mut *widget);
            let node =
                widget.measure_snapshot(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
            let layout = iced::advanced::Layout::of(&node);
            let mut semantics = SemanticsCollector::new();
            widget.operate(&mut tree, layout, &viewport, &renderer, &mut semantics);
            let control = |part| {
                let id = Id::from(format!("control-{}-{part}", target.key()));
                semantics
                    .collected()
                    .iter()
                    .find(|node| node.id.as_ref() == Some(&id))
                    .unwrap()
            };
            let disclosure = control("details");
            assert!(!disclosure.semantics.disabled && disclosure.semantics.supports_default_action);
            let primary = control("primary");
            assert!(primary.semantics.disabled && !primary.semantics.supports_default_action);

            let mut click = |point: Point, viewport: Rectangle| {
                let mut bus = Bus::new();
                let window = window::Headless;
                for event in [
                    mouse::Event::ButtonPressed(mouse::Button::Left),
                    mouse::Event::ButtonReleased(mouse::Button::Left),
                ] {
                    widget.update(
                        &mut tree,
                        &Event::Mouse(event),
                        layout,
                        mouse::Cursor::Available(point),
                        &renderer,
                        &mut EventShell::new(&window, Waker::noop(), &mut bus),
                        &viewport,
                    );
                }
                bus.drain_messages().collect::<Vec<_>>()
            };
            let center = disclosure.bounds.center();
            let clipped = Rectangle {
                width: center.x - 1.0,
                ..viewport
            };
            assert!(click(center, clipped).is_empty());
            assert!(click(primary.bounds.center(), viewport).is_empty());
            let mut messages = click(center, viewport).into_iter();
            let message = messages.next().unwrap();
            assert!(matches!(message, Message::ControlDetails(Some(action)) if action == target));
            assert!(messages.next().is_none());
            message
        };
        let _ = shell.update_chrome(message);
        assert_eq!(shell.control_details, Some(target));
        assert!(shell.control_details_open);
    }
}

#[test]
fn shell_behaviour() {
    // Panel action dispatch is once only and new intent rejects an already queued completion.
    {
        let mut shell = controls_shell();
        shell.control_visible = true;
        let id = shell.control_center.unwrap();
        let _ = shell.queue_panel_action(id, Message::ShowPower);
        let serial = shell.panel_action_serial;
        // A bar click can move native focus before its queued handoff runs.
        let _ = shell.update(Message::Window(id, window::Event::Unfocused));
        let _ = shell.update(Message::ControlsHidden(id));
        assert_eq!(shell.control_center, Some(id));
        assert!(shell.control_started);
        let _ = shell.update(Message::PanelActionReady(
            serial,
            Box::new(Message::ShowPower),
        ));
        assert!(shell.power_menu);
        let _ = shell.update(Message::PanelActionReady(
            serial,
            Box::new(Message::ShowPower),
        ));
        assert!(shell.power_menu);
        let mut shell = controls_shell();
        shell.control_visible = true;
        let id = shell.control_center.unwrap();
        let _ = shell.queue_panel_action(id, Message::ShowPower);
        let serial = shell.panel_action_serial;
        let _ = shell.update(Message::ControlsHidden(id));
        let _ = shell.close_controls_for_panel();
        let _ = shell.update(Message::PanelActionReady(
            serial,
            Box::new(Message::ShowPower),
        ));
        assert!(!shell.power_menu);
        let mut shell = controls_shell();
        shell.control_visible = true;
        let _ = shell.update(Message::SettingsPage("phone"));
        assert!(
            !shell.control_visible,
            "Settings must not open behind the quick controls"
        );
        assert!(
            shell
                .panel_action
                .as_ref()
                .is_some_and(|(_, _, action)| matches!(
                    action.as_ref(),
                    Message::SettingsPage("phone")
                ))
        );
    }
    // The tiling tile and its menu change this desktop or the default.
    {
        use enzui_core::menu_model::EntryId;
        use enzui_shell::settings::QuickAction;
        let compositor = Compositor::new(PathBuf::from("/usr/bin/true"), "fixture".into()).unwrap();
        let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
        shell.snapshot = Some(Snapshot {
            workspace: 3,
            tiling: true,
            keyboard_layout: None,
            keyboard: None,
        });
        shell.desired_settings.tiling = Some(false);
        shell.desired_settings.desktop_tiling = std::collections::BTreeMap::from([(3, true)]);
        let rows = control_center::TileMenu::rows(&shell, QuickAction::Tiling).unwrap();
        let checked = |id: &str| {
            rows.entry(&EntryId::new(id))
                .unwrap()
                .check_state()
                .is_checked()
        };
        assert!(checked("desktop-tile") && !checked("desktop-float"));
        assert!(checked("default-float") && !checked("default-tile"));
        assert!(
            rows.entry(&EntryId::new("desktop-default"))
                .unwrap()
                .is_enabled()
        );
        assert!(control_center::TileMenu::rows(&shell, QuickAction::Volume).is_none());
        let message = |shell: &Shell, id: &str| {
            control_center::TileMenu::message(shell, QuickAction::Tiling, &EntryId::new(id))
        };
        assert!(matches!(
            message(&shell, "default-tile"),
            Some(Message::TilingDefault(true))
        ));
        assert!(matches!(
            message(&shell, "desktop-float"),
            Some(Message::Tiling(false))
        ));
        assert!(matches!(
            message(&shell, "desktop-default"),
            Some(Message::TilingFollowDefault)
        ));
        assert!(message(&shell, "default-rule").is_none());
        shell.settings_save_busy = true;
        let _ = shell.update(Message::Tiling(false));
        let _ = shell.update(Message::TilingDefault(true));
        let _ = shell.update(Message::TilingFollowDefault);
        assert!(matches!(
            shell.settings_queue.front(),
            Some(SettingChange::DesktopTiling(3, Some(false)))
        ));
        assert!(matches!(
            shell.settings_queue.get(1),
            Some(SettingChange::Tiling(true))
        ));
        assert!(matches!(
            shell.settings_queue.get(2),
            Some(SettingChange::DesktopTiling(3, None))
        ));
        shell.desired_settings.desktop_tiling.clear();
        assert!(message(&shell, "desktop-default").is_none());
        let _ = shell.update(Message::ControlMenuOpen(
            QuickAction::Tiling,
            iced::Point::new(40.0, 50.0),
        ));
        assert!(shell.control_menu.is_some());
        let _ = shell.update(Message::ControlMenu(
            enzui_widgets::context_menu_host::ContextMenuEvent::Dismissed(
                enzui_core::menu_model::CloseCause::Escape,
            ),
        ));
        assert!(shell.control_menu.is_none());
        let _ = shell.update(Message::ControlMenuOpen(
            QuickAction::Volume,
            iced::Point::new(40.0, 50.0),
        ));
        assert!(shell.control_menu.is_none());
    }
    // Stale settings reads cannot replace a newer write or bar result.
    {
        let compositor = Compositor::new(PathBuf::from("/usr/bin/true"), "fixture".into()).unwrap();
        let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
        shell.chrome_styled = true;
        shell.settings_generation = 3;
        shell.desired_settings.bar = BarEdge::Bottom;
        shell.light = true;
        let _ = shell.update(Message::SettingsLoaded(2, Ok(Settings::default())));
        assert!(shell.light);
        assert_eq!(shell.desired_settings.bar, BarEdge::Bottom);
        let _ = shell.update(Message::BarApplied(
            BarEdge::Bottom,
            Metrics::default(),
            Ok(ControlGeometry {
                width: 1440.0,
                height: 36.0,
                output: "TEST".into(),
                attachment: Default::default(),
            }),
        ));
        assert_eq!(shell.bar, BarEdge::Bottom);
        assert!(shell.frame_appearance.borrow().bottom);
        shell.theme_document = enzui_core::theme_file::ThemeDocument::empty();
        let mut preferences = Settings::default();
        preferences.fonts.interface = Some("Desktop Interface".into());
        preferences.fonts.weight = enzui_shell::font_settings::Weight::Light;
        let _ = shell.update(Message::SettingsLoaded(3, Ok(preferences)));
        let _ = shell.update(Message::SettingsLoaded(2, Ok(Settings::default())));
        let theme = shell.base_tokens();
        assert_eq!(
            theme.font_catalog.family(enzui_theme::FontRole::Ui),
            &enzui_theme::FontFamily::Named("Desktop Interface".into())
        );
        assert_eq!(theme.typography.weight_regular, 300);
        // A change reported while a read runs is read again once it ends.
        shell.settings_read_busy = true;
        let _ = shell.update(Message::SettingsChanged);
        assert!(shell.settings_reread);
        let mut black = shell.desired_settings.clone();
        black.light = true;
        black.chrome.color = enzui_shell::chrome_style::ChromeColor::Custom(0x000000);
        black.chrome.opacity = 80;
        let _ = shell.update(Message::SettingsLoaded(3, Ok(black)));
        assert!(!shell.settings_reread);
        // A black shell on a light theme takes the dark palette, see-through.
        let theme = shell.base_tokens();
        assert_eq!(
            (theme.palette.surface.r, theme.palette.surface.a),
            (0.0, 0.8)
        );
        assert!(enzui_shell::chrome_style::is_light(theme.palette.text));
        let frame = *shell.frame_appearance.borrow();
        assert_eq!(
            (frame.surface, frame.light, frame.transparency),
            (Some([0, 0, 0]), false, 20)
        );
    }
    // The saved style must be in place before the startup fade-in.
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let _runtime = runtime.enter();
        let compositor = Compositor::new(PathBuf::from("/usr/bin/true"), "fixture".into()).unwrap();
        let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
        assert_eq!(shell.frame_appearance.borrow().hidden, 100);
        shell.chrome_styled = false;
        shell.update_frame();
        assert!(!shell.chrome_shown());
        assert_eq!(shell.frame_appearance.borrow().hidden, 100);
        let saved = Settings::default();
        let metrics = saved.effective_metrics();
        assert!(metrics.islands.is_some() && shell.metrics.islands.is_none());
        let generation = shell.settings_generation;
        let _ = shell.update(Message::SettingsLoaded(generation, Ok(saved)));
        assert!(!shell.chrome_shown());
        assert_eq!(shell.frame_appearance.borrow().hidden, 100);
        let _ = shell.update(Message::BarApplied(
            BarEdge::Top,
            metrics,
            Ok(ControlGeometry {
                width: 1440.0,
                height: 50.0,
                output: "TEST".into(),
                attachment: Default::default(),
            }),
        ));
        assert!(shell.frame_fade.is_some_and(|(_, out)| !out));
        shell.frame_fade = None;
        shell.update_frame();
        assert!(shell.chrome_shown());
        let frame = *shell.frame_appearance.borrow();
        assert_eq!((frame.hidden, frame.metrics.islands), (0, metrics.islands));
        // A bar placed again later (the sidebar asked for it) does not hide it.
        shell.bar_ready = false;
        assert!(shell.chrome_shown());
        shell.bar_ready = true;
        // Rescale the bar in place when the output stays the same.
        use enzui_shell::compositor::OutputChange;
        let placed = |shell: &mut Shell, width: f32| {
            let _ = shell.update(Message::BarApplied(
                BarEdge::Top,
                metrics,
                Ok(ControlGeometry {
                    width,
                    height: 50.0,
                    output: "TEST".into(),
                    attachment: Default::default(),
                }),
            ));
        };
        let _ = shell.update(Message::OutputsChecked(OutputChange::Rescaled));
        assert!(shell.bar_busy && shell.chrome_shown());
        // A second rescale while that placement runs is placed once it ends.
        let _ = shell.update(Message::OutputsChecked(OutputChange::Rescaled));
        placed(&mut shell, 1805.0);
        assert_eq!(shell.bar_size, iced::Size::new(1805.0, 50.0));
        assert!(shell.bar_busy && shell.chrome_shown());
        placed(&mut shell, 1504.0);
        assert!(!shell.bar_busy && shell.chrome_shown());
        assert_eq!(shell.bar_size, iced::Size::new(1504.0, 50.0));
        assert!(shell.frame_fade.is_none());
        assert_eq!(shell.frame_appearance.borrow().hidden, 0);
        // Before its saved style is in place, a starting shell leaves the
        // rescale to its first placement, which must not use the default style.
        shell.chrome_styled = false;
        let _ = shell.update(Message::OutputsChecked(OutputChange::Rescaled));
        assert!(!shell.bar_busy);
    }
    // Settings keeps the chrome hidden until the settled bar placement lands.
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let _runtime = runtime.enter();
        use enzui_shell::commands::Command;
        use enzui_shell::compositor::OutputChange;
        let compositor = Compositor::new(PathBuf::from("/usr/bin/true"), "fixture".into()).unwrap();
        let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
        let metrics = Settings::default().effective_metrics();
        shell.desired_settings = Settings::default();
        shell.metrics = metrics;
        shell.chrome_styled = true;
        shell.bar_ready = true;
        shell.frame_fade = None;
        assert!(shell.chrome_shown());
        let placed = |shell: &mut Shell, result: Result<f32, &str>| {
            let _ = shell.update(Message::BarApplied(
                BarEdge::Top,
                metrics,
                result
                    .map(|width| ControlGeometry {
                        width,
                        height: 50.0,
                        output: "TEST".into(),
                        attachment: Default::default(),
                    })
                    .map_err(String::from),
            ));
        };
        let generation = |shell: &Shell| shell.rescale_hold.unwrap().generation;
        let shown_after_fade = |shell: &mut Shell| {
            assert!(shell.rescale_hold.is_none());
            assert!(shell.frame_fade.is_some_and(|(_, out)| !out));
            shell.frame_fade = None;
            shell.update_frame();
            assert!(shell.chrome_shown());
            assert_eq!(shell.frame_appearance.borrow().hidden, 0);
        };

        let _ = shell.update(Message::Command(Command::OutputRescaling));
        assert!(!shell.chrome_shown());
        assert!(shell.frame_fade.is_some_and(|(_, out)| out));
        assert_eq!(shell.frame_appearance.borrow().hidden, 100);
        let first = generation(&shell);
        // While the output changes, and before its pinning, the bar is not
        // placed; it stays hidden at the old size and the new.
        let _ = shell.update(Message::OutputsChecked(OutputChange::Rescaled));
        assert!(!shell.bar_busy && !shell.chrome_shown());
        let _ = shell.update(Message::Command(Command::OutputRescaled));
        assert!(shell.rescale_hold.unwrap().applied.is_some());
        let _ = shell.update(Message::RescaleChecked(
            first,
            OutputChange::Rescaled,
            false,
        ));
        assert!(!shell.bar_busy && !shell.chrome_shown());
        // Settled: placed once more, still hidden until that placement lands.
        let _ = shell.update(Message::RescaleChecked(first, OutputChange::Rescaled, true));
        assert!(shell.bar_busy && shell.rescale_hold.unwrap().placing);
        assert!(!shell.chrome_shown());
        // A rescale while it is placed places it again before it shows.
        let _ = shell.update(Message::OutputsChecked(OutputChange::Rescaled));
        placed(&mut shell, Ok(1504.0));
        assert!(shell.bar_busy && shell.rescale_hold.is_some() && !shell.chrome_shown());
        placed(&mut shell, Ok(1805.0));
        assert_eq!(shell.bar_size, iced::Size::new(1805.0, 50.0));
        shown_after_fade(&mut shell);

        // Back to back, as when the countdown reverts or another scale is
        // picked: the newer announcement owns the hold, and the older one's
        // checks and placement do not end it.
        let _ = shell.update(Message::Command(Command::OutputRescaling));
        let older = generation(&shell);
        let _ = shell.update(Message::Command(Command::OutputRescaled));
        let _ = shell.update(Message::RescaleChecked(older, OutputChange::Rescaled, true));
        assert!(shell.bar_busy);
        let _ = shell.update(Message::Command(Command::OutputRescaling));
        let newer = generation(&shell);
        assert_ne!(older, newer);
        assert!(shell.rescale_hold.unwrap().applied.is_none());
        let _ = shell.update(Message::RescaleChecked(older, OutputChange::Rescaled, true));
        let _ = shell.update(Message::RescaleLimit(older));
        placed(&mut shell, Ok(1504.0));
        assert!(shell.rescale_hold.is_some_and(|hold| !hold.placing));
        assert!(!shell.chrome_shown());
        let _ = shell.update(Message::Command(Command::OutputRescaled));
        let _ = shell.update(Message::RescaleChecked(newer, OutputChange::Rescaled, true));
        placed(&mut shell, Ok(1504.0));
        shown_after_fade(&mut shell);

        // A change that failed, or left the scale as it was, still ends the
        // hold, and so does a placement that fails.
        let _ = shell.update(Message::Command(Command::OutputRescaling));
        let failed = generation(&shell);
        let _ = shell.update(Message::Command(Command::OutputRescaled));
        let _ = shell.update(Message::RescaleChecked(failed, OutputChange::Same, true));
        assert!(shell.bar_busy && !shell.chrome_shown());
        placed(&mut shell, Err("rejected"));
        shown_after_fade(&mut shell);

        // Settings never saying it finished (or the output never settling)
        // brings the chrome back after a bound, placed for the output.
        let _ = shell.update(Message::Command(Command::OutputRescaling));
        let lost = generation(&shell);
        let _ = shell.update(Message::RescaleLimit(lost));
        assert!(shell.bar_busy);
        shown_after_fade(&mut shell);
        placed(&mut shell, Ok(1504.0));
        assert!(shell.chrome_shown() && !shell.bar_busy);
        // A finish without an announcement changes nothing.
        let _ = shell.update(Message::Command(Command::OutputRescaled));
        assert!(shell.rescale_hold.is_none() && shell.chrome_shown());
    }
    // Dock input failure and close request remove the canvas.
    {
        for request in [false, true] {
            let mut shell = controls_shell();
            let id = window::Id::unique();
            shell.dock.window = Some(id);
            shell.desired_settings.dock.enabled = true;
            shell.pending_surface_requests.insert(id);
            let task = if request {
                shell.update_windows(Message::Window(id, window::Event::CloseRequested))
            } else {
                shell.close_input_window(id)
            };
            assert!(task.units() > 0);
            assert!(shell.dock.window.is_none());
            assert!(!shell.pending_surface_requests.contains(&id));
            assert!(!shell.desired_settings.dock.enabled);
        }
    }
}
