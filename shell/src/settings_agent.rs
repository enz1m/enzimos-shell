//! Allowlisted settings exposed to authorized MCP Center agents.
//!
//! Changes use GUI validation, persistence and polkit. Broker credentials and
//! security-weakening settings are excluded from the agent-facing catalogue.

use crate::settings::{BarEdge, Change, Metric, Settings, SidebarEdge};
use crate::shell_style::ShellStyle;
use crate::system_settings::{Clock, Request};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::{Value, json};

/// The app id Settings registers and is enrolled under.
pub const APP_ID: &str = "dev.enzui.Settings";
/// The most entries one listing returns.
pub const PAGE_LIMIT: usize = 50;
// A choice list longer than this is left out of listings; reading the one
// setting returns it.
const LISTED_CHOICES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// This account's desktop preferences.
    User,
    /// The computer's; changing it needs an administrator through polkit.
    System,
}

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Boolean,
    Integer {
        min: u16,
        max: u16,
    },
    Choice(&'static [&'static str]),
    /// One of the time zones the system offers.
    TimeZone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Language,
    DarkMode,
    CornerRadius,
    TextSize,
    BarPosition,
    BarHeight,
    ShellStyle,
    SidebarEnabled,
    ActivityStatus,
    SidebarSide,
    Tiling,
    InputMethod,
    NightLight,
    NightLightTemperature,
    Clock24h,
    DoNotDisturb,
    TimeZone,
    AutomaticTime,
}

pub struct Entry {
    pub key: Key,
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub scope: Scope,
    pub kind: Kind,
}

const fn entry(
    key: Key,
    id: &'static str,
    title: &'static str,
    description: &'static str,
    category: &'static str,
    scope: Scope,
    kind: Kind,
) -> Entry {
    Entry {
        key,
        id,
        title,
        description,
        category,
        scope,
        kind,
    }
}

pub static ENTRIES: [Entry; 18] = [
    entry(
        Key::Language,
        "desktop.language",
        "Language",
        "Language for the desktop and shell applications.",
        "date_time",
        Scope::User,
        Kind::Choice(&["auto", "en-US", "es"]),
    ),
    entry(
        Key::DarkMode,
        "appearance.dark_mode",
        "Dark mode",
        "The shell and applications that follow the desktop use the dark colour scheme.",
        "appearance",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::CornerRadius,
        "appearance.corner_radius",
        "Corner radius",
        "Rounding of the shell's corners, in pixels.",
        "appearance",
        Scope::User,
        Kind::Integer { min: 0, max: 32 },
    ),
    entry(
        Key::TextSize,
        "appearance.text_size",
        "Shell text size",
        "Body text size of the shell, in pixels.",
        "appearance",
        Scope::User,
        Kind::Integer { min: 10, max: 20 },
    ),
    entry(
        Key::BarPosition,
        "shell.bar_position",
        "Bar position",
        "The screen edge the bar sits on.",
        "shell",
        Scope::User,
        Kind::Choice(&["top", "bottom"]),
    ),
    entry(
        Key::BarHeight,
        "shell.bar_height",
        "Bar thickness",
        "Thickness of the top or bottom bar, in pixels.",
        "shell",
        Scope::User,
        Kind::Integer { min: 32, max: 72 },
    ),
    entry(
        Key::ShellStyle,
        "shell.style",
        "Shell style",
        "`frame` draws a frame around the desktop; `islands` floats the bar and sidebar.",
        "shell",
        Scope::User,
        Kind::Choice(&["frame", "islands"]),
    ),
    entry(
        Key::SidebarEnabled,
        "shell.sidebar_enabled",
        "Sidebar",
        "Whether the sidebar of pinned apps is shown.",
        "shell",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::ActivityStatus,
        "shell.activity_status",
        "Background work in the bar",
        "Copies and downloads of other windows show in the bar's center while they run.",
        "shell",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::SidebarSide,
        "shell.sidebar_side",
        "Sidebar side",
        "The screen edge the sidebar sits on.",
        "shell",
        Scope::User,
        Kind::Choice(&["left", "right"]),
    ),
    entry(
        Key::Tiling,
        "windows.tiling",
        "Tile windows",
        "Desktops without their own layout tile their windows instead of floating them.",
        "windows",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::NightLight,
        "display.night_light",
        "Night light",
        "Warmer screen colours on its schedule (by default 20:00 to 07:00).",
        "display",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::NightLightTemperature,
        "display.night_light_temperature",
        "Night light colour temperature",
        "Kelvin while night light is on; lower is warmer.",
        "display",
        Scope::User,
        Kind::Integer {
            min: 2500,
            max: 6000,
        },
    ),
    entry(
        Key::InputMethod,
        "keyboard.input_method",
        "Input method",
        "Fcitx5 for typing Chinese, Japanese, Korean and other scripts. Unset, it follows the language and the account's Fcitx5 setup.",
        "keyboard",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::Clock24h,
        "clock.24_hour",
        "24-hour clock",
        "The bar and lock screen show the time in 24-hour format.",
        "clock",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::DoNotDisturb,
        "notifications.do_not_disturb",
        "Do not disturb",
        "Notifications are kept in history without being shown.",
        "notifications",
        Scope::User,
        Kind::Boolean,
    ),
    entry(
        Key::TimeZone,
        "system.time_zone",
        "Time zone",
        "The computer's time zone. Changing it asks an administrator for approval.",
        "date_time",
        Scope::System,
        Kind::TimeZone,
    ),
    entry(
        Key::AutomaticTime,
        "system.automatic_time",
        "Set time automatically",
        "Keep the clock synchronized over the network. Changing it asks an administrator for approval.",
        "date_time",
        Scope::System,
        Kind::Boolean,
    ),
];

#[must_use]
pub fn find(id: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.id == id)
}

/// What an agent's change becomes.
#[derive(Clone, Debug)]
pub enum Target {
    User(Change),
    System(Request),
}

/// Why a read or change is refused: a short code and a sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// What the running Settings knows beyond its saved preferences.
#[derive(Clone, Copy, Default)]
pub struct Context<'a> {
    pub clock: Option<&'a Clock>,
    pub islands_supported: bool,
    /// Whether this Settings may change the computer (not a preview).
    pub system_writes: bool,
}

impl Entry {
    /// The current value, or `None` while it cannot be read.
    #[must_use]
    pub fn value(&self, settings: &Settings, context: &Context) -> Option<Value> {
        Some(match self.key {
            Key::Language => json!(settings.language.key()),
            Key::DarkMode => json!(!settings.light),
            Key::CornerRadius => json!(settings.metrics.radius),
            Key::TextSize => json!(settings.metrics.text),
            Key::BarPosition => json!(match settings.bar {
                BarEdge::Top => "top",
                BarEdge::Bottom => "bottom",
            }),
            Key::BarHeight => json!(settings.metrics.bar),
            Key::ShellStyle => json!(match settings.shell_style {
                ShellStyle::Frame => "frame",
                ShellStyle::Islands => "islands",
            }),
            Key::SidebarEnabled => json!(settings.sidebar_enabled),
            Key::ActivityStatus => json!(settings.activity_status),
            Key::SidebarSide => json!(match settings.sidebar {
                SidebarEdge::Left => "left",
                SidebarEdge::Right => "right",
            }),
            Key::Tiling => json!(settings.tiling.unwrap_or(false)),
            Key::InputMethod => json!(crate::input_method::wanted(settings.input_method)),
            Key::NightLight => json!(settings.night_light.enabled),
            Key::NightLightTemperature => json!(settings.night_light.temperature),
            Key::Clock24h => json!(settings.session.clock_24h),
            Key::DoNotDisturb => json!(settings.notifications.policy.dnd),
            Key::TimeZone => json!(context.clock?.timezone.as_ref()?),
            Key::AutomaticTime => json!(context.clock?.ntp?),
        })
    }

    /// Why this setting cannot be changed now, if it cannot.
    #[must_use]
    pub fn unavailable(&self, context: &Context) -> Option<&'static str> {
        match self.key {
            Key::TimeZone | Key::AutomaticTime if !context.system_writes => {
                Some(tr("shell-system-changes-are-off-in-this-preview"))
            }
            Key::TimeZone if context.clock.is_none_or(|clock| clock.zones.is_empty()) => {
                Some(tr("shell-the-time-zone-service-is-not-available"))
            }
            Key::AutomaticTime if !context.clock.is_some_and(|clock| clock.can_ntp) => {
                Some(tr("shell-network-time-is-not-available-on-this-computer"))
            }
            _ => None,
        }
    }

    fn choices(&self, context: &Context) -> Option<Vec<String>> {
        match self.kind {
            Kind::Choice(values) => Some(values.iter().map(|v| (*v).to_owned()).collect()),
            Kind::TimeZone => Some(
                context
                    .clock
                    .map(|clock| clock.zones.clone())
                    .unwrap_or_default(),
            ),
            _ => None,
        }
    }

    /// The setting as an agent sees it. `full` includes long choice lists.
    #[must_use]
    pub fn describe(&self, settings: &Settings, context: &Context, full: bool) -> Value {
        let unavailable = self.unavailable(context);
        let mut out = json!({
            "key": self.id,
            "title": crate::i18n::label(self.title),
            "description": crate::i18n::label(self.description),
            "category": self.category,
            "scope": self.scope.name(),
            "type": match self.kind {
                Kind::Boolean => "boolean",
                Kind::Integer { .. } => "integer",
                Kind::Choice(_) | Kind::TimeZone => "choice",
            },
            "value": self.value(settings, context),
            "available": unavailable.is_none(),
            "requires_admin": self.scope == Scope::System,
            "change_function": match self.scope {
                Scope::User => "settings.set",
                Scope::System => "settings.set_system",
            },
        });
        if let Some(reason) = unavailable {
            out["unavailable_reason"] = json!(reason);
        }
        if let Kind::Integer { min, max } = self.kind {
            out["minimum"] = json!(min);
            out["maximum"] = json!(max);
        }
        if let Some(choices) = self.choices(context) {
            if full || choices.len() <= LISTED_CHOICES {
                out["choices"] = json!(choices);
            } else {
                out["choice_count"] = json!(choices.len());
            }
        }
        out
    }

    /// The Settings change `value` asks for, validated as the controls would.
    ///
    /// # Errors
    /// `invalid_value` for a value of the wrong type or outside its range or
    /// choices, `unavailable` when the setting cannot change now.
    pub fn target(&self, value: &Value, context: &Context) -> Result<Target, Refusal> {
        if let Some(reason) = self.unavailable(context) {
            return Err(Refusal::new("unavailable", reason));
        }
        let invalid =
            |detail: String| Refusal::new("invalid_value", format!("{}: {detail}", self.id));
        let boolean = || {
            value
                .as_bool()
                .ok_or_else(|| invalid(tr("shell-expected-true-or-false").into()))
        };
        let integer = || {
            match self.kind {
            Kind::Integer { min, max } => value
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .filter(|v| (min..=max).contains(v))
                .ok_or_else(|| invalid(tr!("shell-expected-a-whole-number-from-to", "arg0" => format!("{:}", min), "arg1" => format!("{:}", max)))),
            _ => Err(invalid(tr("shell-not-a-number-setting").into())),
        }
        };
        let choice = || match (self.choices(context), value.as_str()) {
            (Some(choices), Some(text)) if choices.iter().any(|c| c == text) => Ok(text.to_owned()),
            _ => Err(invalid(tr("shell-not-one-of-its-choices").into())),
        };
        Ok(match self.key {
            Key::Language => Target::User(Change::Language(
                crate::i18n::Language::from_key(&choice()?)
                    .ok_or_else(|| invalid(tr("shell-not-one-of-its-choices").into()))?,
            )),
            Key::DarkMode => Target::User(Change::Light(!boolean()?)),
            Key::CornerRadius => Target::User(Change::Metric(Metric::Radius, integer()?)),
            Key::TextSize => Target::User(Change::Metric(Metric::Text, integer()?)),
            Key::BarHeight => Target::User(Change::Metric(Metric::Bar, integer()?)),
            Key::BarPosition => Target::User(Change::Bar(if choice()? == "top" {
                BarEdge::Top
            } else {
                BarEdge::Bottom
            })),
            Key::SidebarSide => Target::User(Change::Sidebar(if choice()? == "left" {
                SidebarEdge::Left
            } else {
                SidebarEdge::Right
            })),
            Key::ShellStyle => {
                let style = if choice()? == "frame" {
                    ShellStyle::Frame
                } else {
                    ShellStyle::Islands
                };
                if style == ShellStyle::Islands && !context.islands_supported {
                    return Err(Refusal::new(
                        "unavailable",
                        tr("shell-islands-needs-a-current-managed-desktop"),
                    ));
                }
                Target::User(Change::ShellStyle(style))
            }
            Key::SidebarEnabled => Target::User(Change::SidebarEnabled(boolean()?)),
            Key::ActivityStatus => Target::User(Change::ActivityStatus(boolean()?)),
            Key::Tiling => Target::User(Change::Tiling(boolean()?)),
            Key::InputMethod => Target::User(Change::InputMethod(boolean()?)),
            Key::NightLight => Target::User(Change::NightLight(
                crate::night_light::Change::Enabled(boolean()?),
            )),
            Key::NightLightTemperature => Target::User(Change::NightLight(
                crate::night_light::Change::Temperature(integer()?),
            )),
            Key::Clock24h => Target::User(Change::Session(
                crate::session_preferences::SessionChange::Clock24h(boolean()?),
            )),
            Key::DoNotDisturb => Target::User(Change::Notification(
                crate::notifications::preferences::Change::Dnd(boolean()?),
            )),
            Key::TimeZone => Target::System(Request::Timezone(choice()?)),
            Key::AutomaticTime => Target::System(Request::Ntp(boolean()?)),
        })
    }
}

/// One page of the catalog, in catalog order.
#[must_use]
pub fn list(
    settings: &Settings,
    context: &Context,
    offset: usize,
    limit: usize,
    scope: Option<Scope>,
    category: Option<&str>,
) -> Value {
    let matching: Vec<&Entry> = ENTRIES
        .iter()
        .filter(|entry| {
            scope.is_none_or(|scope| entry.scope == scope)
                && category.is_none_or(|category| entry.category == category)
        })
        .collect();
    let limit = limit.clamp(1, PAGE_LIMIT);
    let page: Vec<Value> = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|entry| entry.describe(settings, context, false))
        .collect();
    let next = offset.saturating_add(limit);
    json!({
        "settings": page,
        "total": matching.len(),
        "next_offset": if next < matching.len() { json!(next) } else { Value::Null },
    })
}

/// Whether a listing with these filters shows a system setting.
#[must_use]
pub fn touches_system(scope: Option<Scope>, category: Option<&str>) -> bool {
    ENTRIES.iter().any(|entry| {
        entry.scope == Scope::System
            && scope.is_none_or(|s| s == entry.scope)
            && category.is_none_or(|c| c == entry.category)
    })
}

/// Parses a listing's `scope` argument.
#[must_use]
pub fn scope(name: &str) -> Option<Scope> {
    match name {
        "user" => Some(Scope::User),
        "system" => Some(Scope::System),
        _ => None,
    }
}

fn key_schema(scope: Option<Scope>) -> Value {
    let keys: Vec<&str> = ENTRIES
        .iter()
        .filter(|e| scope.is_none_or(|s| e.scope == s))
        .map(|e| e.id)
        .collect();
    json!({"type": "string", "enum": keys})
}

fn value_schema() -> Value {
    json!({"type": ["boolean", "integer", "string"]})
}

fn entry_schema() -> Value {
    json!({
        "type": "object",
        "required": ["key", "title", "description", "category", "scope", "type", "value", "available", "requires_admin", "change_function"],
        "properties": {
            "key": {"type": "string"},
            "title": {"type": "string"},
            "description": {"type": "string"},
            "category": {"type": "string"},
            "scope": {"type": "string", "enum": ["user", "system"]},
            "type": {"type": "string", "enum": ["boolean", "integer", "choice"]},
            "value": {"type": ["boolean", "integer", "string", "null"]},
            "available": {"type": "boolean"},
            "unavailable_reason": {"type": "string"},
            "requires_admin": {"type": "boolean"},
            "change_function": {"type": "string"},
            "minimum": {"type": "integer"},
            "maximum": {"type": "integer"},
            "choices": {"type": "array", "items": {"type": "string"}, "maxItems": 1024},
            "choice_count": {"type": "integer"}
        }
    })
}

/// Input and output schemas of the four functions: list, get, set and
/// set_system, in that order.
#[must_use]
pub fn schemas() -> [(Value, Value); 4] {
    let categories: Vec<&str> = {
        let mut all: Vec<&str> = ENTRIES.iter().map(|e| e.category).collect();
        all.dedup();
        all
    };
    let list_in = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "offset": {"type": "integer", "minimum": 0, "maximum": 1000},
            "limit": {"type": "integer", "minimum": 1, "maximum": PAGE_LIMIT},
            "scope": {"type": "string", "enum": ["user", "system"]},
            "category": {"type": "string", "enum": categories}
        }
    });
    let list_out = json!({
        "type": "object",
        "required": ["settings", "total", "next_offset"],
        "properties": {
            "settings": {"type": "array", "items": entry_schema(), "maxItems": PAGE_LIMIT},
            "total": {"type": "integer"},
            "next_offset": {"type": ["integer", "null"]}
        }
    });
    let get_in = json!({
        "type": "object", "additionalProperties": false, "required": ["key"],
        "properties": {"key": key_schema(None)}
    });
    let set_out = json!({
        "type": "object",
        "required": ["key", "value"],
        "properties": {"key": {"type": "string"}, "value": value_schema()}
    });
    let set_in = |scope| {
        json!({
            "type": "object", "additionalProperties": false, "required": ["key", "value"],
            "properties": {"key": key_schema(Some(scope)), "value": value_schema()}
        })
    };
    [
        (list_in, list_out),
        (get_in, entry_schema()),
        (set_in(Scope::User), set_out.clone()),
        (set_in(Scope::System), set_out),
    ]
}
