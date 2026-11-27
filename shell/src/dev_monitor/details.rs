use super::*;
use enzui_shell::i18n::tr;

#[derive(Clone, Debug, Deserialize)]
pub struct Detail {
    #[serde(flatten)]
    pub identity: ProcessId,
    pub label: String,
    pub cmdline: Vec<String>,
    pub exe: Option<String>,
    pub cwd: Option<String>,
    pub unit: Option<String>,
    pub threads: u32,
    pub rss_bytes: Option<u64>,
    pub pss_bytes: Option<u64>,
    pub open_descriptors: Option<usize>,
    pub read_bytes: Option<u64>,
    pub written_bytes: Option<u64>,
    pub children: Vec<Child>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Child {
    #[serde(flatten)]
    pub identity: ProcessId,
    pub label: String,
    pub rss_bytes: Option<u64>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Lookup {
    pub ports: Vec<Port>,
    pub processes: Vec<Process>,
    pub warnings: Vec<String>,
}
impl Detail {
    pub(super) fn parse(text: &str, expected: ProcessId) -> Result<Self, String> {
        let detail: Self = super::parse(text)?;
        if detail.identity != expected
            || detail.children.len() > 256
            || detail.cmdline.len() > 4096
            || detail
                .children
                .iter()
                .any(|child| child.identity.pid <= 1 || child.identity.start_time == 0)
        {
            return Err(
                tr("shell-dev-monitor-returned-a-changed-or-incomplete-process-detail").into(),
            );
        }
        Ok(detail)
    }
}
impl Lookup {
    pub(super) fn parse(text: &str, port: u16) -> Result<Self, String> {
        let lookup: Self = super::parse(text)?;
        if port == 0
            || lookup.ports.len() > 8192
            || lookup.processes.len() > 4096
            || lookup.ports.iter().any(|row| row.port != port)
        {
            return Err(tr("shell-dev-monitor-returned-an-invalid-port-lookup").into());
        }
        Ok(lookup)
    }
    pub fn snapshot(self) -> Snapshot {
        Snapshot {
            version: VERSION,
            scan_id: 0,
            taken_at_unix_ms: 0,
            ports: self.ports,
            processes: self.processes,
            engines: Vec::new(),
            projects: Vec::new(),
            warnings: self.warnings,
        }
    }
}
