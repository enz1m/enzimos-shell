use super::*;
use enzui_shell::system_settings::{Discovered, ManualIpv4, Secret};

#[derive(Default)]
pub(super) struct Forms {
    pub join: Option<Join>,
    pub details: Option<String>,
    pub dns: Option<(String, String)>,
    pub ipv4: Option<Ipv4Edit>,
    pub vpn_path: Option<String>,
    /// The VPN (by UUID) whose options are shown.
    pub vpn_open: Option<String>,
    /// The sign-in name being edited for that VPN.
    pub vpn_username: Option<(String, String)>,
    /// A VPN being added by hand.
    pub vpn_manual: Option<enzui_shell::vpn_manual::Spec>,
    pub vpn_manual_error: Option<&'static str>,
    /// The shared key is shown in clear.
    pub vpn_manual_reveal: bool,
    /// The sign-in notice was dismissed; it returns when the state changes.
    pub portal_dismissed: bool,
    pub trust_prompt: Option<String>,
    pub printers_nearby: Option<Vec<Discovered>>,
    pub printer_search: bool,
    pub zone_query: Option<String>,
    /// The search for a world clock's time zone, while adding one.
    pub clock_query: Option<String>,
    pub hostname: Option<String>,
    // Questions from the pairing agent, oldest first.
    pub pair_prompts: Vec<enzui_shell::system_settings::pairing::Prompt>,
    pub pin: String,
}

#[derive(Default)]
pub(super) struct Join {
    pub ssid: String,
    pub password: String,
    pub revealed: bool,
    /// A network that does not broadcast its name; the SSID is typed in.
    pub hidden: bool,
    /// WPA Enterprise: a username as well as a password.
    pub enterprise: bool,
    pub identity: String,
    pub error: Option<String>,
}

pub(super) struct Ipv4Edit {
    pub connection: String,
    pub manual: bool,
    pub address: String,
    pub gateway: String,
    pub error: Option<String>,
}

/// A field of the manual VPN form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VpnField {
    Name,
    Server,
    User,
    Group,
    Secret,
}

#[derive(Clone, Debug)]
pub(crate) enum Form {
    Join(Option<(String, bool)>),
    JoinSsid(String),
    JoinPassword(String),
    JoinIdentity(String),
    JoinReveal,
    JoinSubmit,
    Details(Option<String>),
    Dns(Option<(String, String)>),
    DnsEdit(String),
    DnsSave,
    Ipv4(Option<String>),
    Ipv4Manual(bool),
    Ipv4Address(String),
    Ipv4Gateway(String),
    Ipv4Save,
    VpnPath(Option<String>),
    VpnSubmit,
    VpnOpen(Option<String>),
    VpnUsername(String, String),
    VpnUsernameSave,
    /// Open (`true`) or close the form for adding a VPN by hand.
    VpnManual(bool),
    VpnManualKind(enzui_shell::vpn_manual::Kind),
    VpnManualField(VpnField, String),
    VpnManualPsk(bool),
    VpnManualReveal,
    VpnManualSubmit,
    PortalDismiss,
    Trust(bool),
    SearchPrinters,
    PrintersFound(Vec<Discovered>),
    ZoneQuery(Option<String>),
    ClockQuery(Option<String>),
    Hostname(Option<String>),
    HostnameSave,
    Pin(String),
}

impl SettingsApp {
    pub(super) fn form_message(&mut self, form: Form) -> Task<Parent> {
        let network = match self.system.readings.get(&Panel::Network) {
            Some(Reading::Network(n)) => Some(n.clone()),
            _ => None,
        };
        let forms = &mut self.system.forms;
        match form {
            Form::Join(target) => {
                forms.join = target.map(|(ssid, hidden)| {
                    let enterprise = network.as_ref().is_some_and(|n| {
                        n.access_points
                            .iter()
                            .any(|p| p.ssid == ssid && p.enterprise)
                    });
                    Join {
                        ssid,
                        hidden,
                        enterprise,
                        ..Join::default()
                    }
                });
                let focus = match forms.join.as_ref() {
                    Some(j) if j.hidden => "network-join-ssid",
                    Some(j) if j.enterprise => "network-join-identity",
                    _ => "network-join-password",
                };
                return if forms.join.is_some() {
                    iced::widget::operation::focus(focus)
                } else {
                    Task::none()
                };
            }
            Form::JoinSsid(ssid) => {
                if let Some(join) = forms.join.as_mut() {
                    join.ssid = ssid.chars().take(32).collect();
                }
            }
            Form::JoinPassword(password) => {
                if let Some(join) = forms.join.as_mut() {
                    // WPA keys stop at 63 characters; account passwords may be longer.
                    join.password = password
                        .chars()
                        .take(if join.enterprise { 256 } else { 63 })
                        .collect();
                    join.error = None;
                }
            }
            Form::JoinIdentity(identity) => {
                if let Some(join) = forms.join.as_mut() {
                    join.identity = identity.chars().take(128).collect();
                    join.error = None;
                }
            }
            Form::JoinReveal => {
                if let Some(join) = forms.join.as_mut() {
                    join.revealed = !join.revealed;
                }
            }
            Form::JoinSubmit => {
                let Some(join) = forms.join.as_ref() else {
                    return Task::none();
                };
                let password = (!join.password.is_empty()).then(|| Secret(join.password.clone()));
                let identity = join.enterprise.then(|| join.identity.trim().to_owned());
                let request = Request::WifiJoin {
                    ssid: join.ssid.trim().to_owned(),
                    password,
                    hidden: join.hidden,
                    identity,
                };
                return self.system_message(Message::Request(request));
            }
            Form::Details(connection) => {
                forms.dns = None;
                forms.ipv4 = None;
                forms.details = if forms.details == connection {
                    None
                } else {
                    connection
                };
            }
            Form::Dns(edit) => {
                forms.dns = edit;
                return if forms.dns.is_some() {
                    iced::widget::operation::focus("network-dns")
                } else {
                    Task::none()
                };
            }
            Form::DnsEdit(text) => {
                if let Some((_, value)) = forms.dns.as_mut() {
                    *value = text.chars().take(160).collect();
                }
            }
            Form::DnsSave => {
                let Some((connection, text)) = forms.dns.clone() else {
                    return Task::none();
                };
                let servers = text
                    .split([',', ' '])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                return self.system_message(Message::Request(Request::Dns {
                    connection,
                    servers,
                }));
            }
            Form::Ipv4(connection) => {
                let profile = connection.as_ref().and_then(|c| {
                    network
                        .as_ref()?
                        .interfaces
                        .iter()
                        .find(|i| i.connection == *c)?
                        .profile
                        .clone()
                });
                forms.ipv4 = connection.map(|connection| {
                    let manual = profile.as_ref().and_then(|p| p.ipv4.clone());
                    Ipv4Edit {
                        connection,
                        manual: manual.is_some(),
                        address: manual
                            .as_ref()
                            .map(|m| m.address.clone())
                            .unwrap_or_default(),
                        gateway: manual.map(|m| m.gateway).unwrap_or_default(),
                        error: None,
                    }
                });
            }
            Form::Ipv4Manual(manual) => {
                if let Some(edit) = forms.ipv4.as_mut() {
                    edit.manual = manual;
                    edit.error = None;
                    // Starts from the address the network gave, the usual edit.
                    let details = network.as_ref().and_then(|n| {
                        n.interfaces
                            .iter()
                            .find(|i| i.connection == edit.connection)?
                            .details
                            .clone()
                    });
                    if manual
                        && edit.address.is_empty()
                        && let Some(details) = details
                    {
                        edit.address = details.ipv4.unwrap_or_default();
                        edit.gateway = details.gateway.unwrap_or_default();
                    }
                    if manual {
                        return iced::widget::operation::focus("network-ipv4-address");
                    }
                }
            }
            Form::Ipv4Address(text) => {
                if let Some(edit) = forms.ipv4.as_mut() {
                    edit.address = text.chars().take(24).collect();
                    edit.error = None;
                }
            }
            Form::Ipv4Gateway(text) => {
                if let Some(edit) = forms.ipv4.as_mut() {
                    edit.gateway = text.chars().take(16).collect();
                    edit.error = None;
                }
            }
            Form::Ipv4Save => {
                let Some(edit) = forms.ipv4.as_mut() else {
                    return Task::none();
                };
                let manual = if edit.manual {
                    match (ManualIpv4 {
                        address: edit.address.clone(),
                        gateway: edit.gateway.clone(),
                    })
                    .validated()
                    {
                        Ok(manual) => Some(manual),
                        Err(error) => {
                            edit.error = Some(error);
                            return Task::none();
                        }
                    }
                } else {
                    None
                };
                let connection = edit.connection.clone();
                return self.system_message(Message::Request(Request::Ipv4 { connection, manual }));
            }
            Form::VpnPath(path) => {
                forms.vpn_path = path;
                return if forms.vpn_path.is_some() {
                    iced::widget::operation::focus("network-vpn-submit")
                } else {
                    Task::none()
                };
            }
            Form::VpnOpen(uuid) => {
                forms.vpn_open = uuid;
                forms.vpn_username = None;
                return Task::none();
            }
            Form::VpnUsername(uuid, name) => {
                forms.vpn_username = Some((uuid, name.chars().take(128).collect()));
                return Task::none();
            }
            Form::VpnUsernameSave => {
                let Some((uuid, name)) = forms.vpn_username.take() else {
                    return Task::none();
                };
                return self.system_message(Message::Request(Request::VpnUsername { uuid, name }));
            }
            Form::VpnManual(open) => {
                forms.vpn_manual = open.then(enzui_shell::vpn_manual::Spec::default);
                forms.vpn_manual_error = None;
                forms.vpn_manual_reveal = false;
                return Task::none();
            }
            Form::VpnManualKind(kind) => {
                self.menu = None;
                if let Some(spec) = &mut self.system.forms.vpn_manual {
                    spec.kind = kind;
                    spec.psk = false;
                }
                return Task::none();
            }
            Form::VpnManualField(field, value) => {
                if let Some(spec) = &mut forms.vpn_manual {
                    let value: String = value.chars().take(256).collect();
                    match field {
                        VpnField::Name => spec.name = value,
                        VpnField::Server => spec.gateway = value,
                        VpnField::User => spec.user = value,
                        VpnField::Group => spec.group = value,
                        VpnField::Secret => spec.secret = value,
                    }
                }
                forms.vpn_manual_error = None;
                return Task::none();
            }
            Form::VpnManualReveal => {
                forms.vpn_manual_reveal = !forms.vpn_manual_reveal;
                return Task::none();
            }
            Form::VpnManualPsk(psk) => {
                if let Some(spec) = &mut forms.vpn_manual {
                    spec.psk = psk;
                }
                return Task::none();
            }
            Form::VpnManualSubmit => {
                let Some(mut spec) = forms.vpn_manual.clone() else {
                    return Task::none();
                };
                if spec.name.trim().is_empty() {
                    spec.gateway.trim().clone_into(&mut spec.name);
                }
                if let Err(problem) = enzui_shell::vpn_manual::validate(&spec) {
                    forms.vpn_manual_error = Some(problem);
                    return Task::none();
                }
                return self.system_message(Message::Request(Request::VpnAdd(Box::new(spec))));
            }
            Form::PortalDismiss => {
                forms.portal_dismissed = true;
                return Task::none();
            }
            Form::VpnSubmit => {
                let Some(path) = forms.vpn_path.clone() else {
                    return Task::none();
                };
                return self
                    .system_message(Message::Request(Request::VpnImport(path.trim().to_owned())));
            }
            Form::Trust(trusted) => {
                let Some(connection) = forms.trust_prompt.take() else {
                    return Task::none();
                };
                // A new network starts out untrusted (D-072); keeping it so
                // needs no change, and no administrator password.
                let already = network.as_ref().and_then(|n| {
                    n.interfaces
                        .iter()
                        .find(|i| i.connection == connection)?
                        .profile
                        .as_ref()
                        .map(|p| p.trusted)
                });
                if already == Some(trusted) {
                    return Task::none();
                }
                return self.system_message(Message::Request(Request::Trust {
                    connection,
                    trusted,
                }));
            }
            Form::SearchPrinters => {
                if forms.printer_search {
                    return Task::none();
                }
                forms.printer_search = true;
                forms.printers_nearby.get_or_insert_default();
                let backend = self.system.backend.clone();
                let network = self.system.backend.clone();
                // The network's trust decides whether printers can answer at
                // all, so it is read alongside, for the empty result to offer.
                return Task::batch([
                    Task::perform(async move { backend.printers_nearby().await }, |found| {
                        Parent::System(Message::Form(Form::PrintersFound(found)))
                    }),
                    Task::perform(
                        async move { network.read(Panel::Network).await },
                        |reading| Parent::System(Message::Loaded(Box::new(reading))),
                    ),
                ]);
            }
            Form::PrintersFound(found) => {
                forms.printer_search = false;
                forms.printers_nearby = Some(found);
            }
            Form::ZoneQuery(query) => {
                let opened = forms.zone_query.is_none() && query.is_some();
                forms.zone_query = query.map(|q| q.chars().take(64).collect());
                return if opened {
                    iced::widget::operation::focus("datetime-zone-search")
                } else {
                    Task::none()
                };
            }
            Form::ClockQuery(query) => {
                let opened = forms.clock_query.is_none() && query.is_some();
                forms.clock_query = query.map(|q| q.chars().take(64).collect());
                return if opened {
                    iced::widget::operation::focus("datetime-clock-search")
                } else {
                    Task::none()
                };
            }
            Form::Hostname(name) => {
                let opened = forms.hostname.is_none() && name.is_some();
                forms.hostname = name.map(|n| n.chars().take(63).collect());
                return if opened {
                    iced::widget::operation::focus("about-hostname")
                } else {
                    Task::none()
                };
            }
            Form::Pin(pin) => {
                forms.pin = pin.chars().filter(char::is_ascii_digit).take(16).collect();
            }
            Form::HostnameSave => {
                let Some(name) = forms.hostname.clone() else {
                    return Task::none();
                };
                return self
                    .system_message(Message::Request(Request::Hostname(name.trim().to_owned())));
            }
        }
        Task::none()
    }
    pub(super) fn finish_form(&mut self, request: &Request, result: &Result<(), String>) -> bool {
        // The reading still describes the network as it was before the join.
        let known = match (request, self.system.readings.get(&Panel::Network)) {
            (Request::WifiJoin { ssid, .. }, Some(Reading::Network(n))) => {
                n.access_points.iter().any(|p| p.ssid == *ssid && p.saved)
            }
            _ => false,
        };
        let trust = self.offers(enzui_shell::platform::Feature::NetworkTrust);
        let forms = &mut self.system.forms;
        match (request, result) {
            (Request::WifiJoin { ssid, .. }, Ok(())) => {
                forms.join = None;
                // D-072: trust is asked after the first successful join,
                // never assumed; a saved network keeps its earlier answer.
                // Without firewalld the answer would change nothing.
                if !known && trust {
                    forms.trust_prompt = Some(ssid.clone());
                }
                true
            }
            (Request::WifiJoin { ssid, .. }, Err(error)) => match forms.join.as_mut() {
                Some(join) if join.ssid.trim() == ssid => {
                    join.error = Some(error.clone());
                    join.password.clear();
                    true
                }
                // A saved or open network joins without a form, and a form open
                // for another network keeps its key: the page reports the failure.
                _ => false,
            },
            (Request::Dns { .. }, Ok(())) => {
                forms.dns = None;
                false
            }
            (Request::Ipv4 { .. }, Ok(())) => {
                forms.ipv4 = None;
                false
            }
            (Request::Ipv4 { .. }, Err(error)) => match forms.ipv4.as_mut() {
                Some(edit) => {
                    edit.error = Some(error.clone());
                    true
                }
                None => false,
            },
            (Request::VpnImport(_), Ok(())) => {
                forms.vpn_path = None;
                false
            }
            (Request::VpnAdd(_), Ok(())) => {
                forms.vpn_manual = None;
                false
            }
            (Request::Hostname(_), Ok(())) => {
                forms.hostname = None;
                false
            }
            (Request::Timezone(_), Ok(())) => {
                forms.zone_query = None;
                false
            }
            (Request::PrinterAdd(added), Ok(())) => {
                if let Some(nearby) = forms.printers_nearby.as_mut() {
                    nearby.retain(|d| d != added);
                    // "Nothing found" is only true of a search that found nothing.
                    if nearby.is_empty() {
                        forms.printers_nearby = None;
                    }
                }
                false
            }
            (Request::Forget(name), Ok(())) => {
                if forms.details.as_ref() == Some(name) {
                    forms.details = None;
                }
                false
            }
            _ => false,
        }
    }
    pub(super) fn busy_with(&self, matches: impl Fn(&Request) -> bool) -> bool {
        self.system.current.as_ref().is_some_and(&matches) || self.system.queue.iter().any(matches)
    }
}
