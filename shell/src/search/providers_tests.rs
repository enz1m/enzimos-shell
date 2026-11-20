use super::*;
use std::sync::{Arc, Mutex};

const ENZUI: &str = "[enzUI Search Provider]\nDesktopId=io.github.enzui.Mail.desktop\nBusName=io.github.enzui.Mail\nObjectPath=/io/github/enzui/Mail/SearchProvider\nVersion=1\n";
const GNOME: &str = "[Shell Search Provider]\nDesktopId=org.gnome.Calendar.desktop\nBusName=org.gnome.Calendar\nObjectPath=/org/gnome/Calendar/SearchProvider\nVersion=2\n";

fn row(pairs: &[(&str, Value<'static>)]) -> HashMap<String, OwnedValue> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.try_to_owned().unwrap()))
        .collect()
}

#[derive(Default, Clone)]
struct Calls(Arc<Mutex<Vec<String>>>);

struct Mail(Calls, Duration);

#[zbus::interface(name = "dev.enzui.SearchProvider1")]
impl Mail {
    async fn search(
        &self,
        query: String,
        options: HashMap<String, OwnedValue>,
    ) -> Vec<HashMap<String, OwnedValue>> {
        tokio::time::sleep(self.1).await;
        let limit = options
            .get("limit")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("search {query} limit {limit}"));
        (0..8)
            .map(|i| {
                row(&[
                    ("id", Value::from(format!("msg-{i}"))),
                    ("title", Value::from(format!("{query} {i}"))),
                    ("subtitle", Value::from("From Ana")),
                ])
            })
            .collect()
    }
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn activate(&self, id: String, query: String, options: HashMap<String, OwnedValue>) {
        let mut calls = self.0.0.lock().unwrap();
        calls.push(format!("activate {id} {query}"));
        if let Some(token) = options
            .get("activation-token")
            .and_then(|value| <&str>::try_from(value).ok())
        {
            calls.push(format!("token {token}"));
        }
    }
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn launch_search(&self, query: String, options: HashMap<String, OwnedValue>) {
        let mut calls = self.0.0.lock().unwrap();
        calls.push(format!("launch {query}"));
        if let Some(token) = options
            .get("activation-token")
            .and_then(|value| <&str>::try_from(value).ok())
        {
            calls.push(format!("token {token}"));
        }
    }
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
}

struct Calendar(Calls);

#[zbus::interface(name = "org.gnome.Shell.SearchProvider2")]
impl Calendar {
    fn get_initial_result_set(&self, terms: Vec<String>) -> Vec<String> {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("initial {}", terms.join("+")));
        (0..9).map(|i| format!("event-{i}")).collect()
    }
    fn get_subsearch_result_set(&self, _previous: Vec<String>, terms: Vec<String>) -> Vec<String> {
        self.get_initial_result_set(terms)
    }
    fn get_result_metas(&self, ids: Vec<String>) -> Vec<HashMap<String, OwnedValue>> {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("metas {}", ids.len()));
        ids.iter()
            .map(|id| {
                row(&[
                    ("id", Value::from(id.clone())),
                    ("name", Value::from(format!("Event {id}"))),
                    ("description", Value::from("Today")),
                ])
            })
            .collect()
    }
    fn activate_result(&self, id: String, terms: Vec<String>, _timestamp: u32) {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("activate {id} {}", terms.join("+")));
    }
    fn launch_search(&self, terms: Vec<String>, _timestamp: u32) {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("launch {}", terms.join("+")));
    }
}

use crate::test_bus::Bus;

async fn serve(
    bus: &Bus,
    declaration: &Declaration,
    provider: impl zbus::object_server::Interface,
) -> zbus::Connection {
    bus.serve(&declaration.bus_name, &declaration.object_path, provider)
        .await
}

#[tokio::test]
async fn providers_async_behaviour() {
    // An enzui provider answers bounded and opens what was chosen.
    {
        let calls = Calls::default();
        let declaration = parse(ENZUI, Protocol::Enzui).unwrap();
        let bus = Bus::start();
        let client = bus.client().await;
        assert!(
            !reachable(&client, &declaration).await,
            "nothing owns the name yet"
        );
        let _server = serve(&bus, &declaration, Mail(calls.clone(), Duration::ZERO)).await;
        assert!(reachable(&client, &declaration).await);
        let found = search(&client, &declaration, "invoice").await.unwrap();
        assert_eq!(
            found.len(),
            8,
            "Search shows the first SHOWN; the bound is LIMIT"
        );
        assert_eq!(
            (found[0].id.as_str(), found[0].title.as_str()),
            ("msg-0", "invoice 0")
        );
        activate(&client, &declaration, "msg-3", "invoice")
            .await
            .unwrap();
        launch_search(&client, &declaration, "invoice")
            .await
            .unwrap();
        assert_eq!(
            *calls.0.lock().unwrap(),
            [
                format!("search invoice limit {SHOWN}"),
                "activate msg-3 invoice".into(),
                "launch invoice".into()
            ]
        );
    }
    // A gnome provider is asked for ids then their details.
    {
        let calls = Calls::default();
        let declaration = parse(GNOME, Protocol::Gnome).unwrap();
        let bus = Bus::start();
        let client = bus.client().await;
        let _server = serve(&bus, &declaration, Calendar(calls.clone())).await;
        let found = search(&client, &declaration, "team  lunch").await.unwrap();
        assert_eq!(
            found.len(),
            SHOWN,
            "only the shown results' details are asked for"
        );
        assert_eq!(
            (found[0].title.as_str(), found[0].subtitle.as_str()),
            ("Event event-0", "Today")
        );
        activate(&client, &declaration, "event-1", "team lunch")
            .await
            .unwrap();
        launch_search(&client, &declaration, "team lunch")
            .await
            .unwrap();
        assert_eq!(
            *calls.0.lock().unwrap(),
            [
                "initial team+lunch".to_owned(),
                format!("metas {SHOWN}"),
                "activate event-1 team+lunch".into(),
                "launch team+lunch".into()
            ]
        );
    }
    // A slow provider misses the deadline instead of holding search.
    {
        let declaration = parse(ENZUI, Protocol::Enzui).unwrap();
        let bus = Bus::start();
        let client = bus.client().await;
        let _server = serve(
            &bus,
            &declaration,
            Mail(Calls::default(), Duration::from_secs(5)),
        )
        .await;
        let started = std::time::Instant::now();
        assert!(matches!(
            search(&client, &declaration, "slow").await,
            Err(Silence::Failed(_))
        ));
        assert!(started.elapsed() < DEADLINE + Duration::from_millis(500));
    }
}
