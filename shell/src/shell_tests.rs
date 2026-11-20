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
