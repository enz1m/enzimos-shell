use enzui_shell::i18n::tr;
mod category;
mod entries;

pub use category::{Category, Folder, Group};
pub use entries::ENTRIES;

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub category: Category,
    pub route: &'static str,
    title_key: &'static str,
    section_key: &'static str,
    keywords_key: &'static str,
}
impl Entry {
    pub fn title(self) -> &'static str {
        tr(self.title_key)
    }
    pub fn section(self) -> &'static str {
        tr(self.section_key)
    }
    fn english_search_text(self) -> String {
        format!(
            "{} {} {}",
            crate::i18n::english(self.title_key),
            crate::i18n::english(self.section_key),
            crate::i18n::english(self.keywords_key)
        )
    }
    pub fn keywords(self) -> &'static str {
        tr(self.keywords_key)
    }

    /// Persisted route prefixes remain stable when settings move between categories.
    pub fn page(self) -> &'static str {
        self.route
            .split_once('/')
            .expect("catalog routes contain a category separator")
            .0
    }
    pub fn category(self) -> Category {
        self.category
    }
    pub fn widget_id(self) -> String {
        format!("setting-{}", self.route)
    }
}
/// Maps legacy whole-page prefixes to categories; individual routes resolve independently.
pub fn category(page: &str) -> Option<Category> {
    Some(match page {
        "appearance" | "sizing" | "wallpaper" | "fonts" => Category::Appearance,
        "shell" | "bar-layout" | "controls" | "dock" => Category::Shell,
        "windows" => Category::Windows,
        "input" => Category::Input,
        "network" => Category::Network,
        "bluetooth" => Category::Bluetooth,
        "phone" => Category::Phone,
        "remote" => Category::Remote,
        "sound" => Category::Sound,
        "power" => Category::Power,
        "keyboard" => Category::Keyboard,
        "controllers" => Category::Controllers,
        "accessibility" => Category::Accessibility,
        "printers" => Category::Printers,
        "thunderbolt" => Category::Thunderbolt,
        "disks" => Category::Disks,
        "datetime" => Category::DateTime,
        "developer" => Category::Developer,
        "updates" => Category::Updates,
        "backups" => Category::Backups,
        "about" => Category::About,
        "agents" => Category::Agents,
        "apps" => Category::Apps,
        "users" => Category::Users,
        "quick-search" => Category::Search,
        "notifications" => Category::Notifications,
        "capture" => Category::Capture,
        "plugins" => Category::Shell,
        "display" => Category::Display,
        _ => return None,
    })
}
/// The leading icon of a setting's row. A section's rows carry icons all
/// together or not at all, so titles in one card stay aligned.
pub fn icon(route: &str) -> Option<&'static str> {
    Some(match route {
        "appearance/theme" => "moon",
        "appearance/window_buttons" => "app-window",
        "appearance/focus_ring" => "focus",
        "appearance/window_frame" => "blend",
        "appearance/button_shape" => "rectangle-horizontal",
        "appearance/app_style" => "app-window",
        "appearance/app_style_mode" => "moon",
        "appearance/app_style_corners" => "square-round-corner",
        "appearance/shadows" => "layers",
        "appearance/scrollbars" => "move-vertical",
        "appearance/corners"
        | "sizing/corner_radius"
        | "appearance/islands_radius"
        | "windows/rounding" => "square-round-corner",
        "appearance/density" => "rows-3",
        "appearance/theme_file" => "file-code",
        "sizing/text_size" => "a-large-small",
        "appearance/icon_theme" => "shapes",
        "sizing/icon_size" => "scaling",
        "fonts/weight" => "bold",
        "wallpaper/source" => "image",
        "wallpaper/motion" => "film",
        "wallpaper/pause_idle" => "moon",
        "wallpaper/hide_clock" | "datetime/lock_wallpaper_clock" | "datetime/lock_time" => "clock",
        "wallpaper/hide_date" | "datetime/lock_wallpaper_date" | "datetime/lock_date" => {
            "calendar-days"
        }
        "datetime/lock_style" => "layers",
        "datetime/lock_layout" => "panels-top-left",
        "datetime/lock_font" => "type",
        "datetime/lock_weight" => "bold",
        "datetime/lock_background" => "image",
        "datetime/lock_size" => "scaling",
        "wallpaper/fit" => "crop",
        "sizing/frame_width" => "square-dashed",
        "sizing/bar_height" | "shell/bar" => "panel-top",
        "sizing/sidebar_width" | "appearance/islands_rail" | "shell/sidebar_enabled" => {
            "panel-left"
        }
        "appearance/islands_joined_bar" => "link",
        "appearance/islands_height" => "rectangle-horizontal",
        "appearance/islands_gap" => "between-horizontal-start",
        "appearance/islands_edge_gap" => "move-vertical",
        "appearance/islands_side_gap" | "appearance/islands_padding" => "move-horizontal",
        "appearance/islands_fullscreen_padding" => "maximize",
        "shell/bar_shadow" => "layers",
        "shell/panel_shadow" => "copy",
        "shell/material" => "blend",
        "shell/chrome_color" => "palette",
        "shell/chrome_custom" => "pipette",
        "shell/chrome_opacity" => "layers",
        "shell/glass_tint" => "droplet",
        "shell/glass_blur" => "aperture",
        "shell/glass_bending" => "scan-eye",
        "shell/glass_highlight" => "sun",
        "shell/activity_status" => "activity",
        "shell/sidebar" => "arrow-left-right",
        "shell/sidebar_pin_scope" => "pin",
        "appearance/islands_sidebar_behavior" => "eye",
        "shell/assistant_enabled" => "sparkles",
        "shell/assistant_corner" => "corner-down-right",
        "windows/gaps_inner" => "columns-2",
        "windows/gaps_outer" => "square-dashed",
        "windows/border_size" | "windows/border_opacity" | "windows/border_inactive_opacity" => {
            "square"
        }
        "windows/border_color" => "palette",
        "windows/border_custom" => "pipette",
        "windows/animations" | "quick-search/animations" | "quick-search/background_fade" => {
            "wand-sparkles"
        }
        "windows/animation_rate" | "windows/pointer_speed" => "gauge",
        "windows/animation_style" => "film",
        "windows/focus_pointer" => "mouse-pointer",
        "windows/restore_apps" => "history",
        "windows/pointer_acceleration" => "trending-up",
        "windows/left_handed" => "mouse",
        "windows/natural_scroll" => "arrow-down-up",
        "windows/scroll_speed" => "chevrons-up-down",
        "windows/tap_to_click" => "pointer",
        "windows/disable_while_typing" => "keyboard-off",
        "windows/gestures" => "hand",
        "windows/repeat_delay" => "timer",
        "windows/repeat_rate" => "repeat",
        "windows/caps_lock" => "arrow-big-up",
        "windows/zoom" => "zoom-in",
        "windows/cursor_size" => "mouse-pointer-2",
        "notifications/dnd" | "notifications/hold_fullscreen" => "bell-off",
        "notifications/critical" => "triangle-alert",
        "notifications/position" => "layout-panel-top",
        "notifications/style" => "brush",
        "notifications/duration" | "quick-search/clipboard-retention" => "timer",
        "notifications/width" => "move-horizontal",
        "notifications/count" => "layers",
        "notifications/images" => "image",
        "notifications/history" | "quick-search/recent" => "history",
        "notifications/retention" => "calendar-clock",
        "notifications/budget" => "hard-drive",
        "notifications/clear" => "trash-2",
        "quick-search/enabled" => "search",
        "quick-search/shortcut" => "keyboard",
        "quick-search/compact" => "rows-3",
        "quick-search/rows" => "list-ordered",
        "quick-search/apps" => "layout-grid",
        "quick-search/windows" => "app-window",
        "quick-search/actions" => "settings",
        "quick-search/calculator" => "calculator",
        "quick-search/files" => "file-search",
        "quick-search/in_apps" => "blocks",
        "quick-search/clipboard" | "capture/copy" => "clipboard",
        "quick-search/web" | "quick-search/web-provider" => "globe",
        "quick-search/clear-usage" => "eraser",
        "quick-search/clear-clipboard" => "clipboard-x",
        "keyboard/input_method" => "languages",
        "windows/layout_switch" => "arrow-right-left",
        "power/blank" => "monitor-off",
        "power/lock" => "lock",
        "power/suspend" => "moon",
        "power/suspend_ac" => "plug",
        "power/lid" => "laptop",
        "power/critical" => "battery-warning",
        "power/memory" => "memory-stick",
        "power/temporary_storage" => "hard-drive",
        "power/game_priority" => "chevrons-up",
        "controllers/notify" => "bell",
        "datetime/clock_24h" => "clock-3",
        "datetime/language" => "languages",
        "display/enabled" => "power",
        "display/color" => "palette",
        "display/resolution" => "maximize-2",
        "display/refresh" => "activity",
        "display/scale" => "zoom-in",
        "display/rotation" => "rotate-cw",
        "display/position" => "move",
        "display/mirror" => "copy",
        "display/night_light" => "sunset",
        "display/night_light_temperature" => "thermometer",
        "display/night_light_schedule" => "calendar-clock",
        "display/night_light_times" => "clock",
        "display/x11_sharp" => "focus",
        "sound/output" => "speaker",
        "sound/input" => "mic",
        "accessibility/reduce_motion" => "move",
        "accessibility/keyboard" => "keyboard",
        "accessibility/menu" => "accessibility",
        "accessibility/screen_reader" => "audio-lines",
        "apps/screen_time" => "hourglass",
        "backups/history" => "history",
        "backups/schedule" => "calendar-clock",
        "capture/save" => "folder",
        "capture/format" => "image",
        "capture/jpeg_quality" => "sliders-horizontal",
        "capture/png_compression" => "shrink",
        "capture/container" => "clapperboard",
        "capture/codec" => "film",
        "capture/framerate" => "gauge",
        "capture/quality" => "sparkles",
        "capture/audio" => "volume-2",
        "capture/pointer" => "mouse-pointer-2",
        _ => return None,
    })
}
pub fn find(route: &str) -> Option<Entry> {
    ENTRIES.iter().find(|entry| entry.route == route).copied()
}
pub fn valid_route(route: &str) -> bool {
    category(route).is_some() || find(route).is_some()
}
pub fn search(query: &str) -> Vec<Entry> {
    let query = enzui_core::palette::normalize(query);
    if query.trim().is_empty() {
        return Vec::new();
    }
    let words: Vec<_> = query.split_whitespace().collect();
    let mut found: Vec<_> = ENTRIES
        .iter()
        .filter(|entry| {
            let text = enzui_core::palette::normalize(&format!(
                "{} {} {} {} {}",
                entry.english_search_text(),
                entry.title(),
                entry.category().title(),
                entry.section(),
                entry.keywords()
            ));
            words.iter().all(|word| text.contains(word))
        })
        .copied()
        .collect();
    found.sort_by_key(|entry| {
        (
            !enzui_core::palette::normalize(entry.title()).starts_with(&query),
            entry.title(),
        )
    });
    found.truncate(30);
    found
}

#[cfg(test)]
mod tests;
