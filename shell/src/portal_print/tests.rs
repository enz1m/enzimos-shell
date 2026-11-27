use super::*;
use crate::desktop_portal::{BUS_NAME, PATH, serve_on};

fn caller(sender: &str, scope: &str, app: &str) -> Caller {
    Caller {
        sender: Some(OwnedUniqueName::try_from(sender).unwrap()),
        app: app.into(),
        request_scope: scope.into(),
    }
}

fn fixture_selection() -> Selection {
    let printer = Printer {
        name: "Test".into(),
        label: "Test".into(),
        is_default: true,
        papers: vec![Paper::default()],
        default_paper: 0,
        duplex: vec![],
        default_duplex: None,
        color: vec![],
        default_color: None,
        max_copies: 99,
    };
    Selection::from_hints(&[printer], &Dict::new(), &Dict::new(), false, false).unwrap()
}

#[test]
fn tokens_are_bound_to_the_sender_app_and_request_scope_and_consumed_once() {
    let mut tokens = Tokens::default();
    let now = Instant::now();
    let owner = || caller(":1.2", "/request/1_5", "org.example.App");
    let token = tokens.issue(owner(), fixture_selection(), now).unwrap();
    assert_ne!(token, 0);
    for other in [
        caller(":1.7", "/request/1_5", "org.example.App"),
        caller(":1.2", "/request/1_6", "org.example.App"),
        caller(":1.2", "/request/1_5", "org.example.Other"),
    ] {
        assert!(tokens.take(token, &other, now).is_none());
    }
    assert!(tokens.take(token, &owner(), now).is_some());
    assert!(tokens.take(token, &owner(), now).is_none());

    let token = tokens.issue(owner(), fixture_selection(), now).unwrap();
    assert!(tokens.take(token, &owner(), now + TOKEN_TTL).is_none());
    assert!(tokens.0.is_empty());
}

const INTERFACE: &str = "org.freedesktop.impl.portal.Print";
const REQUEST_SCOPE: &str = "/org/freedesktop/portal/desktop/request/1_2";

async fn ask_event(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
) -> (DialogRequest, oneshot::Sender<DialogAnswer>) {
    let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("dialog timeout")
        .expect("dialog event");
    match event {
        Event::Print(request, reply) => (request, reply),
        other => panic!("unexpected portal event: {other:?}"),
    }
}

fn prepare(client: &zbus::Connection, request: &'static str) -> tokio::task::JoinHandle<Response> {
    let client = client.clone();
    tokio::spawn(async move {
        let handle = format!("{REQUEST_SCOPE}/{request}");
        client
            .call_method(
                Some(BUS_NAME),
                PATH,
                Some(INTERFACE),
                "PreparePrint",
                &(
                    ObjectPath::try_from(handle.as_str()).unwrap(),
                    "org.example.Print",
                    "wayland-example",
                    "Choose printer",
                    Dict::new(),
                    Dict::new(),
                    Dict::from([
                        ("has_current_page".into(), true.into()),
                        ("modal".into(), false.into()),
                    ]),
                ),
            )
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap()
    })
}

fn submit(
    client: &zbus::Connection,
    request: &'static str,
    file: &str,
    token: Option<u32>,
) -> tokio::task::JoinHandle<Response> {
    let client = client.clone();
    let file = std::fs::File::open(file).unwrap();
    tokio::spawn(async move {
        let handle = format!("{REQUEST_SCOPE}/{request}");
        let fd = zbus::zvariant::OwnedFd::from(OwnedFd::from(file));
        let options = token
            .map(|token| Dict::from([("token".into(), token.into())]))
            .unwrap_or_default();
        client
            .call_method(
                Some(BUS_NAME),
                PATH,
                Some(INTERFACE),
                "Print",
                &(
                    ObjectPath::try_from(handle.as_str()).unwrap(),
                    "org.example.Print",
                    "",
                    "Portal test document",
                    fd,
                    options,
                ),
            )
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap()
    })
}

#[tokio::test]
#[ignore = "requires tools/test-session/run and the private CUPS PDF fixture"]
async fn print_interface_on_private_bus() {
    assert!(
        std::env::var_os("TS_ROOT").is_some(),
        "run in tools/test-session"
    );
    let file = std::env::var("PORTAL_PRINT_TEST_PDF").expect("fixture PDF path");
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let _backend = serve_on(zbus::connection::Builder::session().unwrap(), tx)
        .await
        .unwrap();
    let client = zbus::Connection::session().await.unwrap();

    let pending = prepare(&client, "prepare");
    let (request, reply) = ask_event(&mut events).await;
    assert_eq!(request.parent_window, "wayland-example");
    assert!(!request.modal);
    assert!(request.has_current_page);
    let mut choice = request.selection.expect("CUPS default printer");
    assert_eq!(choice.queue(), "Sandbox_PDF");
    assert!(
        !request.printers.unwrap()[choice.printer_index]
            .papers()
            .is_empty()
    );
    choice.copies = "2".into();
    reply.send(DialogAnswer::Chosen(Box::new(choice))).unwrap();
    let (response, mut results) = pending.await.unwrap();
    assert_eq!(response, 0);
    let settings = Dict::try_from(results.remove("settings").unwrap()).unwrap();
    assert_eq!(<&str>::try_from(&settings["n-copies"]).unwrap(), "2");
    let page_setup = Dict::try_from(results.remove("page-setup").unwrap()).unwrap();
    assert!(f64::try_from(&page_setup["Width"]).unwrap() > 0.0);
    let token = u32::try_from(results.remove("token").unwrap()).unwrap();

    let (response, results) = submit(&client, "prepared", &file, Some(token))
        .await
        .unwrap();
    assert_eq!(response, 0, "{results:?}");
    assert!(u32::try_from(&results["job-id"]).unwrap() > 0);

    let pending = submit(&client, "replay", &file, Some(token));
    let (_, reply) = ask_event(&mut events).await;
    drop(reply);
    assert_eq!(pending.await.unwrap().0, 1);

    let pending = prepare(&client, "close");
    let (request, _reply) = ask_event(&mut events).await;
    client
        .call_method(
            Some(BUS_NAME),
            format!("{REQUEST_SCOPE}/close").as_str(),
            Some("org.freedesktop.impl.portal.Request"),
            "Close",
            &(),
        )
        .await
        .unwrap();
    assert!(matches!(events.recv().await, Some(Event::Close(id)) if id == request.id));
    assert_eq!(pending.await.unwrap().0, 1);

    let pending = submit(&client, "direct", &file, None);
    let (request, reply) = ask_event(&mut events).await;
    assert!(request.modal);
    assert!(!request.has_current_page);
    reply
        .send(DialogAnswer::Chosen(Box::new(request.selection.unwrap())))
        .unwrap();
    let (response, results) = pending.await.unwrap();
    assert_eq!(response, 0, "{results:?}");

    let pending = submit(
        &client,
        "invalid_fd",
        &std::env::var("TS_ROOT").unwrap(),
        None,
    );
    let (request, reply) = ask_event(&mut events).await;
    reply
        .send(DialogAnswer::Chosen(Box::new(request.selection.unwrap())))
        .unwrap();
    let (response, results) = pending.await.unwrap();
    assert_eq!(response, 2);
    assert!(results.contains_key("error"));
}

#[tokio::test]
#[ignore = "requires tools/test-session/run and an empty or unavailable CUPS scheduler"]
async fn missing_printer_returns_a_print_error() {
    assert!(
        std::env::var_os("TS_ROOT").is_some(),
        "run in tools/test-session"
    );
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let _backend = serve_on(zbus::connection::Builder::session().unwrap(), tx)
        .await
        .unwrap();
    let client = zbus::Connection::session().await.unwrap();
    let pending = submit(
        &client,
        "missing",
        &std::env::var("PORTAL_PRINT_TEST_PDF").unwrap(),
        None,
    );
    let (request, reply) = ask_event(&mut events).await;
    assert!(request.selection.is_none());
    reply.send(DialogAnswer::Unavailable).unwrap();
    assert_eq!(pending.await.unwrap().0, 2);
}

#[test]
fn common_paper_sizes_use_familiar_names() {
    assert_eq!(
        super::cups::common_paper_label("na_letter_8.5x11in"),
        Some("Letter")
    );
    assert_eq!(
        super::cups::common_paper_label("iso_a4_210x297mm"),
        Some("A4")
    );
    assert_eq!(
        super::cups::common_paper_label("om_small-photo_100x150mm"),
        None
    );
}
