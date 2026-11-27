//! The helper on a private bus with stand-ins for polkit and power-profiles-daemon.
#![allow(
    clippy::unwrap_used,
    reason = "Test fixtures report failures by panicking."
)]

use enzui_power_boost::service::{self, Config};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::zvariant::OwnedValue;

struct Bus {
    daemon: std::process::Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Bus {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("bus");
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>unix:path={}</listen><auth>EXTERNAL</auth>\
                 <policy context=\"default\"><allow user=\"*\"/><allow send_destination=\"*\"/>\
                 <allow receive_sender=\"*\"/><allow own=\"*\"/></policy></busconfig>",
                socket.display()
            ),
        )
        .unwrap();
        let daemon = std::process::Command::new("/usr/bin/dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--nopidfile"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Self {
            daemon,
            address: format!("unix:path={}", socket.display()),
            _dir: dir,
        }
    }

    async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

struct Authority {
    allow: bool,
    actions: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl Authority {
    fn check_authorization(
        &self,
        _subject: (String, HashMap<String, OwnedValue>),
        action: String,
        _details: HashMap<String, String>,
        _flags: u32,
        _cancellation: String,
    ) -> (bool, bool, HashMap<String, String>) {
        self.actions.lock().unwrap().push(action);
        (self.allow, false, HashMap::new())
    }
}

struct Profiles {
    active: String,
}

#[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
impl Profiles {
    #[zbus(property)]
    fn active_profile(&self) -> String {
        self.active.clone()
    }
}

struct Host {
    bus: Bus,
    sysfs: tempfile::TempDir,
    actions: Arc<Mutex<Vec<String>>>,
    _authority: zbus::Connection,
    profiles: zbus::Connection,
}

impl Host {
    async fn start(allow: bool, preferences: &[&str]) -> Self {
        let bus = Bus::start();
        let sysfs = tempfile::tempdir().unwrap();
        for (number, preference) in preferences.iter().enumerate() {
            let policy = sysfs.path().join(format!("cpufreq/policy{number}"));
            std::fs::create_dir_all(&policy).unwrap();
            std::fs::write(policy.join("energy_performance_preference"), preference).unwrap();
            std::fs::write(policy.join("scaling_governor"), "powersave\n").unwrap();
            std::fs::write(
                policy.join("energy_performance_available_preferences"),
                "default performance balance_performance balance_power power\n",
            )
            .unwrap();
        }
        let actions = Arc::new(Mutex::new(Vec::new()));
        let authority = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name("org.freedesktop.PolicyKit1")
            .unwrap()
            .serve_at(
                "/org/freedesktop/PolicyKit1/Authority",
                Authority {
                    allow,
                    actions: Arc::clone(&actions),
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        let profiles = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name("org.freedesktop.UPower.PowerProfiles")
            .unwrap()
            .serve_at(
                "/org/freedesktop/UPower/PowerProfiles",
                Profiles {
                    active: "balanced".into(),
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        Self {
            bus,
            sysfs,
            actions,
            _authority: authority,
            profiles,
        }
    }

    fn cpufreq(&self) -> PathBuf {
        self.sysfs.path().join("cpufreq")
    }

    fn marker(&self) -> PathBuf {
        self.sysfs.path().join("boost")
    }

    async fn serve(&self, idle: Duration) -> tokio::task::JoinHandle<zbus::Result<()>> {
        let connection = self.bus.connect().await;
        let config = Config {
            cpufreq: self.cpufreq(),
            marker: self.marker(),
            idle,
        };
        let task = tokio::spawn(service::run(connection, config));
        let client = self.bus.connect().await;
        let dbus = zbus::fdo::DBusProxy::new(&client).await.unwrap();
        for _ in 0..200 {
            if dbus
                .name_has_owner(service::NAME.try_into().unwrap())
                .await
                .unwrap()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        task
    }

    async fn set_profile(&self, profile: &str) {
        let server = self.profiles.object_server();
        let interface = server
            .interface::<_, Profiles>("/org/freedesktop/UPower/PowerProfiles")
            .await
            .unwrap();
        interface.get_mut().await.active = profile.to_owned();
    }

    fn preference(&self, number: u32) -> String {
        preference(&self.cpufreq(), number)
    }
}

fn preference(cpufreq: &Path, number: u32) -> String {
    std::fs::read_to_string(cpufreq.join(format!("policy{number}/energy_performance_preference")))
        .unwrap()
        .trim()
        .to_owned()
}

async fn boost(client: &zbus::Connection) -> String {
    client
        .call_method(
            Some(service::NAME),
            service::PATH,
            Some(service::NAME),
            "Boost",
            &(),
        )
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap()
}

async fn wait_for(cpufreq: &Path, number: u32, value: &str, within: Duration) -> Duration {
    let start = Instant::now();
    while preference(cpufreq, number) != value {
        assert!(
            start.elapsed() < within,
            "policy{number} never became {value}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    start.elapsed()
}

#[tokio::test]
async fn an_authorized_boost_is_restored_after_it_lapses_and_the_helper_then_exits() {
    let host = Host::start(true, &["balance_power", "balance_performance"]).await;
    let helper = host.serve(Duration::from_millis(800)).await;
    let client = host.bus.connect().await;
    assert_eq!(boost(&client).await, "boosted");
    assert_eq!(host.preference(0), "performance");
    assert_eq!(host.preference(1), "performance");
    assert_eq!(boost(&client).await, "busy");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(boost(&client).await, "extended");
    let restored = wait_for(&host.cpufreq(), 0, "balance_power", Duration::from_secs(3)).await;
    assert!(restored >= Duration::from_millis(1300), "{restored:?}");
    assert_eq!(host.preference(1), "balance_performance");
    assert!(!host.marker().exists());
    assert_eq!(*host.actions.lock().unwrap(), ["dev.enzui.power-boost"]);
    tokio::time::timeout(Duration::from_secs(3), helper)
        .await
        .expect("an idle helper exits")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_profile_change_during_a_boost_is_kept_and_performance_is_not_boosted() {
    let host = Host::start(true, &["balance_power"]).await;
    let _helper = host.serve(Duration::from_secs(30)).await;
    let client = host.bus.connect().await;
    assert_eq!(boost(&client).await, "boosted");
    // power-profiles-daemon writes the performance preference when its profile changes.
    host.set_profile("performance").await;
    std::fs::write(
        host.cpufreq().join("policy0/energy_performance_preference"),
        "performance",
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert_eq!(host.preference(0), "performance");
    assert!(!host.marker().exists());
    assert_eq!(boost(&client).await, "performance");
}

#[tokio::test]
async fn unauthorized_callers_change_nothing() {
    let host = Host::start(false, &["balance_power"]).await;
    let _helper = host.serve(Duration::from_secs(30)).await;
    let client = host.bus.connect().await;
    assert_eq!(boost(&client).await, "denied");
    assert_eq!(host.preference(0), "balance_power");
}
