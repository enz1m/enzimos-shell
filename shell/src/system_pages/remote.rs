#[path = "remote_keys.rs"]
mod keys;
#[path = "remote_view.rs"]
mod view;

use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::remote_access::{
    self as remote, Change, Credentials, Endpoint, Overview, Review, SharingRequest, SshRequest,
};
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_widgets::text_field::enz_text_field;
use iced::widget::{container, row};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::settings_app) enum Kind {
    Ssh,
    Sharing,
}

pub(super) struct Draft {
    kind: Kind,
    address: Option<String>,
    addresses: Vec<String>,
    networks: String,
    port: String,
    keys: Vec<String>,
    key_draft: String,
    autostart: bool,
    allow_password: bool,
    control: bool,
}

#[derive(Default)]
pub(super) struct Remote {
    // `Some(None)`: this system has no remote access service.
    overview: Option<Option<Overview>>,
    credentials: Option<Credentials>,
    loading: bool,
    read_again: bool,
    read_epoch: u64,
    read_at: Option<Instant>,
    epoch: u64,
    draft: Option<Draft>,
    reviewing: Option<(u64, Kind)>,
    review: Option<(u64, Kind, Result<Review, String>)>,
    applying: Option<(u64, Kind)>,
    outcome: Option<(Kind, Result<(), String>)>,
    show_password: bool,
    key_work: Option<u64>,
    key_epoch: u64,
    generated_folder: Option<std::path::PathBuf>,
}

impl Remote {
    fn retire_precommit(&mut self) {
        self.key_work = None;
        self.key_epoch = self.key_epoch.saturating_add(1);
        self.generated_folder = None;
        self.draft = None;
        self.reviewing = None;
        self.review = None;
        self.show_password = false;
        self.epoch = self.epoch.saturating_add(1);
    }
    fn busy(&self) -> bool {
        self.reviewing.is_some() || self.applying.is_some() || self.key_work.is_some()
    }
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum RemoteMessage {
    Read(u64, Option<Box<Overview>>, Option<Credentials>),
    Invalidated(bool),
    SessionActivity(enzui_shell::session_activity::State),
    Edit(Kind),
    Address(String),
    ListenAddress(String, bool),
    Networks(String),
    Port(String),
    KeyDraft(String),
    AddKey,
    LoadKeyFile(std::path::PathBuf),
    GenerateKey,
    KeyReady(u64, Result<keys::SelectedKey, ()>),
    RemoveKey(usize),
    Autostart(bool),
    PasswordAuth(bool),
    Control(bool),
    CancelEdit,
    ReviewDraft,
    ReviewOff(Kind),
    ReviewDisconnect,
    ReviewUnattended(bool),
    Reviewed(u64, Result<Review, String>),
    Apply(u64),
    Applied(u64, Result<(), String>),
    Discard(u64),
    ShowPassword(bool),
    Copy(String),
}

fn kind_of(change: &Change) -> Kind {
    match change {
        Change::Ssh(_) | Change::DisconnectSsh => Kind::Ssh,
        Change::Share(_) | Change::StopSharing | Change::Unattended(_) => Kind::Sharing,
    }
}

// ENZUI_DEMO_REMOTE_SHARING=sharing shows sharing running; =unattended adds
// unattended access; =unattended-waiting shows it waiting for sign-in or its network.
fn demo_sharing() -> remote::Sharing {
    let state = std::env::var("ENZUI_DEMO_REMOTE_SHARING").unwrap_or_default();
    let endpoint = Endpoint {
        address: "192.168.1.37".into(),
        port: 5900,
        networks: vec!["192.168.1.0/24".into()],
    };
    let running = matches!(state.as_str(), "sharing" | "unattended");
    remote::Sharing {
        setup: running.then(|| remote::SharingSetup {
            endpoint: endpoint.clone(),
            view_only: true,
        }),
        unattended: state.starts_with("unattended").then(|| remote::Unattended {
            endpoint,
            interface: "enp1s0".into(),
        }),
        listening: running,
        state: if running { "active" } else { "inactive" }.into(),
        cleanup_pending: false,
        viewers: Some(0),
    }
}

fn demo_overview() -> Overview {
    Overview {
        capabilities: remote::Capabilities {
            ssh: true,
            sftp: true,
            desktop_sharing: true,
            desktop_unattended: true,
        },
        ssh: Ok(remote::Ssh {
            setup: None,
            listening: false,
            sessions: Vec::new(),
            host_key: None,
            other_account: false,
            failure: None,
        }),
        sharing: Ok(demo_sharing()),
        addresses: vec![remote::Address {
            address: "192.168.1.37".into(),
            interface: "enp1s0".into(),
            network: "192.168.1.0/24".into(),
        }],
    }
}

fn message(message: RemoteMessage) -> Parent {
    Parent::System(Message::Remote(message))
}

impl SettingsApp {
    pub(in crate::settings_app) fn remote_key_file(
        &mut self,
        path: std::path::PathBuf,
    ) -> Task<Parent> {
        self.remote_message(RemoteMessage::LoadKeyFile(path))
    }

    pub(in crate::settings_app) fn remote_subscription(&self) -> iced::Subscription<Parent> {
        if self.page == Category::Remote && !self.system.backend.demo {
            iced::Subscription::batch([
                remote::subscription()
                    .map(|unavailable| message(RemoteMessage::Invalidated(unavailable))),
                enzui_shell::session_activity::subscription()
                    .map(|state| message(RemoteMessage::SessionActivity(state))),
            ])
        } else {
            iced::Subscription::none()
        }
    }

    pub(super) fn remote_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.remote;
        if self.system.backend.demo {
            if state.overview.is_none() {
                state.overview = Some(Some(demo_overview()));
            }
            return Task::none();
        }
        let stale = state
            .read_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(5));
        if state.loading || state.applying.is_some() || !(force || stale) {
            return Task::none();
        }
        let Some(epoch) = state.read_epoch.checked_add(1) else {
            return Task::none();
        };
        state.read_epoch = epoch;
        state.loading = true;
        Task::perform(
            async move {
                let overview = remote::read().await;
                let credentials = tokio::task::spawn_blocking(remote::credentials)
                    .await
                    .ok()
                    .flatten();
                (overview, credentials)
            },
            move |(overview, credentials)| {
                message(RemoteMessage::Read(
                    epoch,
                    overview.map(Box::new),
                    credentials,
                ))
            },
        )
    }

    pub(in crate::settings_app) fn remote_leave(&mut self) {
        self.system.remote.retire_precommit();
    }

    fn remote_admitted(&self) -> bool {
        !self.closing
            && self.page == Category::Remote
            && self.filter.trim().is_empty()
            && self.system.backend.writes_allowed
    }

    fn remote_draft(&self, kind: Kind) -> Draft {
        let overview = self
            .system
            .remote
            .overview
            .as_ref()
            .and_then(Option::as_ref);
        let first = overview.and_then(|o| o.addresses.first());
        let selected_addresses = overview
            .and_then(|o| o.ssh.as_ref().ok()?.setup.as_ref())
            .map(|ssh| ssh.addresses.clone());
        let allow_password = overview
            .and_then(|o| o.ssh.as_ref().ok()?.setup.as_ref())
            .is_some_and(|ssh| ssh.allow_password);
        let current = match kind {
            Kind::Ssh => overview
                .and_then(|o| o.ssh.as_ref().ok()?.setup.clone())
                .map(|s| {
                    (
                        s.endpoint,
                        s.keys.into_iter().map(|k| k.key).collect(),
                        s.autostart,
                        false,
                    )
                }),
            Kind::Sharing => overview
                .and_then(|o| o.sharing.as_ref().ok()?.setup.clone())
                .map(|s| (s.endpoint, Vec::new(), false, !s.view_only)),
        };
        let fresh_keys = || {
            if kind == Kind::Ssh {
                remote::own_keys()
            } else {
                Vec::new()
            }
        };
        match current {
            Some((endpoint, keys, autostart, control)) => Draft {
                kind,
                addresses: selected_addresses.unwrap_or_else(|| vec![endpoint.address.clone()]),
                address: Some(endpoint.address),
                networks: endpoint.networks.join(", "),
                port: endpoint.port.to_string(),
                keys: if keys.is_empty() { fresh_keys() } else { keys },
                key_draft: String::new(),
                autostart,
                allow_password,
                control,
            },
            None => Draft {
                kind,
                address: first.map(|a| a.address.clone()),
                addresses: first.map(|a| vec![a.address.clone()]).unwrap_or_default(),
                networks: first.map(|a| a.network.clone()).unwrap_or_default(),
                port: if kind == Kind::Ssh {
                    "22".into()
                } else {
                    "5900".into()
                },
                keys: fresh_keys(),
                key_draft: String::new(),
                autostart: false,
                allow_password: false,
                control: false,
            },
        }
    }

    fn remote_start_review(&mut self, change: Change) -> Task<Parent> {
        let state = &mut self.system.remote;
        let Some(epoch) = state.epoch.checked_add(1) else {
            return Task::none();
        };
        let kind = kind_of(&change);
        state.epoch = epoch;
        state.review = None;
        state.outcome = None;
        state.reviewing = Some((epoch, kind));
        if self.system.backend.demo {
            let consequences = remote::consequences(&change, &serde_json::Value::Null);
            return Task::done(message(RemoteMessage::Reviewed(
                epoch,
                Ok(Review {
                    change,
                    plan_sha256: "0".repeat(64),
                    consequences,
                }),
            )));
        }
        Task::perform(remote::review(change), move |result| {
            message(RemoteMessage::Reviewed(epoch, result))
        })
    }

    pub(super) fn remote_message(&mut self, msg: RemoteMessage) -> Task<Parent> {
        let admitted = self.remote_admitted();
        let state = &mut self.system.remote;
        match msg {
            RemoteMessage::SessionActivity(activity) => {
                if activity != enzui_shell::session_activity::State::Active {
                    return Task::none();
                }
                if state.loading {
                    state.read_again = true;
                    Task::none()
                } else {
                    self.remote_read(true)
                }
            }
            RemoteMessage::Invalidated(unavailable) => {
                if unavailable {
                    state.read_epoch = state.read_epoch.saturating_add(1);
                    state.loading = false;
                    state.read_again = false;
                    state.retire_precommit();
                    state.overview = None;
                    state.credentials = None;
                } else if state.loading {
                    state.read_again = true;
                    return Task::none();
                }
                self.remote_read(true)
            }
            RemoteMessage::Read(epoch, overview, credentials) => {
                if self.closing || epoch != state.read_epoch {
                    return Task::none();
                }
                state.loading = false;
                state.read_at = Some(Instant::now());
                state.overview = Some(overview.map(|value| *value));
                state.credentials = credentials;
                if std::mem::take(&mut state.read_again) {
                    self.remote_read(true)
                } else {
                    Task::none()
                }
            }
            RemoteMessage::Edit(kind) => {
                if !admitted || state.busy() {
                    return Task::none();
                }
                self.file_chooser_leave();
                let draft = self.remote_draft(kind);
                let state = &mut self.system.remote;
                state.retire_precommit();
                state.outcome = None;
                state.draft = Some(draft);
                Task::none()
            }
            RemoteMessage::CancelEdit => {
                if state.applying.is_none() {
                    state.retire_precommit();
                    self.file_chooser_leave();
                }
                Task::none()
            }
            RemoteMessage::ListenAddress(address, enabled) => {
                let known = state
                    .overview
                    .as_ref()
                    .and_then(Option::as_ref)
                    .and_then(|overview| {
                        overview
                            .addresses
                            .iter()
                            .find(|entry| entry.address == address)
                    });
                if let Some(draft) = state
                    .draft
                    .as_mut()
                    .filter(|draft| draft.kind == Kind::Ssh && state.reviewing.is_none())
                {
                    if enabled {
                        let Some(known) = known.filter(|_| {
                            !draft.addresses.contains(&address) && draft.addresses.len() < 8
                        }) else {
                            return Task::none();
                        };
                        draft.addresses.push(address);
                        draft.addresses.sort();
                        let mut networks: Vec<_> = draft
                            .networks
                            .split(',')
                            .map(str::trim)
                            .filter(|network| !network.is_empty())
                            .map(str::to_owned)
                            .collect();
                        if !networks.contains(&known.network) {
                            networks.push(known.network.clone());
                        }
                        draft.networks = networks.join(", ");
                    } else {
                        draft.addresses.retain(|entry| *entry != address);
                        draft.networks = draft
                            .networks
                            .split(',')
                            .map(str::trim)
                            .filter(|network| {
                                network
                                    .split_once('/')
                                    .and_then(|(ip, _)| ip.parse::<std::net::IpAddr>().ok())
                                    .is_none_or(|ip| {
                                        draft
                                            .addresses
                                            .iter()
                                            .filter_map(|address| {
                                                address.parse::<std::net::IpAddr>().ok()
                                            })
                                            .any(|address| address.is_ipv4() == ip.is_ipv4())
                                    })
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                    }
                    draft.address = draft.addresses.first().cloned();
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::Address(address) => {
                let network = state
                    .overview
                    .as_ref()
                    .and_then(Option::as_ref)
                    .and_then(|o| o.addresses.iter().find(|a| a.address == address))
                    .map(|a| a.network.clone());
                if let Some(draft) = state.draft.as_mut().filter(|_| state.reviewing.is_none()) {
                    if let Some(network) = network {
                        draft.networks = network;
                    }
                    draft.address = Some(address);
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::Networks(text) => {
                if let Some(draft) = state.draft.as_mut() {
                    draft.networks = text.chars().take(400).collect();
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::Port(text) => {
                if let Some(draft) = state.draft.as_mut() {
                    draft.port = text.chars().filter(char::is_ascii_digit).take(5).collect();
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::KeyDraft(text) => {
                if let Some(draft) = state.draft.as_mut() {
                    draft.key_draft = text.chars().take(16384).collect();
                }
                Task::none()
            }
            RemoteMessage::LoadKeyFile(_) | RemoteMessage::GenerateKey => {
                if !admitted
                    || state.busy()
                    || !state
                        .draft
                        .as_ref()
                        .is_some_and(|draft| draft.kind == Kind::Ssh && draft.keys.len() < 32)
                {
                    return Task::none();
                }
                state.key_epoch = state.key_epoch.saturating_add(1);
                let epoch = state.key_epoch;
                state.key_work = Some(epoch);
                state.review = None;
                Task::perform(
                    async move {
                        match msg {
                            RemoteMessage::LoadKeyFile(path) => keys::select(path).await,
                            _ => keys::generate().await,
                        }
                    },
                    move |result| message(RemoteMessage::KeyReady(epoch, result)),
                )
            }
            RemoteMessage::KeyReady(epoch, result) => {
                if !admitted || state.key_work != Some(epoch) {
                    return Task::none();
                }
                state.key_work = None;
                match result {
                    Ok(key) => {
                        if let Some(draft) = state
                            .draft
                            .as_mut()
                            .filter(|draft| draft.kind == Kind::Ssh && draft.keys.len() < 32)
                            && !draft.keys.contains(&key.public_key)
                        {
                            draft.keys.push(key.public_key.clone());
                            if let Some(folder) = key.accept() {
                                state.generated_folder = Some(folder);
                            }
                            state.review = None;
                        }
                    }
                    Err(()) => self.error = Some(tr("shell-ssh-key-operation-failed").into()),
                }
                Task::none()
            }
            RemoteMessage::AddKey => {
                if let Some(draft) = state.draft.as_mut() {
                    let key = draft.key_draft.trim().to_owned();
                    if remote::key_fingerprint(&key).is_some()
                        && !draft.keys.contains(&key)
                        && draft.keys.len() < 32
                    {
                        draft.keys.push(key);
                        draft.key_draft.clear();
                        state.review = None;
                    }
                }
                Task::none()
            }
            RemoteMessage::RemoveKey(index) => {
                if let Some(draft) = state.draft.as_mut().filter(|d| index < d.keys.len()) {
                    draft.keys.remove(index);
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::PasswordAuth(on) => {
                if let Some(draft) = state.draft.as_mut().filter(|draft| draft.kind == Kind::Ssh) {
                    draft.allow_password = on;
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::Autostart(on) => {
                if let Some(draft) = state.draft.as_mut() {
                    draft.autostart = on;
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::Control(on) => {
                if let Some(draft) = state.draft.as_mut() {
                    draft.control = on;
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::ReviewDraft => {
                if !admitted || state.busy() {
                    return Task::none();
                }
                let Some(draft) = state.draft.as_ref() else {
                    return Task::none();
                };
                let (Some(address), Ok(port)) = (draft.address.clone(), draft.port.parse::<u16>())
                else {
                    state.review = Some((
                        state.epoch,
                        draft.kind,
                        Err(tr("shell-choose-an-address-and-a-port-first").into()),
                    ));
                    return Task::none();
                };
                let endpoint = Endpoint {
                    address,
                    port,
                    networks: remote::networks(&draft.networks),
                };
                let change = match draft.kind {
                    Kind::Ssh if draft.keys.is_empty() && !draft.allow_password => {
                        state.review = Some((
                            state.epoch,
                            Kind::Ssh,
                            Err(tr("shell-ssh-choose-authentication").into()),
                        ));
                        return Task::none();
                    }
                    Kind::Ssh => Change::Ssh(SshRequest {
                        enabled: true,
                        autostart: draft.autostart,
                        allow_password: draft.allow_password,
                        addresses: draft.addresses.clone(),
                        endpoint,
                        keys: draft.keys.clone(),
                    }),
                    Kind::Sharing => Change::Share(SharingRequest {
                        endpoint,
                        view_only: !draft.control,
                    }),
                };
                self.remote_start_review(change)
            }
            RemoteMessage::ReviewOff(kind) => {
                if !admitted || state.busy() {
                    return Task::none();
                }
                let change = match kind {
                    Kind::Sharing => Change::StopSharing,
                    Kind::Ssh => {
                        let Some(setup) = state
                            .overview
                            .as_ref()
                            .and_then(Option::as_ref)
                            .and_then(|o| o.ssh.as_ref().ok()?.setup.clone())
                        else {
                            return Task::none();
                        };
                        Change::Ssh(SshRequest {
                            enabled: false,
                            autostart: false,
                            allow_password: setup.allow_password,
                            addresses: setup.addresses,
                            endpoint: setup.endpoint,
                            keys: setup.keys.into_iter().map(|k| k.key).collect(),
                        })
                    }
                };
                state.draft = None;
                self.remote_start_review(change)
            }
            RemoteMessage::ReviewUnattended(on) => {
                if !admitted || state.busy() {
                    return Task::none();
                }
                state.draft = None;
                self.remote_start_review(Change::Unattended(on))
            }
            RemoteMessage::ReviewDisconnect => {
                if !admitted || state.busy() {
                    return Task::none();
                }
                state.draft = None;
                self.remote_start_review(Change::DisconnectSsh)
            }
            RemoteMessage::Reviewed(epoch, result) => {
                let Some((current, kind)) = state
                    .reviewing
                    .filter(|(current, _)| *current == epoch && !self.closing)
                else {
                    return Task::none();
                };
                state.reviewing = None;
                state.review = Some((current, kind, result));
                Task::none()
            }
            RemoteMessage::Discard(epoch) => {
                if state
                    .review
                    .as_ref()
                    .is_some_and(|(current, _, _)| *current == epoch)
                    && state.applying.is_none()
                {
                    state.review = None;
                }
                Task::none()
            }
            RemoteMessage::Apply(epoch) => {
                if !admitted || state.busy() || epoch != state.epoch {
                    return Task::none();
                }
                let Some((_, kind, Ok(review))) = state.review.take() else {
                    return Task::none();
                };
                state.applying = Some((epoch, kind));
                if self.system.backend.demo {
                    if let (Change::Unattended(on), Some(Some(overview))) =
                        (&review.change, state.overview.as_mut())
                        && let Ok(sharing) = overview.sharing.as_mut()
                    {
                        sharing.unattended = if *on {
                            sharing.setup.as_ref().map(|setup| remote::Unattended {
                                endpoint: setup.endpoint.clone(),
                                interface: "enp1s0".into(),
                            })
                        } else {
                            None
                        };
                    }
                    return Task::perform(
                        tokio::time::sleep(Duration::from_millis(900)),
                        move |()| message(RemoteMessage::Applied(epoch, Ok(()))),
                    );
                }
                Task::perform(remote::apply(review), move |result| {
                    message(RemoteMessage::Applied(epoch, result))
                })
            }
            RemoteMessage::Applied(epoch, result) => {
                let Some((_, kind)) = state.applying.filter(|(current, _)| *current == epoch)
                else {
                    return Task::none();
                };
                state.applying = None;
                if result.is_ok() {
                    state.draft = None;
                }
                state.outcome = Some((kind, result));
                self.remote_read(true)
            }
            RemoteMessage::ShowPassword(shown) => {
                state.show_password =
                    shown && state.credentials.is_some() && self.page == Category::Remote;
                Task::none()
            }
            RemoteMessage::Copy(text) => {
                if self.closing || self.page != Category::Remote {
                    return Task::none();
                }
                iced::clipboard::write(text).discard()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, SettingsApp) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = SettingsApp::new(
            enzui_shell::settings::Store::at(directory.path().join("settings.json")).unwrap(),
        );
        app.page = Category::Remote;
        app.live_system_backend();
        app.system.remote.overview = Some(Some(demo_overview()));
        (directory, app)
    }
    fn review(change: Change) -> Review {
        Review {
            change,
            plan_sha256: "a".repeat(64),
            consequences: vec![],
        }
    }

    #[test]
    fn unattended_toggle_reviews_a_sharing_change_and_waits_while_busy() {
        let (_directory, mut app) = fixture();
        let _ = app.remote_message(RemoteMessage::Edit(Kind::Sharing));
        let _ = app.remote_message(RemoteMessage::ReviewUnattended(true));
        let (epoch, kind) = app.system.remote.reviewing.expect("unattended review");
        assert_eq!(kind, Kind::Sharing);
        assert!(
            app.system.remote.draft.is_none(),
            "the toggle replaces an open sharing form"
        );
        let _ = app.remote_message(RemoteMessage::ReviewUnattended(false));
        assert_eq!(app.system.remote.reviewing, Some((epoch, Kind::Sharing)));
        let _ = app.remote_message(RemoteMessage::Reviewed(
            epoch,
            Ok(review(Change::Unattended(true))),
        ));
        let _ = app.remote_message(RemoteMessage::Apply(epoch));
        assert_eq!(app.system.remote.applying, Some((epoch, Kind::Sharing)));
    }

    #[test]
    fn remote_behaviour() {
        // Replace a disconnected SSH address.
        {
            let (_directory, mut app) = fixture();
            let _ = app.remote_message(RemoteMessage::Edit(Kind::Ssh));
            let draft = app.system.remote.draft.as_mut().unwrap();
            draft.address = Some("192.168.10.4".into());
            draft.addresses = vec!["192.168.10.4".into()];
            draft.networks = "192.168.10.0/24".into();
            let _ = app.remote_message(RemoteMessage::ListenAddress("192.168.10.4".into(), false));
            let _ = app.remote_message(RemoteMessage::ListenAddress("192.168.1.37".into(), true));
            let draft = app.system.remote.draft.as_ref().unwrap();
            assert_eq!(draft.addresses, ["192.168.1.37"]);
            assert_eq!(draft.networks, "192.168.1.0/24");
        }

        // Ssh needs a key and a stale review never applies.
        {
            let (_directory, mut app) = fixture();
            let _ = app.remote_message(RemoteMessage::Edit(Kind::Ssh));
            app.system.remote.draft.as_mut().unwrap().keys.clear();
            let _ = app.remote_message(RemoteMessage::ReviewDraft);
            assert!(app.system.remote.reviewing.is_none());
            assert!(
                matches!(&app.system.remote.review, Some((_, Kind::Ssh, Err(error))) if error.contains("public key"))
            );
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let generated = runtime
                .block_on(keys::generate_at(_directory.path()))
                .unwrap();
            let folder = generated.accept().unwrap();
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(folder.join("id_ed25519"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert!(
                runtime
                    .block_on(keys::select(folder.join("id_ed25519")))
                    .is_err(),
                "private material never becomes a public key result"
            );
            std::os::unix::fs::symlink(folder.join("id_ed25519.pub"), folder.join("linked.pub"))
                .unwrap();
            assert!(
                runtime
                    .block_on(keys::select(folder.join("linked.pub")))
                    .is_err()
            );
            let public = runtime
                .block_on(keys::select(folder.join("id_ed25519.pub")))
                .unwrap();
            app.system.remote.key_work = Some(7);
            let _ = app.remote_message(RemoteMessage::KeyReady(7, Ok(public)));
            assert_eq!(
                app.system.remote.draft.as_ref().unwrap().keys,
                [generated.public_key]
            );
            app.system.remote.draft.as_mut().unwrap().keys.clear();
            let abandoned = runtime
                .block_on(keys::generate_at(_directory.path()))
                .unwrap();
            app.system.remote.key_work = Some(8);
            let _ = app.remote_message(RemoteMessage::CancelEdit);
            let _ = app.remote_message(RemoteMessage::KeyReady(8, Ok(abandoned)));
            assert_eq!(
                std::fs::read_dir(_directory.path()).unwrap().count(),
                1,
                "retired key generation removes its own unaccepted private pair"
            );
            let _ = app.remote_message(RemoteMessage::Edit(Kind::Ssh));
            app.system.remote.draft.as_mut().unwrap().keys.clear();
            let _ = app.remote_message(RemoteMessage::PasswordAuth(true));
            let _ = app.remote_message(RemoteMessage::ReviewDraft);
            assert!(
                app.system.remote.reviewing.is_some(),
                "password-only setup is explicitly reviewable"
            );
            let _ = app.remote_message(RemoteMessage::CancelEdit);
            let _ = app.remote_message(RemoteMessage::Edit(Kind::Ssh));
            app.system.remote.draft.as_mut().unwrap().keys.clear();
            let _ = app.remote_message(RemoteMessage::KeyDraft(
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA== me@laptop".into(),
            ));
            let _ = app.remote_message(RemoteMessage::AddKey);
            assert_eq!(app.system.remote.draft.as_ref().unwrap().keys.len(), 1);
            let _ = app.remote_message(RemoteMessage::ReviewDraft);
            let (old, _) = app.system.remote.reviewing.unwrap();
            let _ = app.remote_message(RemoteMessage::CancelEdit);
            let _ = app.remote_message(RemoteMessage::Reviewed(
                old,
                Ok(review(Change::DisconnectSsh)),
            ));
            assert!(
                app.system.remote.review.is_none(),
                "a cancelled review does not come back"
            );
            let _ = app.remote_message(RemoteMessage::Apply(old));
            assert!(app.system.remote.applying.is_none());
        }
        // A review applies once and leaving the page retires it.
        {
            let (_directory, mut app) = fixture();
            let _ = app.remote_message(RemoteMessage::Edit(Kind::Sharing));
            let _ = app.remote_message(RemoteMessage::ReviewDraft);
            let (epoch, kind) = app.system.remote.reviewing.unwrap();
            assert_eq!(kind, Kind::Sharing);
            let endpoint = Endpoint {
                address: "192.168.1.37".into(),
                port: 5900,
                networks: vec!["192.168.1.0/24".into()],
            };
            let _ = app.remote_message(RemoteMessage::Reviewed(
                epoch,
                Ok(review(Change::Share(SharingRequest {
                    endpoint,
                    view_only: true,
                }))),
            ));
            let _ = app.remote_message(RemoteMessage::Apply(epoch));
            assert!(app.system.remote.applying.is_some() && app.system.remote.review.is_none());
            let _ = app.remote_message(RemoteMessage::Apply(epoch));
            let _ = app.remote_message(RemoteMessage::Applied(epoch + 1, Ok(())));
            assert!(
                app.system.remote.applying.is_some(),
                "only its own reply settles it"
            );
            let _ = app.remote_message(RemoteMessage::Applied(
                epoch,
                Err("An administrator did not approve this change".into()),
            ));
            assert!(matches!(
                app.system.remote.outcome,
                Some((Kind::Sharing, Err(_)))
            ));
            assert!(
                app.system.remote.draft.is_some(),
                "a refused change keeps what was typed"
            );

            let _ = app.remote_message(RemoteMessage::ReviewDraft);
            let (lost_epoch, _) = app.system.remote.reviewing.expect("pending review");
            let _ = app.remote_message(RemoteMessage::Invalidated(false));
            assert_eq!(
                app.system.remote.reviewing,
                Some((lost_epoch, Kind::Sharing))
            );
            let _ = app.remote_message(RemoteMessage::Invalidated(true));
            let _ = app.remote_message(RemoteMessage::Reviewed(
                lost_epoch,
                Ok(review(Change::StopSharing)),
            ));
            let _ = app.remote_message(RemoteMessage::Apply(lost_epoch));
            assert!(app.system.remote.reviewing.is_none() && app.system.remote.review.is_none());
            assert!(app.system.remote.draft.is_none() && app.system.remote.applying.is_none());

            let _ = app.remote_message(RemoteMessage::Edit(Kind::Sharing));
            let _ = app.remote_message(RemoteMessage::ReviewDraft);
            let _ = app.update(Parent::Page(Category::Apps));
            assert!(app.system.remote.reviewing.is_none() && app.system.remote.draft.is_none());

            app.page = Category::Remote;
            app.system.backend.demo = false;
            app.system.remote.read_epoch = 20;
            app.system.remote.loading = true;
            for _ in 0..4 {
                let _ = app.remote_message(RemoteMessage::Invalidated(false));
            }
            assert_eq!(
                app.system.remote.read_epoch, 20,
                "invalidations cannot overlap an owned read"
            );
            let _ = app.remote_message(RemoteMessage::Read(
                20,
                Some(Box::new(demo_overview())),
                None,
            ));
            assert_eq!(
                app.system.remote.read_epoch, 21,
                "coalesced invalidations schedule one fresh read"
            );
            assert!(app.system.remote.loading);
            let _ = app.remote_message(RemoteMessage::Read(20, None, None));
            assert!(
                app.system.remote.loading,
                "a late reply cannot retire the newer read"
            );
            let _ = app.remote_message(RemoteMessage::Read(
                21,
                Some(Box::new(demo_overview())),
                None,
            ));
            assert!(!app.system.remote.loading && !app.system.remote.read_again);
        }
    }
}
