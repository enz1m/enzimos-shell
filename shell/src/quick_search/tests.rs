use super::*;
use crate::Shell;
use enzui_shell::search::{Category, Item, Target, compositor::SearchGeometry};
use iced::window;
use std::sync::{Arc, atomic::Ordering};

fn composing_shell() -> Shell {
    let mut shell = shell();
    shell.search.showing = true;
    shell.search.window = Some(window::Id::unique());
    shell.search.model.set_query("2+3".into());
    shell.search.model.set_category(Some(Category::Calculator));
    shell.search.model.replace(vec![Item::new(
        "calculator",
        Category::Calculator,
        "5",
        "Local",
        "Copy",
        "calculator",
        Some(Target::Copy("5".into())),
    )]);
    shell
}
fn preedit(shell: &mut Shell, text: Option<&str>) {
    let event = Message::Preedit {
        window: shell.search.window.unwrap(),
        epoch: shell.search.epoch,
        alias: shell.search.alias.as_ref().map(|value| value.0.clone()),
        text: text.map(str::to_owned),
    };
    let _ = shell.search_message(event);
}

#[test]
fn quick_search_behaviour() {
    // Composition gates result activation navigation and escape.
    {
        let mut shell = composing_shell();
        preedit(&mut shell, Some("4+5"));
        assert_eq!(shell.search_surface_size().height, 520.0);
        assert_eq!(shell.search.model.query, "2+3");
        let generation = shell.search.model.generation;
        let _ = shell.search_message(Message::Navigate(1));
        let _ = shell.search_message(Message::Category(Some(Category::Apps)));
        let _ = shell.search_message(Message::Close);
        let _ = shell.search_message(Message::Key(
            shell.search.window.unwrap(),
            generation,
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
            iced::keyboard::Modifiers::default(),
            false,
        ));
        let ticket = shell
            .search
            .model
            .ticket(shell.search.model.selected_item().unwrap());
        let _ = shell.search_message(Message::Activate(ticket));
        assert!(shell.search.showing);
        assert!(!shell.search.action_busy);
        assert_eq!(shell.search.model.generation, generation);
        assert_eq!(shell.search.model.category, Some(Category::Calculator));
        preedit(&mut shell, None);
        let ticket = shell
            .search
            .model
            .ticket(shell.search.model.selected_item().unwrap());
        let _ = shell.search_message(Message::Activate(ticket));
        assert!(shell.search.action_busy);
    }
    // Closing search invalidates provider work and rejects late capabilities.
    {
        let mut shell = shell();
        shell.search.showing = true;
        shell.search.epoch = 3;
        shell.search.model.set_query("invoice".into());
        shell.search.provider_generation.store(7, Ordering::Release);
        shell.search.provider_results.push(Item::new(
            "test",
            Category::InApps,
            "Old result",
            "",
            "Open",
            "search",
            None,
        ));
        let _ = shell.close_search();
        assert!(shell.search.provider_results.is_empty());
        assert_eq!(shell.search.provider_generation.load(Ordering::Acquire), 8);
        let _ = shell.search_message(model::Message::Activation(3, None));
        assert!(!shell.search.showing);
        assert_eq!(shell.search.provider_generation.load(Ordering::Acquire), 8);
        assert!(shell.search.activation.is_none());
        assert!(shell.search.activation_tried.is_empty());
        let declaration = enzui_shell::search::providers::Declaration {
            desktop_id: "org.example.Mail.desktop".into(),
            bus_name: "org.example.Mail".into(),
            object_path: "/org/example/Mail".into(),
            protocol: enzui_shell::search::providers::Protocol::Enzui,
            min_query: 2,
        };
        shell.search.providers.push(super::providers::Provider {
            declaration,
            label: Arc::from("Mail"),
            icon: "mail".into(),
            image: None,
        });
        let found = enzui_shell::search::providers::Found {
            id: "invoice".into(),
            title: "Invoice".into(),
            subtitle: String::new(),
            icon: None,
            score: None,
        };
        let _ = shell.search_provided(
            8,
            "org.example.Mail.desktop".into(),
            "invoice".into(),
            Some(Ok(vec![found])),
        );
        assert!(shell.search.provider_results.is_empty());
    }
    // Old geometry and hide completions cannot replace a reopened window.
    {
        let mut shell = shell();
        let id = window::Id::unique();
        shell.search.window = Some(id);
        shell.search.showing = true;
        shell.search.epoch = 5;
        let _ = shell.search_message(Message::Prepared(
            4,
            Ok(SearchGeometry {
                body_max: iced::Size::new(300.0, 300.0),
                output: "TEST".into(),
                island: None,
            }),
        ));
        let _ = shell.search_message(Message::Hidden(id, 4));
        let _ = shell.search_message(Message::Parked(4, Err("old".into())));
        assert_eq!(shell.search.window, Some(id));
        assert!(shell.search.showing);
        assert!(shell.search.geometry.is_none());
    }
    // A press on the Applications button held past the outside check closes
    // Search; its click must not open Search again, while the next click does.
    //
    // Sabotage: make `search_trigger` toggle without consulting `closed_by`.
    {
        let mut shell = shell();
        shell.search.configured = true;
        shell.search.preferences.enabled = true;
        shell.search.showing = true;
        shell.search.started = true;
        let _ = shell.search_message(Message::BarPressed);
        let at = std::time::Instant::now();
        let _ = shell.search_message(Message::OutsideCheck(shell.search.epoch, at));
        assert!(!shell.search.showing);
        let _ = shell.search_trigger();
        assert!(!shell.search.showing, "the held click reopened Search");
        let _ = shell.search_message(Message::BarPressed);
        let _ = shell.search_trigger();
        assert!(shell.search.showing, "the next click did not open Search");
    }
    // Quick search lists only the Settings pages this computer shows, and the
    // Game mode tile follows GameMode: off enzimOS without NetworkManager or
    // GameMode those are not offered; on enzimOS everything is.
    //
    // Sabotage: drop the `shows` filter on the categories or on the catalog
    // entries in `search_commands`, or make the Game mode tile always available.
    {
        use enzui_shell::platform::Platform;
        use enzui_shell::settings::QuickAction;
        let mut shell = shell();
        let ids = |shell: &Shell| -> Vec<String> {
            shell
                .search_commands()
                .into_iter()
                .map(|item| item.id)
                .collect()
        };
        shell.system.platform = Some(Platform {
            pipewire: true,
            ..Platform::default()
        });
        let arch = ids(&shell);
        for hidden in [
            "setting:network",
            "setting:network/wifi",
            "setting:updates",
            "setting:updates/recovery",
            "setting:power/mode",
        ] {
            assert!(!arch.iter().any(|id| id == hidden), "{hidden}");
        }
        for shown in ["setting:appearance", "setting:sound/output_volume"] {
            assert!(arch.iter().any(|id| id == shown), "{shown}");
        }
        assert!(!shell.system.available(QuickAction::GameMode));
        shell.system.platform = Some(Platform::everything());
        let enzimos = ids(&shell);
        for shown in [
            "setting:network/wifi",
            "setting:updates/recovery",
            "setting:power/mode",
        ] {
            assert!(enzimos.iter().any(|id| id == shown), "{shown}");
        }
        assert!(shell.system.available(QuickAction::GameMode));
    }
}

fn shell() -> Shell {
    let compositor =
        enzui_shell::compositor::Compositor::new("/usr/bin/true".into(), "search-fixture".into())
            .unwrap();
    Shell::new(compositor, "/".into()).0
}
