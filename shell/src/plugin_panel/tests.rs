use super::state::message;
use super::*;
use crate::Shell;
use enzui_shell::{
    plugins::{
        manifest::Manifest,
        protocol::Snapshot,
        service::{Event, Owner},
    },
    settings::BarEdge,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn shell(path: std::path::PathBuf) -> Shell {
    let compositor =
        enzui_shell::compositor::Compositor::new("/usr/bin/true".into(), "fixture".into()).unwrap();
    let (mut shell, _) = Shell::new(compositor, "/".into());
    shell.settings_store = Some(enzui_shell::settings::Store::at(path).unwrap());
    shell
}
fn manifest() -> Arc<Manifest> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "format": 1,
        "id": "fixture",
        "name": "Fixture",
        "version": "1",
        "description": "Disposable fixture",
        "permissions": [],
        "elements": [],
        "settings": [],
        "program": "pass\n"
    }))
    .unwrap();
    Arc::new(
        enzui_shell::plugins::manifest::Package::parse(&bytes)
            .unwrap()
            .manifest()
            .clone(),
    )
}
#[test]
fn plugin_panel_behaviour() {
    // Replacement and unavailable events revoke owner leases.
    {
        let root = tempfile::tempdir().unwrap();
        let mut shell = shell(root.path().join("settings.json"));
        let first = Owner {
            id: "fixture".into(),
            activation: 1,
            run: 1,
        };
        let second = Owner {
            run: 2,
            ..first.clone()
        };
        let _ = shell.update(message(Message::Event(Event::Started {
            owner: first.clone(),
            manifest: manifest(),
            authority: Arc::new(AtomicBool::new(true)),
        })));
        let first_lease = std::sync::Arc::clone(&shell.plugin.live["fixture"].authority);
        let _ = shell.update(message(Message::Event(Event::Started {
            owner: second.clone(),
            manifest: manifest(),
            authority: Arc::new(AtomicBool::new(true)),
        })));
        assert!(!first_lease.load(Ordering::Acquire));
        let second_lease = std::sync::Arc::clone(&shell.plugin.live["fixture"].authority);
        let _ = shell.update(message(Message::Event(Event::Snapshot {
            owner: first,
            manifest: manifest(),
            snapshot: Arc::new(Snapshot {
                revision: 1,
                elements: Vec::new(),
            }),
        })));
        assert_eq!(shell.plugin.live["fixture"].owner, second);
        let _ = shell.update(message(Message::Event(Event::Unavailable(
            "fixture".into(),
        ))));
        assert!(!second_lease.load(Ordering::Acquire));
        assert!(shell.plugin.live.is_empty());
    }
    // Layout completion does not overwrite newer shell preferences.
    {
        let root = tempfile::tempdir().unwrap();
        let mut shell = shell(root.path().join("settings.json"));
        shell.settings_save_busy = true;
        shell.light = true;
        shell.desired_settings.bar = BarEdge::Bottom;
        let _ = shell.update(message(Message::LayoutSaved(Ok(()))));
        assert!(shell.light);
        assert_eq!(shell.desired_settings.bar, BarEdge::Bottom);
        assert!(shell.settings_save_busy);
    }
}
