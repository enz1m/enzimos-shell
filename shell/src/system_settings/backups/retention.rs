use enzui_shell::i18n::tr;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupRetention {
    pub automatic: bool,
    pub last: u32,
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
    /// Space the backups may use on the drive; the oldest go first.
    pub budget: Option<u64>,
}
impl BackupRetention {
    /// Smallest space limit the backend accepts.
    pub const BUDGET_MIN: u64 = 1 << 30;
    pub const MAX: u32 = 1000;
    pub fn from_json(value: &Value) -> Option<Self> {
        if value["schema"].as_u64() != Some(1) {
            return None;
        }
        let count = |key: &str| value[key].as_u64().and_then(|v| u32::try_from(v).ok());
        let policy = Self {
            automatic: value["automatic"].as_bool()?,
            last: count("last")?,
            daily: count("daily")?,
            weekly: count("weekly")?,
            monthly: count("monthly")?,
            yearly: count("yearly")?,
            budget: value["budget_bytes"].as_u64(),
        };
        policy.valid().then_some(policy)
    }
    pub fn valid(self) -> bool {
        self.last >= 1
            && [
                self.last,
                self.daily,
                self.weekly,
                self.monthly,
                self.yearly,
            ]
            .iter()
            .all(|v| *v <= Self::MAX)
            && self.budget.is_none_or(|b| b >= Self::BUDGET_MIN)
    }
    pub fn selection(self) -> Value {
        // The policy is replaced as a whole, so no limit is simply no field
        // (and backends without space limits keep accepting the selection).
        let mut selection = json!({"automatic":self.automatic,"last":self.last,"daily":self.daily,"weekly":self.weekly,"monthly":self.monthly,"yearly":self.yearly});
        if let Some(budget) = self.budget {
            selection["budget_bytes"] = json!(budget);
        }
        selection
    }
    pub fn preset(self) -> BackupPreset {
        BackupPreset::ALL
            .into_iter()
            .find(|p| {
                p.policy(self.automatic).map(|mut p| {
                    p.budget = self.budget;
                    p
                }) == Some(self)
            })
            .unwrap_or(BackupPreset::Custom)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupPreset {
    Recommended,
    More,
    Less,
    Custom,
}
impl BackupPreset {
    pub const ALL: [Self; 4] = [Self::Recommended, Self::More, Self::Less, Self::Custom];
    pub fn label(self) -> &'static str {
        match self {
            Self::Recommended => tr("shell-recommended"),
            Self::More => tr("shell-keep-more"),
            Self::Less => tr("shell-keep-less"),
            Self::Custom => tr("shell-custom"),
        }
    }
    pub fn policy(self, automatic: bool) -> Option<BackupRetention> {
        let [last, daily, weekly, monthly, yearly] = match self {
            Self::Recommended => [3, 14, 8, 12, 2],
            Self::More => [5, 30, 12, 24, 5],
            Self::Less => [2, 7, 4, 6, 1],
            Self::Custom => return None,
        };
        Some(BackupRetention {
            automatic,
            last,
            daily,
            weekly,
            monthly,
            yearly,
            budget: None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupState {
    Removed,
    Unchanged,
    Off,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cleanup {
    pub state: CleanupState,
    pub removed: Option<u64>,
    pub at_unix: i64,
    pub reason: Option<String>,
}
impl Cleanup {
    pub fn from_json(value: &Value) -> Option<Self> {
        let state = match value["status"].as_str()? {
            "removed" => CleanupState::Removed,
            "nothing-to-remove" => CleanupState::Unchanged,
            "automatic-cleanup-off" => CleanupState::Off,
            "failed" => CleanupState::Failed,
            _ => return None,
        };
        let removed = value["removed"].as_u64();
        if state != CleanupState::Failed && removed.is_none() {
            return None;
        }
        Some(Self {
            state,
            removed,
            at_unix: i64::try_from(value["at_unix"].as_u64()?).ok()?,
            reason: value["reason"]
                .as_str()
                .map(|s| s.chars().filter(|c| !c.is_control()).take(160).collect()),
        })
    }
}
