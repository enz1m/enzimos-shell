use super::*;
use crate::Shell;
use enzui_shell::search::{Category, Item, Target};
use iced::window;
use std::sync::Arc;

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

#[test]
fn all_apps_keep_their_place_in_the_frame() {
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
        attachment: enzui_shell::reveal::Attachment::LeftFrame,
        frame: true,
        dim: false,
        overview: false,
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
