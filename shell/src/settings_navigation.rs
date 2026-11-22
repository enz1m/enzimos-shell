use super::{Message, SettingsApp};
use enzui_shell::settings_catalog::Category;
use iced::{
    Rectangle, Subscription, Task, Vector,
    advanced::widget::{
        Id, Operation,
        operation::{Outcome, Scrollable},
    },
    window,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(super) struct Section {
    pub category: Category,
    pub label: &'static str,
    pub route: &'static str,
}
const SECTIONS: &[Section] = &[
    Section {
        category: Category::Phone,
        label: "shell-phone-devices",
        route: "phone/overview",
    },
    Section {
        category: Category::Phone,
        label: "shell-phone-sharing",
        route: "phone/sharing",
    },
    Section {
        category: Category::DateTime,
        label: "language-title",
        route: "datetime/language",
    },
    Section {
        category: Category::Users,
        label: "shell-accounts",
        route: "users/accounts",
    },
    Section {
        category: Category::Appearance,
        label: "shell-style-color",
        route: "appearance/theme",
    },
    Section {
        category: Category::Appearance,
        label: "shell-shape-and-spacing",
        route: "appearance/corners",
    },
    Section {
        category: Category::Appearance,
        label: "shell-app-styles",
        route: "appearance/app_style",
    },
    Section {
        category: Category::Appearance,
        label: "shell-wallpaper",
        route: "wallpaper/source",
    },
    Section {
        category: Category::Appearance,
        label: "shell-text-icons",
        route: "sizing/text_size",
    },
    Section {
        category: Category::Appearance,
        label: "shell-fonts",
        route: "fonts/ui",
    },
    Section {
        category: Category::Shell,
        label: "shell-shell-style",
        route: "appearance/shell_style",
    },
    Section {
        category: Category::Shell,
        label: "shell-frame-size",
        route: "sizing/frame_width",
    },
    Section {
        category: Category::Shell,
        label: "shell-islands-size",
        route: "appearance/islands_height",
    },
    Section {
        category: Category::Shell,
        label: "shell-placement",
        route: "shell/bar",
    },
    Section {
        category: Category::Shell,
        label: "shell-ai-assistant",
        route: "shell/assistant_enabled",
    },
    Section {
        category: Category::Shell,
        label: "shell-bar-contents",
        route: "bar-layout/items",
    },
    Section {
        category: Category::Shell,
        label: "shell-quick-controls",
        route: "controls/network",
    },
    Section {
        category: Category::Shell,
        label: "shell-plugins",
        route: "plugins/installed",
    },
    Section {
        category: Category::Windows,
        label: "shell-tiling-gaps",
        route: "windows/tiling",
    },
    Section {
        category: Category::Windows,
        label: "shell-appearance-motion",
        route: "windows/border_size",
    },
    Section {
        category: Category::Windows,
        label: "shell-behavior",
        route: "windows/focus_pointer",
    },
    Section {
        category: Category::Search,
        label: "shell-quick-search",
        route: "quick-search/enabled",
    },
    Section {
        category: Category::Search,
        label: "shell-sources",
        route: "quick-search/apps",
    },
    Section {
        category: Category::Search,
        label: "shell-filename-search",
        route: "quick-search/roots",
    },
    Section {
        category: Category::Search,
        label: "shell-ranking-privacy",
        route: "quick-search/recent",
    },
    Section {
        category: Category::Search,
        label: "shell-quick-keys-favorites",
        route: "quick-search/aliases",
    },
    Section {
        category: Category::Notifications,
        label: "shell-delivery",
        route: "notifications/dnd",
    },
    Section {
        category: Category::Notifications,
        label: "shell-toast-appearance",
        route: "notifications/position",
    },
    Section {
        category: Category::Notifications,
        label: "shell-history-privacy",
        route: "notifications/history",
    },
    Section {
        category: Category::Notifications,
        label: "shell-apps",
        route: "notifications/applications",
    },
    Section {
        category: Category::Capture,
        label: "shell-images",
        route: "capture/format",
    },
    Section {
        category: Category::Capture,
        label: "shell-screen-recording",
        route: "capture/container",
    },
    Section {
        category: Category::Network,
        label: "shell-internet",
        route: "network/internet",
    },
    Section {
        category: Category::Network,
        label: "shell-wi-fi",
        route: "network/wifi",
    },
    Section {
        category: Category::Network,
        label: "shell-wired",
        route: "network/wired",
    },
    Section {
        category: Category::Network,
        label: "shell-vpn",
        route: "network/vpn",
    },
    Section {
        category: Category::Network,
        label: "shell-firewall",
        route: "network/firewall",
    },
    Section {
        category: Category::Bluetooth,
        label: "shell-paired-devices",
        route: "bluetooth/devices",
    },
    Section {
        category: Category::Remote,
        label: "shell-remote-sign-in",
        route: "remote/ssh",
    },
    Section {
        category: Category::Remote,
        label: "shell-desktop-sharing",
        route: "remote/sharing",
    },
    Section {
        category: Category::Display,
        label: "shell-arrangement",
        route: "display/arrangement",
    },
    Section {
        category: Category::Display,
        label: "shell-display-settings",
        route: "display/resolution",
    },
    Section {
        category: Category::Display,
        label: "shell-legacy-apps",
        route: "display/x11_sharp",
    },
    Section {
        category: Category::Display,
        label: "shell-night-light",
        route: "display/night_light",
    },
    Section {
        category: Category::Sound,
        label: "shell-output",
        route: "sound/output",
    },
    Section {
        category: Category::Sound,
        label: "shell-input",
        route: "sound/input",
    },
    Section {
        category: Category::Power,
        label: "shell-battery",
        route: "power/battery",
    },
    Section {
        category: Category::Power,
        label: "shell-power-mode",
        route: "power/mode",
    },
    Section {
        category: Category::Power,
        label: "shell-game-mode",
        route: "power/game_mode",
    },
    Section {
        category: Category::Power,
        label: "shell-screen-sleep",
        route: "power/blank",
    },
    Section {
        category: Category::Power,
        label: "shell-memory-protection",
        route: "power/memory",
    },
    Section {
        category: Category::Power,
        label: "shell-tmp-storage-heading",
        route: "power/temporary_storage",
    },
    Section {
        category: Category::Input,
        label: "shell-mouse",
        route: "windows/pointer_speed",
    },
    Section {
        category: Category::Input,
        label: "shell-scrolling",
        route: "windows/natural_scroll",
    },
    Section {
        category: Category::Input,
        label: "shell-touchpad",
        route: "windows/tap_to_click",
    },
    Section {
        category: Category::Keyboard,
        label: "shell-layouts",
        route: "keyboard/layouts",
    },
    Section {
        category: Category::Keyboard,
        label: "shell-typing",
        route: "windows/repeat_delay",
    },
    Section {
        category: Category::Keyboard,
        label: "shell-shortcuts",
        route: "keyboard/shortcuts",
    },
    Section {
        category: Category::Accessibility,
        label: "shell-seeing",
        route: "windows/zoom",
    },
    Section {
        category: Category::Accessibility,
        label: "shell-bar-buttons",
        route: "accessibility/keyboard",
    },
    Section {
        category: Category::Controllers,
        label: "shell-connected-controllers",
        route: "controllers/list",
    },
    Section {
        category: Category::Printers,
        label: "shell-your-printers",
        route: "printers/list",
    },
    Section {
        category: Category::Printers,
        label: "shell-print-jobs",
        route: "printers/jobs",
    },
    Section {
        category: Category::Thunderbolt,
        label: "shell-thunderbolt-devices",
        route: "thunderbolt/devices",
    },
    Section {
        category: Category::Disks,
        label: "shell-disks",
        route: "disks/list",
    },
    Section {
        category: Category::DateTime,
        label: "shell-time",
        route: "datetime/timezone",
    },
    Section {
        category: Category::Appearance,
        label: "shell-login-screen",
        route: "datetime/lock_time",
    },
    Section {
        category: Category::Developer,
        label: "shell-developer-tools",
        route: "developer/tools",
    },
    Section {
        category: Category::Developer,
        label: "shell-projects",
        route: "developer/projects",
    },
    Section {
        category: Category::Updates,
        label: "shell-enzimos-section",
        route: "updates/install",
    },
    Section {
        category: Category::Updates,
        label: "shell-versions",
        route: "updates/generations",
    },
    Section {
        category: Category::Updates,
        label: "shell-device-firmware",
        route: "updates/firmware",
    },
    Section {
        category: Category::Backups,
        label: "shell-overview",
        route: "backups/overview",
    },
    Section {
        category: Category::Backups,
        label: "shell-snapshots",
        route: "backups/history",
    },
    Section {
        category: Category::Backups,
        label: "shell-encrypted-backups",
        route: "backups/schedule",
    },
    Section {
        category: Category::Backups,
        label: "shell-whats-backed-up",
        route: "backups/rules",
    },
    Section {
        category: Category::Backups,
        label: "shell-backup-space",
        route: "backups/space",
    },
    Section {
        category: Category::Backups,
        label: "shell-restore-from-a-backup",
        route: "backups/points",
    },
    Section {
        category: Category::Apps,
        label: "shell-default-apps",
        route: "apps/defaults",
    },
    Section {
        category: Category::Apps,
        label: "shell-startup-apps",
        route: "apps/startup",
    },
    Section {
        category: Category::Apps,
        label: "shell-background-apps",
        route: "apps/background",
    },
    Section {
        category: Category::Apps,
        label: "shell-app-permissions",
        route: "apps/permissions",
    },
    Section {
        category: Category::Apps,
        label: "shell-location",
        route: "apps/location",
    },
    Section {
        category: Category::Apps,
        label: "shell-parental-controls",
        route: "apps/parental",
    },
    Section {
        category: Category::Apps,
        label: "shell-screen-time",
        route: "apps/screen_time",
    },
    Section {
        category: Category::Agents,
        label: "shell-permissions",
        route: "agents/permissions",
    },
    Section {
        category: Category::Agents,
        label: "shell-assistant",
        route: "agents/assistant",
    },
    Section {
        category: Category::Agents,
        label: "shell-on-device-ai",
        route: "agents/local-ai",
    },
    Section {
        category: Category::Agents,
        label: "shell-ai-providers",
        route: "agents/ai-providers",
    },
    Section {
        category: Category::Agents,
        label: "shell-identities",
        route: "agents/identities",
    },
    Section {
        category: Category::Agents,
        label: "shell-activity",
        route: "agents/activity",
    },
    Section {
        category: Category::About,
        label: "shell-device",
        route: "about/device",
    },
    Section {
        category: Category::About,
        label: "shell-system",
        route: "about/system",
    },
    Section {
        category: Category::About,
        label: "shell-hardware",
        route: "about/hardware",
    },
    Section {
        category: Category::About,
        label: "shell-problem-reports",
        route: "about/problems",
    },
];
impl SettingsApp {
    pub(super) fn sections(&self, category: Category) -> Vec<Section> {
        SECTIONS
            .iter()
            .copied()
            .filter(|s| s.category == category)
            .filter(|s| {
                self.layout_preview
                    || !matches!(
                        s.route,
                        "bar-layout/items"
                            | "notifications/dnd"
                            | "notifications/position"
                            | "notifications/history"
                            | "notifications/applications"
                            | "plugins/installed"
                    )
            })
            .filter(|s| s.route != "wallpaper/source" || self.wallpaper.supported)
            .filter(|s| {
                s.route != "sizing/frame_width"
                    || self.settings.shell_style == enzui_shell::shell_style::ShellStyle::Frame
            })
            .filter(|s| {
                s.route != "appearance/islands_height"
                    || self.settings.shell_style == enzui_shell::shell_style::ShellStyle::Islands
            })
            .collect()
    }
    pub(super) fn section(&mut self, route: &'static str) -> Task<Message> {
        let Some(section) = SECTIONS.iter().find(|s| s.route == route).copied() else {
            return Task::none();
        };
        if !self
            .sections(section.category)
            .iter()
            .any(|s| s.route == route)
        {
            return Task::none();
        }
        self.backup_setup_leave();
        if section.category != Category::Phone {
            self.phone_leave();
        }
        self.controls.navigation();
        self.numeric.cancel();
        self.nav.dismiss_overlay();
        self.picker = None;
        self.menu = None;
        self.filter.clear();
        self.highlighted = None;
        self.reveal_pending = false;
        if self.page != section.category {
            self.navigation.reset_scroll();
        }
        self.page = section.category;
        self.navigation.expanded = Some(section.category);
        self.navigation.request(route);
        Task::batch([
            self.read_controls(),
            self.read_wallpaper_gallery(),
            self.warm_wallpaper_preview(),
        ])
    }
    pub(super) fn observe_scrolled_section(&mut self) -> Task<Message> {
        let Some(window) = self.window else {
            return Task::none();
        };
        if !self.filter.trim().is_empty() || self.navigation.moving() {
            return Task::none();
        }
        let sections = self.sections(self.page);
        if sections.is_empty() {
            return Task::none();
        }
        self.navigation.spy_revision = self.navigation.spy_revision.wrapping_add(1);
        let revision = self.navigation.spy_revision;
        iced_runtime::task::widget_in(window, ViewportSections::new(sections))
            .map(move |route| Message::SectionObserved(revision, route))
    }

    pub(super) fn reveal_route(&mut self, route: &'static str) -> Task<Message> {
        self.navigation.request(route);
        self.navigation_frame()
    }

    pub(super) fn navigation_frame(&mut self) -> Task<Message> {
        let Some(window) = self.window else {
            return Task::none();
        };
        if !self.navigation_ready() {
            return Task::none();
        }
        if let Some(route) = self.navigation.pending.take() {
            self.navigation.locating = true;
            let epoch = self.navigation.epoch;
            return iced_runtime::task::widget_in(window, Locate::new(route))
                .map(move |position| Message::SectionLocated(epoch, position));
        }
        if let Some(y) = self.navigation.sample(Instant::now()) {
            return iced::widget::operation::scroll_to(
                "settings-scroll",
                iced::widget::scrollable::AbsoluteOffset { x: 0.0, y },
                iced::widget::operation::Animation::Instant,
            );
        }
        Task::none()
    }

    pub(super) fn navigation_ready(&self) -> bool {
        self.navigation
            .pending
            .is_none_or(|route| self.route_ready(route))
    }
}
#[derive(Default)]
pub(super) struct Navigation {
    pub scroll: enzui_widgets::scroll_view::ScrollTranslation,
    pub scroll_generation: u64,
    pub expanded: Option<Category>,
    pub selected: Option<&'static str>,
    pending: Option<&'static str>,
    locating: bool,
    pub epoch: u64,
    pub spy_revision: u64,
    motion: Option<ScrollMotion>,
    // Hold a jump target until scrolling moves away; short final sections cannot reach the
    // viewport top.
    held: Option<(&'static str, Option<f32>)>,
}
impl Navigation {
    pub(super) fn reset_scroll(&mut self) {
        self.scroll = Default::default();
        self.scroll_generation = self.scroll_generation.wrapping_add(1);
    }
    pub(super) fn interrupt(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.pending = None;
        self.locating = false;
        self.motion = None;
        self.held = None;
    }
    // What the scroll spy's observation means for the selection.
    pub(super) fn observed(&mut self, route: &'static str, offset: f32) -> Option<&'static str> {
        match self.held {
            Some((held, None)) => {
                self.held = Some((held, Some(offset)));
                None
            }
            Some((_, Some(at))) if (offset - at).abs() <= 2.0 => None,
            _ => {
                self.held = None;
                Some(route)
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        self.interrupt();
        self.spy_revision = self.spy_revision.wrapping_add(1);
    }
    fn request(&mut self, route: &'static str) {
        self.cancel();
        self.selected = Some(route);
        self.pending = Some(route);
    }
    pub(super) fn moving(&self) -> bool {
        self.pending.is_some() || self.locating || self.motion.is_some()
    }
    pub(super) fn located(&mut self, epoch: u64, position: Option<Position>, duration: Duration) {
        if epoch != self.epoch {
            return;
        }
        self.locating = false;
        self.motion = position.map(|position| ScrollMotion {
            position,
            began: Instant::now(),
            duration,
        });
    }
    fn sample(&mut self, at: Instant) -> Option<f32> {
        let motion = self.motion.as_ref()?;
        let (y, done) = motion.sample(at);
        if done {
            self.motion = None;
            self.held = self.selected.map(|route| (route, None));
        }
        Some(y)
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Position {
    from: f32,
    to: f32,
}
struct ScrollMotion {
    position: Position,
    began: Instant,
    duration: Duration,
}
impl ScrollMotion {
    fn sample(&self, at: Instant) -> (f32, bool) {
        let t = if self.duration.is_zero() {
            1.0
        } else {
            (at.saturating_duration_since(self.began).as_secs_f32() / self.duration.as_secs_f32())
                .clamp(0.0, 1.0)
        };
        let eased = 1.0 - (1.0 - t).powi(3);
        (
            self.position.from + (self.position.to - self.position.from) * eased,
            t == 1.0,
        )
    }
}
// Route, section and setting ids, and their observed top edges.
type Anchor = (&'static str, Id, Id, Option<f32>, Option<f32>);
struct ViewportSections {
    anchors: Vec<Anchor>,
    scroll: Option<(Rectangle, iced::Size, Vector)>,
    ambiguous: bool,
}
impl ViewportSections {
    fn new(sections: Vec<Section>) -> Self {
        Self {
            anchors: sections
                .into_iter()
                .map(|section| {
                    (
                        section.route,
                        format!("section-{}", section.route).into(),
                        format!("setting-{}", section.route).into(),
                        None,
                        None,
                    )
                })
                .collect(),
            scroll: None,
            ambiguous: false,
        }
    }
}
impl Operation<Option<(&'static str, f32)>> for ViewportSections {
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _: &Rectangle) {
        if !bounds.height.is_finite() || bounds.height <= 0.0 {
            return;
        }
        for (_, section, setting, anchor, fallback) in &mut self.anchors {
            let target = if id == Some(section) {
                anchor
            } else if id == Some(setting) {
                fallback
            } else {
                continue;
            };
            self.ambiguous |= target.replace(bounds.y).is_some();
        }
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: iced::Size,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new("settings-scroll")) {
            self.ambiguous |= self
                .scroll
                .replace((bounds, content, translation))
                .is_some();
        }
    }
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn Operation<Option<(&'static str, f32)>>)) {
        visit(self);
    }
    fn finish(&self) -> Outcome<Option<(&'static str, f32)>> {
        let selected =
            self.scroll
                .filter(|_| !self.ambiguous)
                .and_then(|(view, content, translation)| {
                    let maximum = (content.height - view.height).max(0.0);
                    if ![view.height, view.y, maximum, translation.y]
                        .into_iter()
                        .all(f32::is_finite)
                    {
                        return None;
                    }
                    let available: Vec<_> = self
                        .anchors
                        .iter()
                        .filter_map(|(route, _, _, anchor, fallback)| {
                            anchor
                                .or(fallback.map(|y| y - 64.0))
                                .filter(|y| y.is_finite())
                                .map(|y| (*route, y - view.y))
                        })
                        .collect();
                    if available.is_empty() {
                        return None;
                    }
                    let offset = translation.y.clamp(0.0, maximum);
                    let focus = offset + view.height * 0.25;
                    if maximum > 0.0 && offset >= maximum - 2.0 {
                        return available.last().map(|(route, _)| (*route, offset));
                    }
                    let mut current = available[0].0;
                    for (route, position) in available {
                        if position > focus {
                            break;
                        }
                        current = route;
                    }
                    Some((current, offset))
                });
        Outcome::Some(selected)
    }
}

struct Locate {
    target: Id,
    section: Id,
    anchor: Option<Rectangle>,
    scroll: Option<(Rectangle, iced::Size, Vector)>,
    found: Option<Rectangle>,
    ambiguous: bool,
}
impl Locate {
    fn new(route: &str) -> Self {
        Self {
            target: format!("setting-{route}").into(),
            section: format!("section-{route}").into(),
            anchor: None,
            scroll: None,
            found: None,
            ambiguous: false,
        }
    }
}
impl Operation<Option<Position>> for Locate {
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _: &Rectangle) {
        if id == Some(&self.section) {
            self.ambiguous |= self.anchor.is_some();
            self.anchor = Some(bounds);
        } else if id == Some(&self.target) {
            self.ambiguous |= self.found.is_some();
            self.found = Some(bounds);
        }
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: iced::Size,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new("settings-scroll")) {
            self.ambiguous |= self.scroll.is_some();
            self.scroll = Some((bounds, content, translation));
        }
    }
    fn traverse(&mut self, f: &mut dyn FnMut(&mut dyn Operation<Option<Position>>)) {
        f(self);
    }
    fn finish(&self) -> Outcome<Option<Position>> {
        let position = self
            .scroll
            .zip(self.anchor.or(self.found))
            .filter(|_| !self.ambiguous)
            .and_then(|((view, content, translation), target)| {
                let maximum = (content.height - view.height).max(0.0);
                let to = (target.y - view.y - 16.0).clamp(0.0, maximum);
                [to, translation.y, maximum]
                    .into_iter()
                    .all(f32::is_finite)
                    .then_some(Position {
                        from: translation.y.clamp(0.0, maximum),
                        to,
                    })
            });
        Outcome::Some(position)
    }
}
fn manual_scroll_interrupt(event: &iced::Event) -> bool {
    match event {
        iced::Event::Mouse(iced::mouse::Event::WheelScrolled { .. })
        | iced::Event::Touch(iced::touch::Event::FingerMoved { .. })
        | iced::Event::Window(window::Event::Resized(_) | window::Event::Unfocused) => true,
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(key),
            ..
        }) => matches!(
            key,
            iced::keyboard::key::Named::ArrowUp
                | iced::keyboard::key::Named::ArrowDown
                | iced::keyboard::key::Named::PageUp
                | iced::keyboard::key::Named::PageDown
                | iced::keyboard::key::Named::Home
                | iced::keyboard::key::Named::End
        ),
        _ => false,
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Interruption {
    User,
    Resize,
    Unfocused,
}

pub(super) fn interrupt_subscription() -> Subscription<Message> {
    iced::event::listen_with(|event, _, _| {
        let reason = match &event {
            iced::Event::Window(window::Event::Resized(_)) => Interruption::Resize,
            iced::Event::Window(window::Event::Unfocused) => Interruption::Unfocused,
            _ => Interruption::User,
        };
        manual_scroll_interrupt(&event).then_some(Message::InterruptScroll(reason))
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_cancellation_and_replacement_reject_late_measurements() {
        let mut state = Navigation::default();
        state.request("wallpaper/source");
        let old = state.epoch;
        state.request("appearance/theme");
        state.located(
            old,
            Some(Position {
                from: 0.0,
                to: 500.0,
            }),
            Duration::from_millis(200),
        );
        assert!(state.motion.is_none());
        state.cancel();
        assert!(!state.moving());
        let revision = state.spy_revision;
        state.interrupt();
        assert_eq!(state.spy_revision, revision);
    }
}
