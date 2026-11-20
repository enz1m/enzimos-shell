use super::*;
use std::sync::{Arc, Mutex};

fn caps() -> Value {
    json!({"queries":["backup-candidates"],"backup_setup":{"schema":1,"control":"BackupSetupControl(s)->s","input":"BackupSetupInput(sh)->s","runtime_qualified":true,"sealed_input_only":true}})
}
fn response(id: &str, mode: Mode, state: &str, digest: &str) -> Value {
    json!({"schema":1,"request_id":id,"mode":mode.key(),"state":state,"plan_sha256":digest,"attempts":0,"retryable":false,"cancellation_available":state=="planned","automatic_backup_enabled":false})
}
struct Service {
    calls: Arc<Mutex<Vec<(String, Value, bool)>>>,
}
#[zbus::interface(name = "org.enzimos.System1")]
impl Service {
    fn get_capabilities(&self) -> String {
        caps().to_string()
    }
    fn backup_setup_control(&self, body: String) -> String {
        let value: Value = serde_json::from_str(&body).unwrap();
        self.calls
            .lock()
            .unwrap()
            .push(("control".into(), value.clone(), false));
        response(
            value["request_id"].as_str().unwrap(),
            Mode::Adopt,
            "planned",
            &"a".repeat(64),
        )
        .to_string()
    }
    fn backup_setup_input(&self, body: String, fd: zbus::zvariant::OwnedFd) -> String {
        use std::os::fd::{AsFd, AsRawFd};
        let value: Value = serde_json::from_str(&body).unwrap();
        let seals = rustix::fs::fcntl_get_seals(fd.as_fd()).unwrap();
        let stat = rustix::fs::fstat(fd.as_fd()).unwrap();
        let good = seals.contains(
            rustix::fs::SealFlags::WRITE
                | rustix::fs::SealFlags::SEAL
                | rustix::fs::SealFlags::GROW
                | rustix::fs::SealFlags::SHRINK,
        ) && stat.st_nlink == 0
            && stat.st_mode & 0o077 == 0
            && fd.as_raw_fd() >= 0;
        self.calls
            .lock()
            .unwrap()
            .push(("input".into(), value.clone(), good));
        let mut result = response(
            value["request_id"].as_str().unwrap(),
            Mode::Adopt,
            "awaiting-approval",
            &"a".repeat(64),
        );
        result["attempts"] = json!(1);
        result.to_string()
    }
}
#[tokio::test]
async fn backup_setup_async_behaviour() {
    // Real bus metadata excludes secret and inputs are sealed.
    {
        let bus = crate::test_bus::Bus::start();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let _service = bus
            .serve(
                NAME,
                PATH,
                Service {
                    calls: std::sync::Arc::clone(&calls),
                },
            )
            .await;
        let client = Client::connect(bus.client().await).await.unwrap();
        let mut job = Job::new(client, Mode::Adopt);
        let status = job.plan(&"b".repeat(64)).await.unwrap();
        job.observe(&status);
        assert_eq!(job.reviewed.as_deref(), Some("a".repeat(64).as_str()));
        let result = job
            .input(SecretText::new("synthetic private café".into()))
            .await
            .unwrap();
        assert_eq!(result.state, "awaiting-approval");
        let records = calls.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[1].2);
        assert_eq!(records[0].1.as_object().unwrap().len(), 7);
        assert_eq!(records[0].1["mode"], "adopt");
        assert_eq!(records[0].1["allow_interaction"], true);
        assert_eq!(records[1].1.as_object().unwrap().len(), 4);
        assert!(!format!("{records:?}").contains("synthetic"));
        assert_eq!(records[1].1["plan_sha256"], "a".repeat(64));
    }
    // Replaced service never receives the existing job mutation.
    {
        let bus = crate::test_bus::Bus::start();
        let old_calls = Arc::new(Mutex::new(Vec::new()));
        let service = bus.serve(NAME, PATH, Service { calls: old_calls }).await;
        let client = Client::connect(bus.client().await).await.unwrap();
        let mut job = Job::new(client, Mode::Adopt);
        let status = job.plan(&"b".repeat(64)).await.unwrap();
        job.observe(&status);
        service.release_name(NAME).await.unwrap();
        let new_calls = Arc::new(Mutex::new(Vec::new()));
        let _replacement = bus
            .serve(
                NAME,
                PATH,
                Service {
                    calls: std::sync::Arc::clone(&new_calls),
                },
            )
            .await;
        assert!(
            job.input(SecretText::new("synthetic private".into()))
                .await
                .is_err()
        );
        assert!(new_calls.lock().unwrap().is_empty());
        service.close().await.unwrap();
    }
}
