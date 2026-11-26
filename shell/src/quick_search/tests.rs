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
        assert!(!shell.system.available(QuickAction::GameMode));
        shell.system.game_mode = Some(enzui_shell::game_mode::Status {
            clients: 0,
            requested: false,
        });
        assert!(shell.system.available(QuickAction::GameMode));
    }
}

fn shell() -> Shell {
    let compositor =
        enzui_shell::compositor::Compositor::new("/usr/bin/true".into(), "search-fixture".into())
            .unwrap();
    Shell::new(compositor, "/".into()).0
}

/// Where the Search window's field, footer row and listed apps lie.
#[derive(Default)]
struct Places {
    found: Vec<(&'static str, iced::Rectangle)>,
}
impl iced::advanced::widget::Operation for Places {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }
    fn container(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        _: &iced::Rectangle,
    ) {
        if id == Some(&"quick-search-visible-card".into()) {
            self.found.push(("card", bounds));
        }
        // A listed app's row: as wide as the list, a row tall.
        if (36.0..=48.0).contains(&bounds.height) && bounds.width > 400.0 {
            self.found.push(("row", bounds));
        }
    }
    fn focusable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        _: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        let name = match id {
            Some(id) if *id == "quick-search-all-apps".into() => "footer",
            Some(id) if *id == "quick-search-top-apps".into() => "footer",
            _ => return,
        };
        self.found.push((name, bounds));
    }
    fn text_input(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        _: &mut dyn iced::advanced::widget::operation::TextInput,
    ) {
        if id == Some(&"quick-search-input".into()) {
            self.found.push(("field", bounds));
        }
    }
}

// The field stays at the top and the footer follows the card's bottom edge
// through every frame of opening and closing All apps.
#[test]
fn all_apps_keep_their_place() {
    all_apps_keep_their_place_in(
        enzui_shell::reveal::Attachment::Island {
            center: 40.0,
            seed_width: 60.0,
        },
        false,
    );
}

// Joined to the frame the card keeps a join below its body; closing All apps
// shrank it past that and grew it back.
#[test]
fn all_apps_keep_their_place_in_the_frame() {
    all_apps_keep_their_place_in(enzui_shell::reveal::Attachment::LeftFrame, true);
}

fn all_apps_keep_their_place_in(attachment: enzui_shell::reveal::Attachment, frame: bool) {
    use iced_runtime::user_interface::{Cache, UserInterface};
    let mut shell = shell();
    let id = window::Id::unique();
    shell.search.window = Some(id);
    shell.search.showing = true;
    shell.search.visible = true;
    shell.search.started = true;
    shell.search.presented = true;
    shell.search.configured = true;
    shell.search.apps_ready = true;
    shell.search.preferences = enzui_shell::search::preferences::Preferences::default();
    shell.search.geometry = Some((iced::Size::new(620.0, 900.0), "TEST".into()));
    shell.search.island = Some(enzui_shell::search::compositor::IslandPlacement {
        local_x: 12.0,
        bar_end: 600.0,
        side_gap: 12,
        far_gap: 12,
        bottom: false,
        attachment,
        frame,
    });
    shell.search.apps = Arc::new(
        (0..14)
            .map(|index| {
                Item::new(
                    format!("app:{index}"),
                    Category::Apps,
                    format!("App {index:02}"),
                    "Application",
                    "Open",
                    "app-window",
                    Some(Target::App(enzui_shell::search::AppTarget::Prepared(
                        format!("{index}.desktop"),
                    ))),
                )
            })
            .collect(),
    );
    shell.search_recompute();
    let _ = shell.search_sync_layout();
    shell.search.layout_busy = false;
    let mut renderer = iced::futures::executor::block_on(
        <iced::Renderer as iced::advanced::renderer::Headless>::new(
            iced::advanced::renderer::Settings::default(),
            Some("tiny-skia"),
        ),
    )
    .expect("a headless renderer");
    let mut cache = Cache::default();
    let window = shell.search.surface_size.unwrap();
    let mut frame = |shell: &Shell, window: iced::Size, cache: Cache| {
        let mut ui = UserInterface::build(shell.view(id), window, cache, &mut renderer);
        let _ = ui.update(
            &iced::window::Headless,
            &iced::advanced::shell::Waker::noop(),
            &[iced::Event::Window(iced::window::Event::RedrawRequested(
                iced::time::Instant::now(),
            ))],
            iced::mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::shell::Bus::new(),
        );
        let mut places = Places::default();
        ui.operate(&renderer, &mut places);
        let place = |name| {
            places
                .found
                .iter()
                .find(|(found, _)| *found == name)
                .map(|(_, bounds)| *bounds)
        };
        let places = (
            place("field").map(|b| b.y),
            place("footer").map(|b| b.y),
            place("card"),
            places
                .found
                .iter()
                .filter(|(found, _)| *found == "row")
                .map(|(_, bounds)| *bounds)
                .max_by(|a, b| a.y.total_cmp(&b.y)),
        );
        (ui.into_cache(), places)
    };
    let (next, (field, footer, card, _)) = frame(&shell, window, cache);
    cache = next;
    let card = card.unwrap();
    let collapsed_footer = footer.unwrap();
    let footer_inset = card.y + card.height - collapsed_footer;
    assert!(field.is_some() && footer_inset > 0.0);
    for expand in [true, false] {
        let _ = shell.search_message(Message::BrowseApps(expand));
        let _ = shell.search_sync_layout();
        let size = shell.search.surface_size.unwrap();
        // The window keeps its size: a resize let the compositor re-centre it for a frame.
        assert_eq!(size, window);
        let mut settled = None;
        let mut height = shell.search_paint_height();
        for index in 0..48 {
            let (next, places) = frame(&shell, window, cache);
            cache = next;
            assert_eq!(places.0, field, "field on frame {index}");
            let card = places.2.unwrap();
            assert!(
                (card.y + card.height - places.1.unwrap() - footer_inset).abs() < 0.5,
                "footer on frame {index} of {expand:?}: {places:?}"
            );
            settled = Some(places);
            let next = shell.search_paint_height();
            assert!(
                if expand {
                    next >= height
                } else {
                    next <= height
                },
                "frame {index}: {height} to {next}"
            );
            height = next;
            let _ = shell.search_message(Message::HeightFrame);
            std::thread::sleep(std::time::Duration::from_millis(6));
        }
        assert_eq!(shell.search.browse_all_apps, expand);
        assert!(shell.search.height_motion.is_none());
        assert_eq!(height, shell.search_surface_size().height);
        // Settled open, the last listed app shows whole inside the card.
        if expand {
            let (_, footer, _, last) = settled.unwrap();
            let (footer, last) = (footer.unwrap(), last.unwrap());
            assert!(
                footer > collapsed_footer && last.y + last.height <= footer,
                "last row {last:?} must fit above the footer at {footer}"
            );
        } else {
            assert_eq!(settled.unwrap().1, Some(collapsed_footer));
        }
    }
}
