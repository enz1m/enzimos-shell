use super::*;
use enzui_shell::plugins::{manifest::Package, service};

pub(crate) fn activate(shell: &mut Shell, id: &str, run: u64) -> Owner {
    let bundled = applications::package().unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(bundled.bytes()).unwrap();
    value["id"] = id.into();
    let package = Package::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let owner = Owner {
        id: id.into(),
        activation: 1,
        run,
    };
    let _ = shell.update(Message::Plugin(crate::plugin_panel::Message::Event(
        service::Event::Started {
            owner: owner.clone(),
            manifest: Arc::new(package.manifest().clone()),
            authority: Arc::new(AtomicBool::new(true)),
        },
    )));
    shell.dock.request_serial += 1;
    let _ = shell.application_surface_event(Event::Loaded {
        serial: shell.dock.request_serial,
        owner: owner.clone(),
        id: 1,
        element: format!("plugin:{id}/applications"),
        result: Ok((
            package.digest(),
            3,
            applications::preferences(package.manifest(), &package.manifest().defaults()).unwrap(),
        )),
    });
    owner
}
fn shell() -> Shell {
    let compositor =
        enzui_shell::compositor::Compositor::new("/usr/bin/true".into(), "fixture".into()).unwrap();
    Shell::new(compositor, "/".into()).0
}
#[test]
fn plugin_applications_behaviour() {
    // Broker is reusable and revocation clears native state.
    {
        let mut shell = shell();
        let owner = activate(&mut shell, "third-party-launcher", 1);
        assert!(shell.dock_active());
        let _ = shell.dock_sync();
        shell.dock.window = Some(iced::window::Id::unique());
        shell
            .dock_owner()
            .unwrap()
            .authority
            .store(false, Ordering::Release);
        assert!(!shell.dock_active());
        let _ = shell.dock_sync();
        assert!(shell.dock.window.is_none());
        assert!(shell.dock.entries.is_empty());
        assert!(shell.dock.preview.preview.is_none());
        shell.application_surface_revoke(&owner);
        assert!(shell.dock.claim.is_none());
    }
    // Another plugin cannot release or cancel the current owner.
    {
        let mut shell = shell();
        let owner = activate(&mut shell, "fixture-launcher", 1);
        let serial = shell.dock.request_serial;
        let _ = shell.application_surface_request(
            Owner {
                id: "unrelated".into(),
                activation: 1,
                run: 1,
            },
            1,
            None,
            Arc::new(AtomicBool::new(true)),
        );
        assert_eq!(shell.dock.request_serial, serial);
        assert_eq!(shell.dock_owner().unwrap().owner, owner);
    }
    // Fullscreen hides passive dock and rejects late hover.
    {
        use crate::{dock_hover, dock_ui};
        use enzui_shell::{dock, search::WindowTarget};
        let mut shell = shell();
        activate(&mut shell, "fullscreen-launcher", 1);
        let _ = shell.dock_sync();
        shell.dock.preferences.visibility = dock::Visibility::Never;
        shell.dock.preferences.position = dock::Position::Bottom;
        let movie = dock::Window {
            target: WindowTarget {
                address: "0x1".into(),
                stable_id: "1".into(),
                pid: 1001,
                start: 1,
                sidebar: None,
            },
            app_id: Some("app:movie".into()),
            title: "Movie".into(),
            class: "Movie".into(),
            workspace: 1,
            output: 0,
            bounds: iced::Rectangle::with_size(iced::Size::new(1280.0, 800.0)),
            fullscreen: false,
            active: true,
            urgent: false,
            minimized: false,
        };
        shell.dock.snapshot = Some(dock::Snapshot {
            windows: vec![movie],
            screen: iced::Size::new(1280.0, 800.0),
            output: "QA".into(),
            output_id: 0,
            desktop: 1,
        });
        shell.dock_rebuild();
        assert!(!shell.dock.hidden);
        assert!(
            !dock::input_regions(
                shell.dock.geometry.as_ref().unwrap(),
                &shell.dock.preferences,
                shell.dock.hidden,
                shell.dock.fullscreen,
                None
            )
            .is_empty()
        );
        let group = shell.dock.entries[0].group.id.clone();
        let _ = shell.dock_event(dock_ui::Event::Hover(true, true));
        let _ = shell.dock_preview_event(dock_hover::Event::Enter(
            group.clone(),
            dock_ui::Origin::Dock,
        ));
        assert!(shell.dock.hovered && shell.dock.preview.hovered.is_some());
        shell.dock.snapshot.as_mut().unwrap().windows[0].fullscreen = true;
        shell.dock_rebuild();
        assert!(shell.dock.hidden && !shell.dock.hovered);
        assert!(shell.dock.preview.hovered.is_none());
        assert!(
            dock::input_regions(
                shell.dock.geometry.as_ref().unwrap(),
                &shell.dock.preferences,
                shell.dock.hidden,
                shell.dock.fullscreen,
                None
            )
            .is_empty(),
            "invisible edge sensor must not steal fullscreen input"
        );
        let _ = shell.dock_event(dock_ui::Event::Hover(true, true));
        let _ = shell.dock_preview_event(dock_hover::Event::Enter(group, dock_ui::Origin::Dock));
        assert!(shell.dock.hidden && !shell.dock.hovered);
        assert!(shell.dock.preview.hovered.is_none());
        for (output, workspace, minimized) in [(1, 1, false), (0, 2, false), (0, 1, true)] {
            let window = &mut shell.dock.snapshot.as_mut().unwrap().windows[0];
            window.output = output;
            window.workspace = workspace;
            window.minimized = minimized;
            shell.dock_rebuild();
            assert!(
                !shell.dock.hidden,
                "unrelated or minimized fullscreen must not hide dock"
            );
        }
        let window = &mut shell.dock.snapshot.as_mut().unwrap().windows[0];
        window.output = 0;
        window.workspace = 1;
        window.minimized = false;
        window.fullscreen = false;
        shell.dock_rebuild();
        assert!(!shell.dock.hidden);
    }
}
