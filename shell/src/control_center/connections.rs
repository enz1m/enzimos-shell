use super::{Message, Shell, detail_panel, option_row, section_label};
use enzui_shell::{
    i18n::tr,
    settings::QuickAction,
    system_settings::{
        AccessPoint, Bluetooth, InterfaceKind, LinkState, Network, Panel, Reading, Request, Secret,
        SystemSettings, pairing,
    },
};
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    text::enz_text,
    text_field::enz_text_field,
};
use iced::{
    Element, Task,
    widget::{column, container, row},
};

#[derive(Debug, Clone)]
pub(crate) enum Event {
    Refresh(bool),
    DisconnectWifi,
    BluetoothDevice(String, bool),
    ChooseNetwork(String),
    Password(Secret),
    Identity(String),
    Join,
    CancelJoin,
    Pair(String),
    Pin(Secret),
    Answer(pairing::Answer),
    Prompt(u64, pairing::Prompt),
    Finished(u64, Result<(), String>, Result<Snapshot, String>),
}

#[derive(Debug, Clone)]
pub(crate) enum Snapshot {
    Network(Box<Network>),
    Bluetooth(Bluetooth),
}

#[derive(Default)]
pub(crate) struct State {
    epoch: u64,
    flight: Option<u64>,
    reading: Option<Snapshot>,
    error: Option<String>,
    join: Option<Join>,
    prompt: Option<pairing::Prompt>,
    pin: String,
}

struct Join {
    ssid: String,
    enterprise: bool,
    identity: String,
    password: Secret,
}

impl Join {
    fn ready(&self) -> bool {
        !self.password.0.chars().any(char::is_control)
            && if self.enterprise {
                !self.identity.trim().is_empty() && !self.password.0.is_empty()
            } else {
                (8..=64).contains(&self.password.0.len())
            }
    }
}

impl State {
    pub(crate) fn busy(&self) -> bool {
        self.flight.is_some()
    }

    pub(crate) fn reset(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.reading = None;
        self.error = None;
        self.clear_forms();
    }

    pub(crate) fn clear_forms(&mut self) {
        self.join = None;
        self.pin.clear();
        if let Some(prompt) = self.prompt.take() {
            prompt.answer(pairing::Answer::Reject);
        }
    }
}

fn send(event: Event) -> Message {
    Message::ControlConnections(event)
}

fn active(shell: &Shell) -> Option<QuickAction> {
    shell.control_details.filter(|action| {
        matches!(action, QuickAction::Network | QuickAction::Bluetooth)
            && shell.control_details_open
            && shell.control_visible
    })
}

async fn read(
    backend: &SystemSettings,
    action: QuickAction,
    scan: bool,
) -> Result<Snapshot, String> {
    if action == QuickAction::Network {
        return backend
            .control_network(scan)
            .await
            .map(|network| Snapshot::Network(Box::new(network)));
    }
    match backend.read(Panel::Bluetooth).await {
        Reading::Bluetooth(devices) => Ok(Snapshot::Bluetooth(devices)),
        _ => Err(tr("shell-bluetooth-is-not-running").into()),
    }
}

fn backend(shell: &Shell) -> SystemSettings {
    let mut settings = SystemSettings::from_environment();
    settings.demo = false;
    settings.writes_allowed = shell.system.system_writes;
    settings
}

pub(super) fn radio_toggle_allowed(shell: &Shell) -> bool {
    shell.system.system_writes && !shell.control_connections.busy()
}

fn permitted(shell: &Shell) -> bool {
    shell.system.system_writes
        && !shell.control_placing
        && !shell.system_action_busy
        && shell.confirm.is_none()
        && !shell.power_menu
        && !shell.accessibility_menu
}

fn start(
    shell: &mut Shell,
    action: QuickAction,
    request: Option<Request>,
    scan: bool,
) -> Task<Message> {
    let backend = backend(shell);
    let state = &mut shell.control_connections;
    let epoch = state.epoch;
    state.flight = Some(epoch);
    state.error = None;
    Task::perform(
        async move {
            let result = match request {
                Some(request) => backend.apply(&request).await,
                None => Ok(()),
            };
            let reading = read(&backend, action, scan).await;
            (result, reading)
        },
        move |(result, reading)| send(Event::Finished(epoch, result, reading)),
    )
}

fn pair(shell: &mut Shell, address: String) -> Task<Message> {
    use iced::futures::{SinkExt, channel::mpsc::Sender};

    let backend = backend(shell);
    let state = &mut shell.control_connections;
    let epoch = state.epoch;
    state.flight = Some(epoch);
    state.error = None;
    Task::run(
        iced::stream::channel(8, async move |mut events: Sender<Event>| {
            let asks = std::sync::Mutex::new(events.clone());
            let result = backend
                .pair_device(&address, move |prompt| {
                    if asks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .try_send(Event::Prompt(epoch, prompt.clone()))
                        .is_err()
                    {
                        prompt.answer(pairing::Answer::Reject);
                    }
                })
                .await;
            let reading = read(&backend, QuickAction::Bluetooth, false).await;
            let _ = events.send(Event::Finished(epoch, result, reading)).await;
        }),
        send,
    )
}

fn finished(
    shell: &mut Shell,
    epoch: u64,
    result: Result<(), String>,
    reading: Result<Snapshot, String>,
) -> Task<Message> {
    let action = active(shell);
    let state = &mut shell.control_connections;
    if state.flight != Some(epoch) {
        return Task::none();
    }
    state.flight = None;
    let Some(action) = action else {
        return Task::none();
    };
    if epoch != state.epoch {
        return start(shell, action, None, false);
    }
    state.prompt = None;
    state.pin.clear();
    if result.is_ok() {
        state.join = None;
    }
    state.error = result.err();
    match reading {
        Ok(reading) => state.reading = Some(reading),
        Err(error) if state.error.is_none() => state.error = Some(error),
        Err(_) => {}
    }
    Task::none()
}

pub(crate) fn update(shell: &mut Shell, event: Event) -> Task<Message> {
    let action = active(shell);
    let allowed = permitted(shell);
    let state = &mut shell.control_connections;
    match event {
        Event::Finished(epoch, result, reading) => finished(shell, epoch, result, reading),
        Event::Prompt(epoch, prompt) => {
            if allowed
                && action == Some(QuickAction::Bluetooth)
                && epoch == state.epoch
                && state.flight == Some(epoch)
            {
                state.pin.clear();
                if let Some(previous) = state.prompt.replace(prompt) {
                    previous.answer(pairing::Answer::Reject);
                }
            } else {
                prompt.answer(pairing::Answer::Reject);
            }
            Task::none()
        }
        _ => {
            let Some(action) = action else {
                return Task::none();
            };
            match event {
                Event::Refresh(scan) if state.flight.is_none() && state.join.is_none() => {
                    let request = (scan && allowed && action == QuickAction::Bluetooth)
                        .then_some(Request::BluetoothSearch);
                    start(shell, action, request, scan && allowed)
                }
                Event::Pin(pin) if state.prompt.is_some() => {
                    state.pin = pin.0.chars().take(16).collect();
                    Task::none()
                }
                Event::Answer(answer) => {
                    if let Some(prompt) = state.prompt.take() {
                        prompt.answer(answer);
                    }
                    state.pin.clear();
                    Task::none()
                }
                Event::CancelJoin if state.flight.is_none() => {
                    state.join = None;
                    state.error = None;
                    Task::none()
                }
                _ if !allowed || state.flight.is_some() => Task::none(),
                Event::DisconnectWifi
                    if action == QuickAction::Network
                        && matches!(&state.reading, Some(Snapshot::Network(network)) if can_disconnect_wifi(network)) =>
                {
                    start(shell, action, Some(Request::WifiDisconnect), false)
                }
                Event::BluetoothDevice(address, connect)
                    if action == QuickAction::Bluetooth
                        && matches!(&state.reading, Some(Snapshot::Bluetooth(bluetooth))
                        if bluetooth.devices.iter().any(|d| d.address == address && d.connected != connect)) =>
                {
                    start(
                        shell,
                        action,
                        Some(Request::BluetoothDevice(address, connect)),
                        false,
                    )
                }
                Event::Pair(address)
                    if action == QuickAction::Bluetooth
                        && matches!(&state.reading, Some(Snapshot::Bluetooth(bluetooth))
                        if bluetooth.nearby.iter().any(|d| d.address == address)) =>
                {
                    pair(shell, address)
                }
                Event::ChooseNetwork(ssid) => {
                    let Some(Snapshot::Network(network)) = &state.reading else {
                        return Task::none();
                    };
                    let Some(point) = network
                        .access_points
                        .iter()
                        .find(|point| point.ssid == ssid && !point.active)
                    else {
                        return Task::none();
                    };
                    if point.saved || !point.secure {
                        return start(
                            shell,
                            action,
                            Some(Request::WifiJoin {
                                ssid,
                                password: None,
                                identity: None,
                                hidden: false,
                            }),
                            false,
                        );
                    }
                    state.error = None;
                    state.join = Some(Join {
                        ssid,
                        enterprise: point.enterprise,
                        identity: String::new(),
                        password: Secret(String::new()),
                    });
                    iced::widget::operation::focus(if point.enterprise {
                        "quick-wifi-identity"
                    } else {
                        "quick-wifi-password"
                    })
                }
                Event::Password(password) => {
                    if let Some(join) = &mut state.join {
                        join.password = password;
                    }
                    Task::none()
                }
                Event::Identity(identity) => {
                    if let Some(join) = &mut state.join {
                        join.identity = identity;
                    }
                    Task::none()
                }
                Event::Join => {
                    let Some(join) = state.join.as_mut().filter(|join| join.ready()) else {
                        return Task::none();
                    };
                    let request = Request::WifiJoin {
                        ssid: join.ssid.clone(),
                        password: Some(std::mem::replace(
                            &mut join.password,
                            Secret(String::new()),
                        )),
                        identity: join.enterprise.then(|| join.identity.clone()),
                        hidden: false,
                    };
                    start(shell, action, Some(request), false)
                }
                _ => Task::none(),
            }
        }
    }
}

fn note(shell: &Shell, text: impl Into<String>) -> Element<'static, Message> {
    let p = shell.window_tokens(shell.control_center).palette;
    container(
        enz_text(text.into())
            .size_px((f32::from(shell.metrics.text) - 2.0).max(10.0))
            .muted_on(p.surface_alt)
            .wrap(true)
            .into_element(),
    )
    .padding([4, 8])
    .into()
}

fn settings(page: &'static str) -> Element<'static, Message> {
    enz_button(tr("shell-settings"), Message::SettingsPage(page))
        .kind(ButtonKind::Quiet)
        .icon("settings")
        .into_element()
}

fn small_button(label: &'static str, event: Event, enabled: bool) -> Element<'static, Message> {
    enz_button(label, send(event))
        .kind(ButtonKind::Quiet)
        .size(ButtonSize::Sm)
        .enabled(enabled)
        .into_element()
}

fn wifi_icon(point: &AccessPoint) -> &'static str {
    match point.signal {
        0..=24 => "wifi-zero",
        25..=49 => "wifi-low",
        50..=74 => "wifi-high",
        _ => "wifi",
    }
}

fn can_disconnect_wifi(network: &Network) -> bool {
    network
        .interfaces
        .iter()
        .filter(|link| link.kind == InterfaceKind::Wifi && link.state == LinkState::Connected)
        .count()
        == 1
}

pub(super) fn network(shell: &Shell) -> Element<'_, Message> {
    let state = &shell.control_connections;
    let busy = state.busy();
    let writes = permitted(shell) && !busy;
    let mut content = column![];
    if let Some(error) = &state.error {
        content = content.push(note(shell, error.clone()));
    }
    if busy {
        content = content.push(note(shell, tr("shell-checking")));
    }
    let mut actions = vec![settings("network/wifi")];
    if let Some(Snapshot::Network(network)) = &state.reading {
        if let Some(join) = &state.join {
            let mut form = column![
                section_label(shell, tr("shell-wi-fi")),
                note(shell, join.ssid.clone())
            ]
            .spacing(4);
            if join.enterprise {
                form = form.push(
                    enz_text_field(tr("shell-username"), "", &join.identity, |value| {
                        send(Event::Identity(value))
                    })
                    .enabled(writes)
                    .id("quick-wifi-identity".into())
                    .into_element(),
                );
            }
            form = form.push(
                enz_text_field(tr("shell-password"), "", &join.password.0, |value| {
                    send(Event::Password(Secret(value)))
                })
                .secure(true)
                .enabled(writes)
                .on_submit(send(Event::Join))
                .id("quick-wifi-password".into())
                .into_element(),
            );
            content = content.push(form);
            actions = vec![
                small_button(tr("shell-cancel"), Event::CancelJoin, !busy),
                small_button(tr("shell-connect"), Event::Join, writes && join.ready()),
            ];
        } else {
            for link in network
                .interfaces
                .iter()
                .filter(|link| link.kind == InterfaceKind::Ethernet)
            {
                let connected = link.state == LinkState::Connected;
                content = content.push(option_row(
                    shell,
                    format!("quick-wired-{}", link.device),
                    "ethernet-port",
                    if link.connection.is_empty() {
                        tr("shell-ethernet-2").into()
                    } else {
                        link.connection.clone()
                    },
                    Some(link.state.label().into()),
                    connected,
                    false,
                    Message::RegionInterior,
                ));
            }
            if network.has_wifi() {
                content = content.push(section_label(shell, tr("shell-wi-fi")));
                match network.wifi_radio {
                    Some(true) => {
                        if network.access_points.is_empty() && !busy {
                            content = content.push(note(shell, tr("shell-no-networks-found")));
                        }
                        for point in &network.access_points {
                            let label = if point.active {
                                "shell-connected"
                            } else if point.saved {
                                "shell-saved"
                            } else if point.secure {
                                "shell-secured"
                            } else {
                                "shell-open-network"
                            };
                            let item = option_row(
                                shell,
                                format!("quick-wifi-{}", point.ssid),
                                wifi_icon(point),
                                point.ssid.clone(),
                                Some(tr(label).into()),
                                point.active,
                                writes,
                                send(Event::ChooseNetwork(point.ssid.clone())),
                            );
                            content = if point.active && can_disconnect_wifi(network) {
                                content.push(
                                    row![
                                        item,
                                        small_button(
                                            tr("shell-disconnect"),
                                            Event::DisconnectWifi,
                                            writes
                                        )
                                    ]
                                    .align_y(iced::Alignment::Center),
                                )
                            } else {
                                content.push(item)
                            };
                        }
                        actions.insert(
                            0,
                            small_button(tr("shell-refresh"), Event::Refresh(true), !busy),
                        );
                    }
                    Some(false) => content = content.push(note(shell, tr("shell-off"))),
                    None => {
                        content =
                            content.push(note(shell, tr("shell-network-settings-are-unavailable")));
                    }
                }
            }
        }
    } else if !busy && state.error.is_some() {
        actions.insert(
            0,
            small_button(tr("shell-refresh"), Event::Refresh(false), true),
        );
    }
    detail_panel(shell, content, actions)
}

pub(super) fn bluetooth(shell: &Shell) -> Element<'_, Message> {
    let state = &shell.control_connections;
    let busy = state.busy();
    let writes = permitted(shell) && !busy;
    let mut content = column![];
    let mut actions = vec![settings("bluetooth/devices")];
    if let Some(error) = &state.error {
        content = content.push(note(shell, error.clone()));
    }
    if busy && state.prompt.is_none() {
        content = content.push(note(shell, tr("shell-checking")));
    }
    if let Some(prompt) = &state.prompt {
        content = content.push(pairing::prompt_view(
            prompt,
            &state.pin,
            shell.window_tokens(shell.control_center).palette,
            |pin| send(Event::Pin(Secret(pin))),
            |answer| send(Event::Answer(answer)),
            |text| note(shell, text),
        ));
    } else if let Some(Snapshot::Bluetooth(devices)) = &state.reading {
        if devices
            .adapter
            .as_ref()
            .is_some_and(|adapter| adapter.powered)
        {
            content = content.push(section_label(shell, tr("shell-paired-devices")));
            if devices.devices.is_empty() {
                content = content.push(note(shell, tr("shell-no-paired-devices")));
            }
            for device in &devices.devices {
                let request = Event::BluetoothDevice(device.address.clone(), !device.connected);
                let item = option_row(
                    shell,
                    format!("quick-bluetooth-{}", device.address),
                    device.kind.icon(),
                    device.name.clone(),
                    Some(
                        tr(if device.connected {
                            "shell-connected"
                        } else {
                            "shell-not-connected"
                        })
                        .into(),
                    ),
                    device.connected,
                    writes,
                    send(request.clone()),
                );
                content = if device.connected {
                    content.push(
                        row![item, small_button(tr("shell-disconnect"), request, writes)]
                            .align_y(iced::Alignment::Center),
                    )
                } else {
                    content.push(item)
                };
            }
            if !devices.nearby.is_empty() {
                content = content.push(section_label(shell, tr("shell-nearby-devices")));
                for device in &devices.nearby {
                    content = content.push(option_row(
                        shell,
                        format!("quick-pair-{}", device.address),
                        device.kind.icon(),
                        device.name.clone(),
                        Some(tr("shell-pair").into()),
                        false,
                        writes,
                        send(Event::Pair(device.address.clone())),
                    ));
                }
            }
            actions.insert(
                0,
                if shell.system.system_writes {
                    small_button(tr("shell-search"), Event::Refresh(true), writes)
                } else {
                    small_button(tr("shell-refresh"), Event::Refresh(false), !busy)
                },
            );
        } else {
            let key = if devices.adapter.is_some() {
                "shell-turn-bluetooth-on-to-add-devices"
            } else {
                "shell-no-bluetooth-adapter"
            };
            content = content.push(note(shell, tr(key)));
        }
    }
    detail_panel(shell, content, actions)
}

#[cfg(test)]
#[test]
fn changing_panels_drops_credentials_and_late_results() {
    use enzui_shell::compositor::Compositor;
    use std::path::PathBuf;

    let compositor = Compositor::new(PathBuf::from("/usr/bin/true"), "fixture".into()).unwrap();
    let (mut shell, _) = Shell::new(compositor, PathBuf::from("/"));
    shell.control_visible = true;
    shell.control_details_open = true;
    shell.control_details = Some(QuickAction::Network);
    shell.system.system_writes = true;
    let wifi = Snapshot::Network(Box::new(Network {
        access_points: vec![AccessPoint {
            ssid: "Protected".into(),
            signal: 80,
            secure: true,
            enterprise: false,
            active: false,
            saved: false,
        }],
        ..Network::default()
    }));
    shell.control_connections.reading = Some(wifi.clone());
    let _ = update(&mut shell, Event::ChooseNetwork("Protected".into()));
    let _ = update(&mut shell, Event::Password(Secret("test-password".into())));
    assert!(shell.control_connections.join.as_ref().unwrap().ready());

    let _ = update(&mut shell, Event::Join);
    let epoch = shell.control_connections.epoch;
    assert_eq!(shell.control_connections.flight, Some(epoch));
    assert!(
        shell
            .control_connections
            .join
            .as_ref()
            .unwrap()
            .password
            .0
            .is_empty()
    );

    shell.control_connections.reset();
    shell.control_details = Some(QuickAction::Bluetooth);
    let _ = update(
        &mut shell,
        Event::Finished(epoch, Err("old request".into()), Ok(wifi)),
    );
    assert!(shell.control_connections.join.is_none());
    assert!(shell.control_connections.reading.is_none());
    assert!(shell.control_connections.error.is_none());
    assert_eq!(
        shell.control_connections.flight,
        Some(shell.control_connections.epoch)
    );
}
