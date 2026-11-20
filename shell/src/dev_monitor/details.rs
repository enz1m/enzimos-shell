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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn details_behaviour() {
        // Detail preserves unavailable metrics and rejects changed identity.
        {
            let id = ProcessId {
                pid: 42,
                start_time: 100,
            };
            let text = r#"{"pid":42,"start_time":100,"label":"web","cmdline":["node","server.js"],"exe":null,"cwd":null,"unit":null,"threads":1,"rss_bytes":123,"pss_bytes":null,"open_descriptors":null,"read_bytes":null,"written_bytes":null,"children":[]}"#;
            let detail = Detail::parse(text, id).unwrap();
            assert!(detail.pss_bytes.is_none());
            assert!(detail.open_descriptors.is_none());
            assert!(
                Detail::parse(
                    text,
                    ProcessId {
                        start_time: 101,
                        ..id
                    }
                )
                .is_err()
            );
        }
        // Lookup does not invent refresh version or timestamp.
        {
            let result = Lookup::parse(
                r#"{"ports":[],"processes":[],"warnings":["engine unavailable"]}"#,
                8080,
            )
            .unwrap()
            .snapshot();
            assert_eq!(result.scan_id, 0);
            assert_eq!(result.taken_at_unix_ms, 0);
            assert_eq!(result.warnings.len(), 1);
            assert!(Lookup::parse(r#"{"ports":[],"processes":[],"warnings":[]}"#, 0).is_err());
        }
    }
}
