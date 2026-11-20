use super::super::manifest::Permission;
use crate::settings;
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf};

pub(super) const MAX_INSTALLED: usize = 16;
pub(super) const MAX_STATE_BYTES: usize = 128 * 1024;
#[derive(Clone)]
pub struct Record {
    pub(super) id: String,
    pub(super) digest: [u8; 32],
    pub(super) enabled: bool,
    pub(super) activation: u64,
    pub(super) config_revision: u64,
    pub(super) grants: BTreeSet<Permission>,
    pub(super) config: Value,
    pub(super) fault: Option<String>,
}
impl Record {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn activation(&self) -> u64 {
        self.activation
    }
    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }
    pub fn grants(&self) -> &BTreeSet<Permission> {
        &self.grants
    }
    pub fn config(&self) -> &Value {
        &self.config
    }
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }
}
#[derive(Clone, Default)]
pub struct Registry {
    pub serial: u64,
    pub entries: Vec<Record>,
}
#[derive(Clone)]
pub struct Store {
    pub(super) root: PathBuf,
    pub(super) settings: Option<settings::Store>,
}
