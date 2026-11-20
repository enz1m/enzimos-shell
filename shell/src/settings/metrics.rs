use enzui_shell::i18n::tr;
use enzui_shell::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]

pub struct Metrics {
    pub frame: u16,
    pub radius: u16,
    pub bar: u16,
    pub rail: u16,
    pub text: u16,
    pub icon: u16,
    pub islands: Option<crate::shell_style::Islands>,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            // Wide enough to read as a frame around the desktop; at 3 it was
            // a hairline.
            frame: 8,
            radius: 22,
            bar: 36,
            rail: 36,
            text: 13,
            icon: 16,
            islands: None,
        }
    }
}
/// Named text-size choices over a persisted pixel body size; arbitrary saved sizes map to the
/// nearest choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStep {
    ExtraSmall,
    Small,
    Medium,
    Large,
    ExtraLarge,
}
impl TextStep {
    pub const ALL: [Self; 5] = [
        Self::ExtraSmall,
        Self::Small,
        Self::Medium,
        Self::Large,
        Self::ExtraLarge,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ExtraSmall => "XS",
            Self::Small => "S",
            Self::Medium => "M",
            Self::Large => "L",
            Self::ExtraLarge => "XL",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::ExtraSmall => tr("shell-extra-small"),
            Self::Small => tr("shell-small"),
            Self::Medium => tr("shell-medium"),
            Self::Large => tr("shell-large"),
            Self::ExtraLarge => tr("shell-extra-large"),
        }
    }
    /// Body size in logical pixels; the whole type ramp is scaled from this.
    pub fn px(self) -> u16 {
        match self {
            Self::ExtraSmall => 11,
            Self::Small => 12,
            Self::Medium => 13,
            Self::Large => 15,
            Self::ExtraLarge => 17,
        }
    }
    /// Returns the nearest text-size choice, preferring the larger size on ties.
    pub fn nearest(px: u16) -> Self {
        Self::ALL
            .into_iter()
            .rev()
            .min_by_key(|step| step.px().abs_diff(px))
            .expect("TextStep::ALL is not empty")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Frame,
    Radius,
    Bar,
    Rail,
    Text,
    Icon,
}
impl Metric {
    pub const ALL: [Self; 6] = [
        Self::Frame,
        Self::Radius,
        Self::Bar,
        Self::Rail,
        Self::Text,
        Self::Icon,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Frame => "frame_width",
            Self::Radius => "corner_radius",
            Self::Bar => "bar_height",
            Self::Rail => "sidebar_width",
            Self::Text => "text_size",
            Self::Icon => "icon_size",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Frame => tr("shell-frame-width"),
            Self::Radius => tr("shell-corner-radius"),
            Self::Bar => tr("shell-top-bottom-bar-thickness"),
            Self::Rail => tr("shell-sidebar-width"),
            Self::Text => tr("shell-shell-text-size"),
            Self::Icon => tr("shell-shell-icon-size"),
        }
    }
    pub fn bounds(self) -> (u16, u16) {
        match self {
            Self::Frame => (0, 24),
            Self::Radius => (0, 32),
            Self::Bar => (32, 72),
            Self::Rail => (32, 64),
            Self::Text => (10, 20),
            Self::Icon => (12, 24),
        }
    }
}
impl Metrics {
    /// Returns the body-size factor applied uniformly to the type ramp.
    pub fn text_scale(self) -> f32 {
        f32::from(self.text) / f32::from(Metrics::default().text)
    }
    pub fn get(self, metric: Metric) -> u16 {
        match metric {
            Metric::Frame => self.frame,
            Metric::Radius => self.radius,
            Metric::Bar => self.bar,
            Metric::Rail => self.rail,
            Metric::Text => self.text,
            Metric::Icon => self.icon,
        }
    }
    pub fn set(&mut self, metric: Metric, value: u16) -> Result<(), String> {
        let (min, max) = metric.bounds();
        if !(min..=max).contains(&value) {
            return Err(
                tr!("shell-must-be-px-2", "arg0" => metric.label().to_owned(), "arg1" => format!("{:}", min), "arg2" => format!("{:}", max)),
            );
        }
        match metric {
            Metric::Frame => self.frame = value,
            Metric::Radius => self.radius = value,
            Metric::Bar => self.bar = value,
            Metric::Rail => self.rail = value,
            Metric::Text => self.text = value,
            Metric::Icon => self.icon = value,
        };
        Ok(())
    }
    pub fn icon_slot(self) -> u16 {
        if self.islands.is_some() {
            self.rail - 4
        } else {
            (self.icon + 16).min(self.rail - 4)
        }
    }
    /// A pinned app's slot in the miniapp rail: as wide as the rail allows, so
    /// its icon follows the rail's width (on the frame it stayed at the icon
    /// size, small in a wide rail). `enzui_sidebar` lays panes out by it too.
    pub fn rail_slot(self) -> u16 {
        self.rail - 4
    }
    pub fn same_bar_placement(self, other: Self) -> bool {
        (
            self.frame,
            self.bar,
            self.bar_band(),
            self.islands.is_some(),
        ) == (
            other.frame,
            other.bar,
            other.bar_band(),
            other.islands.is_some(),
        )
    }
    #[must_use]
    pub fn sidebar_layout(self) -> Self {
        let mut metrics = self;
        metrics.text = Metrics::default().text;
        if let Some(style) = &mut metrics.islands {
            metrics.icon = Metrics::default().icon;
            style.gap = 8;
            style.padding = crate::shell_style::Islands::default().padding;
        }
        metrics
    }
    pub fn bar_item_height(self) -> u16 {
        if self.islands.is_some() {
            self.bar
        } else {
            self.bar - 8
        }
    }
    pub fn bar_item_radius(self, frame_radius: f32) -> f32 {
        if self.islands.is_some() {
            self.island_radius()
        } else {
            frame_radius
        }
    }
    /// The corners of an island and of the connected bar: the panel corner
    /// radius, at most a pill.
    pub fn island_radius(self) -> f32 {
        (f32::from(self.bar) / 2.0).min(f32::from(self.radius))
    }
    pub fn bar_padding(self, frame: u16) -> u16 {
        self.islands.map_or(frame, |style| style.padding)
    }
    pub fn bar_offset(self) -> u16 {
        self.islands.map_or(0, |style| style.edge_gap)
    }
    pub fn bar_band(self) -> u16 {
        self.bar + self.islands.map_or(0, |style| style.edge_gap + 8)
    }
    pub fn screen_inset(self) -> u16 {
        self.islands.map_or(self.frame, |style| style.side_gap)
    }
    pub fn lua(self) -> String {
        let style = self
            .islands
            .map_or(String::new(), |style| format!(",islands={}", style.lua()));
        format!(
            "{{frame={},radius={},bar={},rail={},text={},icon={}{style}}}",
            self.frame, self.radius, self.bar, self.rail, self.text, self.icon
        )
    }
}

/// Replaces the theme's text-scale factor without compounding earlier calls.
pub fn apply_text_scale(theme: &mut enzui_theme::EnzTheme, factor: f32) {
    let (scale, _) = enzui_theme::text_scale::TextScale::admit(factor);
    *theme = theme.with_text_scale(scale);
}
