//! Request and reply of the screen-sharing picker that the screen-cast portal starts.
//!
//! The portal passes the request in the environment and reads one line from the
//! picker's standard output: `[SELECTION]<flags>/<source>`, where flags are `r`
//! (remember for this session) and `p` (also after a restart), and the source is
//! `screen:<output>`, `window:<handle>` or `region:<output>@<x>,<y>,<w>,<h>`.
//! Anything else, including no output, cancels the request.

use std::collections::HashMap;
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_output, wl_registry},
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

/// A window the portal can share, from `XDPH_WINDOW_SHARING_LIST`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedWindow {
    /// The low 32 bits of the portal's toplevel handle, echoed back in the reply.
    pub handle: u32,
    pub class: String,
    pub title: String,
}

/// What the portal asked the picker to offer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// The requesting app's ID, empty for host apps without one.
    pub app_id: String,
    /// Whether "Allow after restart" may be offered.
    pub offer_persist: bool,
    /// Whether "Remember for this session" starts ticked.
    pub remember_by_default: bool,
    pub windows: Vec<SharedWindow>,
}

impl Request {
    /// Reads the request from the arguments and environment the portal passes.
    pub fn new(args: &[String], env: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            app_id: env("XDPH_APP_ID").unwrap_or_default().trim().to_owned(),
            offer_persist: env("XDPH_OFFER_PERSIST").as_deref() == Some("1"),
            remember_by_default: args.iter().any(|arg| arg == "--allow-token"),
            windows: parse_window_list(&env("XDPH_WINDOW_SHARING_LIST").unwrap_or_default())
                .into_iter()
                .filter(|window| !crate::workspaces::is_shell_surface(&window.class))
                .collect(),
        }
    }
}

/// Parses `<handle>[HC>]<class>[HT>]<title>[HE>]<address>[HA>]` records.
pub fn parse_window_list(list: &str) -> Vec<SharedWindow> {
    list.split("[HA>]")
        .filter_map(|record| {
            let (handle, rest) = record.split_once("[HC>]")?;
            let (class, rest) = rest.split_once("[HT>]")?;
            let (title, _) = rest.split_once("[HE>]")?;
            Some(SharedWindow {
                handle: handle.trim().parse().ok()?,
                class: class.to_owned(),
                title: title.to_owned(),
            })
        })
        .collect()
}

/// A rectangle in an output's logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Screen(String),
    Window(u32),
    Region(String, Region),
}

impl Selection {
    /// Whether the portal can keep this source after a restart; windows cannot be.
    pub fn persistable(&self) -> bool {
        !matches!(self, Self::Window(_))
    }
}

/// The reply line, newline included, for a selection and the grant choices.
pub fn reply(selection: &Selection, remember: bool, after_restart: bool, offered: bool) -> String {
    let flags = if after_restart && offered && selection.persistable() {
        "rp"
    } else if remember || (after_restart && offered) {
        "r"
    } else {
        ""
    };
    let source = match selection {
        Selection::Screen(output) => format!("screen:{output}"),
        Selection::Window(handle) => format!("window:{handle}"),
        Selection::Region(output, r) => {
            format!("region:{output}@{},{},{},{}", r.x, r.y, r.width, r.height)
        }
    };
    format!("[SELECTION]{flags}/{source}\n")
}

/// An output as the compositor presents it to clients.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
    /// Logical position in the global layout.
    pub x: i32,
    pub y: i32,
    /// Logical size.
    pub width: i32,
    pub height: i32,
    /// Current mode in device pixels.
    pub pixel_width: i32,
    pub pixel_height: i32,
    rotated: bool,
}

impl Output {
    /// Device pixels per logical pixel, two decimals.
    pub fn scale(&self) -> f64 {
        let pixels = if self.rotated {
            self.pixel_height
        } else {
            self.pixel_width
        };
        if self.width <= 0 || pixels <= 0 {
            return 1.0;
        }
        (f64::from(pixels) / f64::from(self.width) * 100.0).round() / 100.0
    }

    pub fn full_region(&self) -> Region {
        Region {
            x: 0,
            y: 0,
            width: self.width.max(1),
            height: self.height.max(1),
        }
    }

    /// Clamps a region to this output, keeping at least one pixel.
    pub fn clamp(&self, region: Region) -> Region {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let x = region.x.clamp(0, w - 1);
        let y = region.y.clamp(0, h - 1);
        Region {
            x,
            y,
            width: region.width.clamp(1, w - x),
            height: region.height.clamp(1, h - y),
        }
    }
}

/// Parses `slurp -f "%o %x %y %w %h"` output into an output-relative region.
pub fn parse_slurp(text: &str, outputs: &[Output]) -> Option<(String, Region)> {
    let mut words = text.split_whitespace();
    let name = words.next()?;
    let mut number = || words.next()?.parse::<i32>().ok();
    let (x, y, width, height) = (number()?, number()?, number()?, number()?);
    let output = outputs.iter().find(|o| o.name == name)?;
    let region = output.clamp(Region {
        x: x - output.x,
        y: y - output.y,
        width,
        height,
    });
    Some((name.to_owned(), region))
}

#[derive(Default)]
struct Enumeration {
    manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    outputs: Vec<(wl_output::WlOutput, Output)>,
}

/// Lists the outputs on the picker's Wayland display, ordered by position.
pub fn outputs() -> Result<Vec<Output>, String> {
    let connection = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());
    let mut state = Enumeration::default();
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    if let Some(manager) = state.manager.clone() {
        for (index, (output, _)) in state.outputs.iter().enumerate() {
            manager.get_xdg_output(output, &handle, index);
        }
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    }
    let mut list: Vec<Output> = state
        .outputs
        .into_iter()
        .map(|(_, o)| o)
        .filter(|o| !o.name.is_empty())
        .collect();
    for output in &mut list {
        if output.width <= 0 {
            (output.width, output.height) = if output.rotated {
                (output.pixel_height, output.pixel_width)
            } else {
                (output.pixel_width, output.pixel_height)
            };
        }
    }
    list.sort_by_key(|o| (o.x, o.y));
    Ok(list)
}

impl Dispatch<wl_registry::WlRegistry, ()> for Enumeration {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_output" if version >= 4 => {
                    let index = state.outputs.len();
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, 4, handle, index);
                    state.outputs.push((output, Output::default()));
                }
                "zxdg_output_manager_v1" if version >= 3 => {
                    state.manager = Some(registry.bind(name, 3, handle, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for Enumeration {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, output)) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => output.name = name,
            wl_output::Event::Description { description } => output.description = description,
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                (output.pixel_width, output.pixel_height) = (width, height);
            }
            wl_output::Event::Geometry {
                transform: WEnum::Value(transform),
                ..
            } => {
                use wl_output::Transform;
                output.rotated = matches!(
                    transform,
                    Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for Enumeration {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, output)) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => (output.x, output.y) = (x, y),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                (output.width, output.height) = (width, height);
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Enumeration: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);

/// Display names for the windows' classes, from their desktop entries where present.
pub fn app_names(windows: &[SharedWindow]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for window in windows {
        names.entry(window.class.clone()).or_insert_with(|| {
            crate::search::catalogue::name_for_app_id(&window.class)
                .unwrap_or_else(|| window.class.clone())
        });
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, x: i32, width: i32, height: i32) -> Output {
        Output {
            name: name.into(),
            x,
            width,
            height,
            pixel_width: width * 2,
            pixel_height: height * 2,
            ..Default::default()
        }
    }

    #[test]
    fn reply_lines_match_the_portal_protocol() {
        let screen = Selection::Screen("DP-1".into());
        let window = Selection::Window(4_000_000_000);
        let region = Selection::Region(
            "eDP-1".into(),
            Region {
                x: 10,
                y: 20,
                width: 640,
                height: 480,
            },
        );
        assert_eq!(
            reply(&screen, false, false, true),
            "[SELECTION]/screen:DP-1\n"
        );
        assert_eq!(
            reply(&screen, true, false, true),
            "[SELECTION]r/screen:DP-1\n"
        );
        assert_eq!(
            reply(&screen, true, true, true),
            "[SELECTION]rp/screen:DP-1\n"
        );
        assert_eq!(
            reply(&region, false, true, true),
            "[SELECTION]rp/region:eDP-1@10,20,640,480\n"
        );
        assert_eq!(
            reply(&window, false, false, true),
            "[SELECTION]/window:4000000000\n"
        );
        assert_eq!(
            reply(&window, true, true, true),
            "[SELECTION]r/window:4000000000\n"
        );
    }

    #[test]
    fn persistence_needs_the_portal_offer() {
        let screen = Selection::Screen("DP-1".into());
        assert_eq!(
            reply(&screen, false, true, false),
            "[SELECTION]/screen:DP-1\n"
        );
        assert_eq!(
            reply(&screen, true, true, false),
            "[SELECTION]r/screen:DP-1\n"
        );
    }

    #[test]
    fn slurp_regions_become_output_relative_and_clamped() {
        let outputs = [
            screen("DP-1", 0, 1920, 1080),
            screen("HDMI-A-1", 1920, 1280, 720),
        ];
        assert_eq!(
            parse_slurp("HDMI-A-1 2000 100 300 200\n", &outputs),
            Some((
                "HDMI-A-1".into(),
                Region {
                    x: 80,
                    y: 100,
                    width: 300,
                    height: 200
                }
            ))
        );
        assert_eq!(
            parse_slurp("DP-1 1800 1000 500 500", &outputs),
            Some((
                "DP-1".into(),
                Region {
                    x: 1800,
                    y: 1000,
                    width: 120,
                    height: 80
                }
            ))
        );
        assert_eq!(parse_slurp("selection cancelled", &outputs), None);
        assert_eq!(parse_slurp("DP-9 0 0 10 10", &outputs), None);
    }
}
