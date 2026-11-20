use super::SystemSettings;
use crate::platform::Firewall;
use crate::system_controls::SystemControls;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const PORTS: &[&str] = &["9470/tcp", "9471-9599/udp"];

/// Firewall access on the zones the desktop considers trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// No managed firewall is active.
    Unfiltered,
    /// Another firewall manages incoming traffic.
    Other(&'static str),
    /// firewalld has no trusted zone available.
    NoTrustedZone,
    /// All required rules are saved and active on trusted zones.
    Allowed,
    /// Explicit administrator approval can add missing trusted-zone rules.
    NeedsApproval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    zone: String,
    permanent: bool,
    kind: &'static str,
    value: &'static str,
}

async fn run(
    controls: &SystemControls,
    args: &[String],
    limit: Duration,
) -> Result<String, String> {
    controls
        .run_with(
            "firewall-cmd",
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            None,
            limit,
        )
        .await
        .map_err(|error| match error {
            crate::system_controls::Failure::Rejected(message)
            | crate::system_controls::Failure::Other(message) => message,
        })
}

fn arguments(zone: &str, permanent: bool, operation: String) -> Vec<String> {
    let mut args = vec![format!("--zone={zone}")];
    if permanent {
        args.push("--permanent".into());
    }
    args.push(operation);
    args
}

async fn missing(controls: &SystemControls) -> Result<(usize, Vec<Rule>), String> {
    let names = run(controls, &["--get-zones".into()], Duration::from_secs(3)).await?;
    let zones: BTreeSet<_> = names.split_whitespace().collect();
    if zones.len() > 256 {
        return Err("The firewall zone inventory is too large".into());
    }
    let selected: Vec<_> = zones
        .iter()
        .copied()
        .filter(|zone| super::trusted_zone(zone))
        .collect();
    let mut missing = Vec::new();
    for zone in &selected {
        for permanent in [false, true] {
            for (kind, wanted) in [("port", PORTS), ("service", &["mdns"][..])] {
                let current = run(
                    controls,
                    &arguments(zone, permanent, format!("--list-{kind}s")),
                    Duration::from_secs(3),
                )
                .await?;
                let present: BTreeSet<_> = current.split_whitespace().collect();
                for value in wanted {
                    if !present.contains(value) {
                        missing.push(Rule {
                            zone: (*zone).to_owned(),
                            permanent,
                            kind,
                            value,
                        });
                    }
                }
            }
        }
    }
    Ok((selected.len(), missing))
}

async fn allow(controls: &SystemControls) -> Result<Access, String> {
    let (zones, plan) = missing(controls).await?;
    if zones == 0 {
        return Err("No trusted firewall zone is available".into());
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    for rule in plan {
        let limit = deadline.saturating_duration_since(Instant::now());
        if limit.is_zero() {
            return Err("Firewall approval timed out. Some trusted-network rules may already have been saved.".into());
        }
        run(
            controls,
            &arguments(
                &rule.zone,
                rule.permanent,
                format!("--add-{}={}", rule.kind, rule.value),
            ),
            limit,
        )
        .await
        .map_err(|error| {
            format!("Trusted-network access is not fully configured. Retry to finish it. {error}")
        })?;
    }
    if !missing(controls).await?.1.is_empty() {
        return Err(
            "The firewall changed during configuration. Retry trusted-network access.".into(),
        );
    }
    Ok(Access::Allowed)
}

impl SystemSettings {
    /// Read Phone Link access without changing or enabling a firewall.
    pub async fn phone_network(&self) -> Result<Access, String> {
        if self.demo {
            return Ok(Access::NeedsApproval);
        }
        match crate::platform::firewall(&self.controls).await {
            Firewall::None => Ok(Access::Unfiltered),
            Firewall::Other(name) => Ok(Access::Other(name)),
            Firewall::Firewalld => {
                let (zones, rules) = missing(&self.controls).await?;
                Ok(if zones == 0 {
                    Access::NoTrustedZone
                } else if rules.is_empty() {
                    Access::Allowed
                } else {
                    Access::NeedsApproval
                })
            }
        }
    }

    /// Allow the fixed phone ports and discovery on trusted zones only.
    pub async fn allow_phone_network(&self) -> Result<Access, String> {
        if self.demo || !self.writes_allowed {
            return Err("Firewall changes are off in this preview".into());
        }
        if crate::platform::firewall(&self.controls).await != Firewall::Firewalld {
            return Err(
                "Phone Link needs an active firewalld service to configure trusted-network access"
                    .into(),
            );
        }
        allow(&self.controls).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt;

    fn executable(root: &Path, name: &str, script: &str) {
        let path = root.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    use std::path::Path;

    #[tokio::test]
    async fn trusted_phone_access_preserves_public_rules_and_retries_partial_approval() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        executable(
            root,
            "systemctl",
            "#!/bin/sh\nif [ \"$1 $2\" = 'is-active firewalld.service' ]; then echo active; exit 0; fi\nexit 3\n",
        );
        executable(
            root,
            "firewall-cmd",
            r#"#!/usr/bin/python3
import json,sys
from pathlib import Path
root=Path(__file__).resolve().parent
path=root/'rules.json';state=json.loads(path.read_text());args=sys.argv[1:]
if args==['--get-zones']:
    print(' '.join(state));sys.exit(0)
zone=next(arg[7:] for arg in args if arg.startswith('--zone='))
scope='saved' if '--permanent' in args else 'runtime'
row=state[zone][scope]
for kind in ('port','service'):
    if '--list-'+kind+'s' in args:
        print(' '.join(row[kind]));sys.exit(0)
    operation=next((arg for arg in args if arg.startswith('--add-'+kind+'=')),None)
    if operation:
        value=operation.split('=',1)[1]
        fail=root/'deny-next'
        if fail.exists() and zone=='home' and value=='9471-9599/udp':
            fail.unlink();print('approval refused',file=sys.stderr);sys.exit(1)
        if value not in row[kind]:row[kind].append(value)
        path.write_text(json.dumps(state));print('success');sys.exit(0)
print('unsupported fixture operation',file=sys.stderr);sys.exit(2)
"#,
        );
        let original = json!({
            "public": {
                "runtime": {"port":["22/tcp"],"service":["ssh"]},
                "saved": {"port":["22/tcp"],"service":["ssh"]}
            },
            "home": {
                "runtime": {"port":["9470/tcp"],"service":["ssh"]},
                "saved": {"port":["9470/tcp"],"service":["ssh"]}
            },
            "enzimos-trusted": {
                "runtime": {"port":[],"service":["mdns"]},
                "saved": {"port":[],"service":["mdns"]}
            }
        });
        let state_file = root.join("rules.json");
        std::fs::write(&state_file, original.to_string()).unwrap();
        let mut settings = SystemSettings {
            controls: SystemControls::fixture(root),
            demo: false,
            writes_allowed: false,
            enzimos: Default::default(),
        };
        assert!(settings.allow_phone_network().await.is_err());
        let read =
            || serde_json::from_slice::<Value>(&std::fs::read(&state_file).unwrap()).unwrap();
        assert_eq!(
            read(),
            original,
            "a read-only preview changed firewall rules"
        );
        settings.writes_allowed = true;
        assert_eq!(
            settings.phone_network().await.unwrap(),
            Access::NeedsApproval
        );
        std::fs::write(root.join("deny-next"), b"refuse one change").unwrap();
        assert!(settings.allow_phone_network().await.is_err());
        let partial = read();
        assert_eq!(partial["public"], original["public"]);
        assert_eq!(
            partial["home"]["runtime"]["service"],
            original["home"]["runtime"]["service"]
        );
        assert_eq!(
            settings.phone_network().await.unwrap(),
            Access::NeedsApproval
        );
        assert_eq!(
            settings.allow_phone_network().await.unwrap(),
            Access::Allowed
        );
        assert_eq!(settings.phone_network().await.unwrap(), Access::Allowed);
        let after = read();
        assert_eq!(
            after["public"], original["public"],
            "public network access changed"
        );
        for zone in ["home", "enzimos-trusted"] {
            for scope in ["runtime", "saved"] {
                let row = &after[zone][scope];
                for port in ["9470/tcp", "9471-9599/udp"] {
                    assert!(
                        row["port"].as_array().unwrap().contains(&json!(port)),
                        "phone media access missing in {zone}/{scope}"
                    );
                }
                assert!(row["service"].as_array().unwrap().contains(&json!("mdns")));
                if zone == "home" {
                    assert!(row["service"].as_array().unwrap().contains(&json!("ssh")));
                }
            }
        }
    }
}
