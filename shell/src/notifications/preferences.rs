use super::{
    model::{Policy, Rule},
    payload,
};
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    #[default]
    BarEnd,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
impl Position {
    pub const ALL: [Self; 5] = [
        Self::BarEnd,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::BarEnd => "bar-end",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::BarEnd => tr("shell-follow-bar"),
            Self::TopLeft => tr("shell-top-left"),
            Self::TopRight => tr("shell-top-right"),
            Self::BottomLeft => tr("shell-bottom-left"),
            Self::BottomRight => tr("shell-bottom-right"),
        }
    }

    /// Whether the toast stack is anchored at the bottom of the output.
    pub fn is_bottom(self, bar: crate::settings::BarEdge) -> bool {
        matches!(self, Self::BottomLeft | Self::BottomRight)
            || self == Self::BarEnd && bar == crate::settings::BarEdge::Bottom
    }
}
/// How dense a notification card is. Every style is raised above windows;
/// they differ in spacing, icon size and how much of the message a toast shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    Compact,
    #[default]
    Standard,
    Spacious,
}
impl Style {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Standard, Self::Spacious];
    pub fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Standard => "standard",
            Self::Spacious => "spacious",
        }
    }
    /// The style a saved key names; the retired tonal, outlined and elevated
    /// styles read as Standard.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "tonal" | "outlined" | "elevated" => Some(Self::Standard),
            _ => Self::ALL.into_iter().find(|style| style.key() == key),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => tr("shell-compact"),
            Self::Standard => tr("shell-standard"),
            Self::Spacious => tr("shell-spacious"),
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Compact => tr("shell-small-cards-with-one-line-of-the-message"),
            Self::Standard => tr("shell-balanced-cards-with-two-lines-of-the-message"),
            Self::Spacious => tr("shell-large-cards-with-a-bigger-icon-and-more-of-the-message"),
        }
    }
    /// Message lines a toast shows before "Show more".
    pub fn toast_lines(self) -> usize {
        match self {
            Self::Compact => 1,
            Self::Standard => 2,
            Self::Spacious => 3,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub label: String,
    pub icon: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub policy: Policy,
    pub position: Position,
    pub style: Style,
    pub width: u16,
    pub maximum_toasts: u8,
    pub images: bool,
    pub silence_game_mode: bool,
    pub applications: BTreeMap<String, Application>,
}
impl Preferences {
    pub fn game_mode_active(&self, observed: Option<bool>, previous: Option<&Policy>) -> bool {
        observed.unwrap_or_else(|| {
            previous.is_some_and(|policy| policy.dnd && self.silence_game_mode && !self.policy.dnd)
        })
    }
    pub fn effective_policy(&self, game_mode_active: bool) -> Policy {
        let mut policy = self.policy.clone();
        policy.dnd |= self.silence_game_mode && game_mode_active;
        policy
    }
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            position: Position::default(),
            style: Style::default(),
            width: 416,
            maximum_toasts: 3,
            images: true,
            silence_game_mode: false,
            applications: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Dnd(bool),
    SilenceGameMode(bool),
    HoldFullscreen(bool),
    Critical(bool),
    Duration(u32),
    Position(Position),
    Style(Style),
    Width(u16),
    MaximumToasts(u8),
    Images(bool),
    History(bool),
    HistoryMinutes(u32),
    HistoryMiB(u32),
    ClearHistory,
    Application {
        principal: String,
        application: Application,
        rule: Rule,
    },
    Observe {
        principal: String,
        application: Application,
    },
    Forget(String),
}
impl Change {
    pub fn same_key(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Application { principal: a, .. }, Self::Application { principal: b, .. })
            | (Self::Observe { principal: a, .. }, Self::Observe { principal: b, .. })
            | (Self::Forget(a), Self::Forget(b)) => a == b,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
    pub fn erases_history(&self) -> bool {
        matches!(self, Self::ClearHistory | Self::History(false))
    }
}
fn rule_key(rule: Rule) -> &'static str {
    match rule {
        Rule::Allow => "allow",
        Rule::Block => "block",
        Rule::Priority => "priority",
    }
}
fn rule(value: &Value) -> Result<Rule, String> {
    match value.as_str() {
        Some("allow") => Ok(Rule::Allow),
        Some("block") => Ok(Rule::Block),
        Some("priority") => Ok(Rule::Priority),
        _ => Err(tr("shell-invalid-application-notification-policy").into()),
    }
}
impl Preferences {
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        if !value.is_object() || value.get("version").is_some_and(|v| v.as_u64() != Some(1)) {
            return Err(tr("shell-unsupported-notification-preferences").into());
        }
        let mut preferences = Self::default();
        let boolean = |key, default| {
            value.get(key).map_or(Ok(default), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-notification-boolean-preference"))
            })
        };
        let number = |key, default| {
            value.get(key).map_or(Ok(default), |value| {
                value
                    .as_u64()
                    .ok_or_else(|| tr("shell-invalid-notification-numeric-preference"))
            })
        };
        preferences.policy.dnd = boolean("dnd", false)?;
        preferences.silence_game_mode = boolean("silence_game_mode", false)?;
        preferences.policy.allow_critical = boolean("allow_critical", false)?;
        preferences.policy.hold_fullscreen = boolean("hold_fullscreen", true)?;
        preferences.policy.toast_ms =
            u32::try_from(number("toast_ms", preferences.policy.toast_ms.into())?)
                .map_err(|_| tr("shell-invalid-toast-duration"))?;
        preferences.policy.history = boolean("history", false)?;
        preferences.policy.history_generation = number("history_generation", 0)?;
        preferences.policy.history_minutes = u32::try_from(number(
            "history_minutes",
            preferences.policy.history_minutes.into(),
        )?)
        .map_err(|_| tr("shell-invalid-history-retention"))?;
        preferences.policy.history_mib = u32::try_from(number(
            "history_mib",
            preferences.policy.history_mib.into(),
        )?)
        .map_err(|_| tr("shell-invalid-history-budget"))?;
        preferences.width = u16::try_from(number("width", preferences.width.into())?)
            .map_err(|_| tr("shell-invalid-toast-width"))?;
        preferences.maximum_toasts =
            u8::try_from(number("maximum_toasts", preferences.maximum_toasts.into())?)
                .map_err(|_| tr("shell-invalid-toast-count"))?;
        preferences.images = boolean("images", true)?;
        if let Some(position) = value.get("position") {
            preferences.position = Position::ALL
                .into_iter()
                .find(|position_| Some(position_.key()) == position.as_str())
                .ok_or_else(|| tr("shell-invalid-notification-position"))?;
        }
        if let Some(style) = value.get("style") {
            preferences.style = style
                .as_str()
                .and_then(Style::from_key)
                .ok_or_else(|| tr("shell-invalid-notification-style"))?;
        }
        if let Some(applications) = value.get("applications") {
            let applications = applications
                .as_object()
                .filter(|v| v.len() <= 128)
                .ok_or_else(|| tr("shell-too-many-application-notification-policies"))?;
            for (principal, app) in applications {
                if !app.is_object() {
                    return Err(tr("shell-invalid-application-notification-preference").into());
                }
                let label = app.get("label").map_or(Ok("Application"), |value| {
                    value
                        .as_str()
                        .ok_or_else(|| tr("shell-invalid-application-label"))
                })?;
                let icon = app.get("icon").map_or(Ok("bell"), |value| {
                    value
                        .as_str()
                        .ok_or_else(|| tr("shell-invalid-application-icon"))
                })?;
                preferences.applications.insert(
                    principal.clone(),
                    Application {
                        label: payload::text(label, 96, false)?,
                        icon: icon.into(),
                    },
                );
                preferences.policy.rules.insert(
                    principal.clone(),
                    app.get("rule")
                        .map(rule)
                        .transpose()?
                        .unwrap_or(Rule::Allow),
                );
            }
        }
        preferences.validate()?;
        Ok(preferences)
    }
    fn validate(&self) -> Result<(), String> {
        self.policy.validate()?;
        if !(300..=640).contains(&self.width)
            || !(1..=5).contains(&self.maximum_toasts)
            || self.applications.len() > 128
            || self.applications.keys().ne(self.policy.rules.keys())
            || self.applications.values().any(|app| {
                app.label.trim().is_empty()
                    || payload::text(&app.label, 96, false).as_ref() != Ok(&app.label)
                    || app.icon.is_empty()
                    || app.icon.len() > 64
                    || !app
                        .icon
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
        {
            return Err(tr("shell-invalid-notification-appearance-or-application-catalog").into());
        }
        Ok(())
    }
    pub fn preview(&mut self, change: &Change) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.apply(change)?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    fn apply(&mut self, change: &Change) -> Result<(), String> {
        match change {
            Change::Dnd(value) => self.policy.dnd = *value,
            Change::SilenceGameMode(value) => self.silence_game_mode = *value,
            Change::Critical(value) => self.policy.allow_critical = *value,
            Change::HoldFullscreen(value) => self.policy.hold_fullscreen = *value,
            Change::Duration(value) => self.policy.toast_ms = *value,
            Change::Position(value) => self.position = *value,
            Change::Style(value) => self.style = *value,
            Change::Width(value) => self.width = *value,
            Change::MaximumToasts(value) => self.maximum_toasts = *value,
            Change::Images(value) => self.images = *value,
            Change::History(value) => {
                if self.policy.history != *value {
                    self.policy.history_generation = self
                        .policy
                        .history_generation
                        .checked_add(1)
                        .ok_or_else(|| tr("shell-notification-history-generation-exhausted"))?;
                    self.policy.history = *value;
                }
            }
            Change::HistoryMinutes(value) => self.policy.history_minutes = *value,
            Change::HistoryMiB(value) => self.policy.history_mib = *value,
            Change::ClearHistory => {
                self.policy.history_generation = self
                    .policy
                    .history_generation
                    .checked_add(1)
                    .ok_or_else(|| tr("shell-notification-history-generation-exhausted"))?;
            }
            Change::Observe {
                principal,
                application,
            }
            | Change::Application {
                principal,
                application,
                ..
            } => {
                if !self.applications.contains_key(principal) && self.applications.len() >= 128 {
                    if let Some(removable) = self
                        .applications
                        .keys()
                        .find(|key| self.policy.rule(key) == Rule::Allow)
                        .cloned()
                    {
                        self.applications.remove(&removable);
                        self.policy.rules.remove(&removable);
                    } else if matches!(change, Change::Observe { .. }) {
                        return Ok(());
                    } else {
                        return Err(
                            tr("shell-remove-an-application-policy-before-adding-another").into(),
                        );
                    }
                }
                match change {
                    Change::Application { rule, .. } => {
                        self.applications
                            .entry(principal.clone())
                            .or_insert_with(|| application.clone());
                        self.policy.rules.insert(principal.clone(), *rule);
                    }
                    _ => {
                        self.applications
                            .insert(principal.clone(), application.clone());
                        self.policy
                            .rules
                            .entry(principal.clone())
                            .or_insert(Rule::Allow);
                    }
                }
            }
            Change::Forget(principal) => {
                self.applications.remove(principal);
                self.policy.rules.remove(principal);
            }
        }
        Ok(())
    }
    pub fn update_json(value: &mut Value, change: &Change) -> Result<(), String> {
        if !value.is_object() {
            return Err(tr("shell-shell-preferences-must-be-an-object").into());
        }
        let before = Self::from_json(value.get("notifications"))?;
        let mut after = before.clone();
        after.preview(change)?;
        if before == after {
            return Ok(());
        }
        if value.get("notifications").is_none() {
            value["notifications"] = json!({"version":1});
        }
        let document = &mut value["notifications"];
        match change {
            Change::Dnd(value) => document["dnd"] = (*value).into(),
            Change::SilenceGameMode(value) => document["silence_game_mode"] = (*value).into(),
            Change::Critical(value) => document["allow_critical"] = (*value).into(),
            Change::HoldFullscreen(value) => document["hold_fullscreen"] = (*value).into(),
            Change::Duration(value) => document["toast_ms"] = (*value).into(),
            Change::Position(value) => document["position"] = value.key().into(),
            Change::Style(value) => document["style"] = value.key().into(),
            Change::Width(value) => document["width"] = (*value).into(),
            Change::MaximumToasts(value) => document["maximum_toasts"] = (*value).into(),
            Change::Images(value) => document["images"] = (*value).into(),
            Change::History(value) => {
                document["history"] = (*value).into();
                document["history_generation"] = after.policy.history_generation.into();
            }
            Change::HistoryMinutes(value) => document["history_minutes"] = (*value).into(),
            Change::HistoryMiB(value) => document["history_mib"] = (*value).into(),
            Change::ClearHistory => {
                document["history_generation"] = after.policy.history_generation.into();
            }
            Change::Application { .. } | Change::Observe { .. } | Change::Forget(_) => {
                if document.get("applications").is_none() {
                    document["applications"] = json!({});
                }
                let entries = document["applications"]
                    .as_object_mut()
                    .ok_or_else(|| tr("shell-invalid-notification-application-preferences"))?;
                entries.retain(|key, _| after.applications.contains_key(key));
                for (principal, app) in &after.applications {
                    if before.applications.get(principal) == Some(app)
                        && before.policy.rules.get(principal) == after.policy.rules.get(principal)
                    {
                        continue;
                    }
                    let entry = entries
                        .entry(principal.clone())
                        .or_insert_with(|| json!({}));
                    entry["label"] = app.label.clone().into();
                    entry["icon"] = app.icon.clone().into();
                    entry["rule"] = rule_key(after.policy.rule(principal)).into();
                }
            }
        }
        Ok(())
    }
}
