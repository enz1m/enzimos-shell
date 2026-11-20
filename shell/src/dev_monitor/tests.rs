use super::*;

#[test]
fn dev_monitor_behaviour() {
    // Closed refresh gate never starts work and rejects late delivery.
    {
        let mut gate = RefreshGate::default();
        let options = Options::default();
        assert!(gate.begin(options).is_none());
        gate.open();
        let first = gate.begin(options).unwrap();
        assert!(gate.begin(options).is_none());
        gate.close();
        assert!(!gate.finish(first, options));
        assert!(gate.begin(options).is_none());
        gate.open();
        let second = gate.begin(options).unwrap();
        assert!(!gate.finish(first, options));
        assert!(gate.begin(options).is_none());
        assert!(gate.finish(second, options));
        let third = gate.begin(options).unwrap();
        assert!(!gate.finish(
            third,
            Options {
                containers: true,
                ..options
            }
        ));
        assert!(gate.begin(options).is_some());
    }
    // Snapshot requires known version and engine controls require complete identity.
    {
        let value = serde_json::json!({"version":2,"scan_id":1,"taken_at_unix_ms":1,"ports":[],"processes":[],"engines":[],"warnings":[],"future":42});
        assert!(Snapshot::parse(&value.to_string()).is_ok());
        let mut unknown = value;
        unknown["version"] = 3.into();
        assert!(
            Snapshot::parse(&unknown.to_string())
                .unwrap_err()
                .replace(['\u{2068}', '\u{2069}'], "")
                .contains("version 3")
        );
        let mut engine: Engine = serde_json::from_value(serde_json::json!({
            "endpoint":{"engine":"podman","socket":"/run/test.sock","rootless":true},"error":null,"stale":false,"truncated":false,
            "containers":[{"id":"a".repeat(64),"name":"web","image":"test","state":"running","health":null,"autostart":null,"cpu_percent":null,"memory_bytes":null,"memory_limit_bytes":null,"detail_error":null,"stats_error":null}]
        })).unwrap();
        assert_eq!(engine.containers[0].autostart, None);
        assert!(engine.identity(&engine.containers[0]).is_some());
        engine.stale = true;
        assert!(engine.identity(&engine.containers[0]).is_none());
        engine.stale = false;
        engine.truncated = true;
        assert!(engine.identity(&engine.containers[0]).is_none());
        engine.truncated = false;
        engine.containers[0].id = "a".repeat(12);
        assert!(engine.identity(&engine.containers[0]).is_none());
    }
}
