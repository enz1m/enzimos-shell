//! VPNs added by hand (rather than imported from a profile file): the
//! NetworkManager settings for each type, built from a few checked fields.
//!
//! Passwords are never stored in the profile: NetworkManager asks for them
//! through the desktop's secret agent when the VPN connects. Only a group or
//! pre-shared secret, which an administrator hands out to everyone, is kept,
//! in NetworkManager's root-only connection store.

use enzui_shell::i18n::tr;

/// The kind of VPN.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// L2TP over IPsec with a pre-shared key (common on routers and NAS).
    #[default]
    L2tp,
    /// IKEv2 (strongSwan), with a user name and password or a pre-shared key.
    Ikev2,
    /// Cisco IPsec (vpnc).
    Vpnc,
    /// Cisco AnyConnect and compatible servers (OpenConnect).
    AnyConnect,
    /// Palo Alto GlobalProtect (OpenConnect).
    GlobalProtect,
    /// Fortinet FortiGate SSL VPN (OpenConnect).
    Fortinet,
    /// Ivanti/Pulse Connect Secure (OpenConnect).
    Pulse,
    /// PPTP: old and insecure, for equipment that offers nothing else.
    Pptp,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::L2tp,
        Self::Ikev2,
        Self::Vpnc,
        Self::AnyConnect,
        Self::GlobalProtect,
        Self::Fortinet,
        Self::Pulse,
        Self::Pptp,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::L2tp => tr("shell-vpn-kind-l2tp"),
            Self::Ikev2 => tr("shell-vpn-kind-ikev2"),
            Self::Vpnc => tr("shell-vpn-kind-vpnc"),
            Self::AnyConnect => tr("shell-vpn-kind-anyconnect"),
            Self::GlobalProtect => tr("shell-vpn-kind-globalprotect"),
            Self::Fortinet => tr("shell-vpn-kind-fortinet"),
            Self::Pulse => tr("shell-vpn-kind-pulse"),
            Self::Pptp => tr("shell-vpn-kind-pptp"),
        }
    }

    /// NetworkManager's plugin short name.
    #[must_use]
    pub fn plugin(self) -> &'static str {
        match self {
            Self::L2tp => "l2tp",
            Self::Ikev2 => "strongswan",
            Self::Vpnc => "vpnc",
            Self::AnyConnect | Self::GlobalProtect | Self::Fortinet | Self::Pulse => "openconnect",
            Self::Pptp => "pptp",
        }
    }

    fn openconnect_protocol(self) -> Option<&'static str> {
        match self {
            Self::AnyConnect => Some("anyconnect"),
            Self::GlobalProtect => Some("gp"),
            Self::Fortinet => Some("fortinet"),
            Self::Pulse => Some("pulse"),
            _ => None,
        }
    }

    /// Which fields the form asks for.
    #[must_use]
    pub fn fields(self) -> Fields {
        match self {
            Self::L2tp => Fields {
                user: true,
                group: false,
                secret: Some(Secret::PreShared),
                method: false,
            },
            Self::Ikev2 => Fields {
                user: true,
                group: false,
                secret: None,
                method: true,
            },
            Self::Vpnc => Fields {
                user: true,
                group: true,
                secret: Some(Secret::Group),
                method: false,
            },
            Self::Pptp => Fields {
                user: true,
                group: false,
                secret: None,
                method: false,
            },
            // OpenConnect servers ask for everything at sign-in.
            _ => Fields {
                user: false,
                group: false,
                secret: None,
                method: false,
            },
        }
    }
}

/// The shared secret a type asks for in the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secret {
    PreShared,
    Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    pub user: bool,
    /// The vpnc group name (IPsec ID).
    pub group: bool,
    pub secret: Option<Secret>,
    /// IKEv2: user name and password, or a pre-shared key.
    pub method: bool,
}

/// What the form collected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    pub kind: Kind,
    pub name: String,
    pub gateway: String,
    pub user: String,
    pub group: String,
    pub secret: String,
    /// IKEv2 with a pre-shared key instead of a user name and password.
    pub psk: bool,
}

// Values travel in NetworkManager's `key = value, …` lists: no separators,
// quotes or control characters.
fn clean(value: &str, max: usize) -> bool {
    value.len() <= max
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, ',' | '=' | '\\' | '"' | '\''))
}

fn valid_gateway(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']' | '/' | '_')
        })
}

/// Check the form; the error is a sentence to show beside it.
///
/// # Errors
/// The first problem found.
pub fn validate(spec: &Spec) -> Result<(), &'static str> {
    let fields = spec.kind.fields();
    if spec.name.trim().is_empty() || !clean(spec.name.trim(), 64) {
        return Err(tr("shell-vpn-name-invalid"));
    }
    if !valid_gateway(spec.gateway.trim()) {
        return Err(tr("shell-vpn-server-invalid"));
    }
    if fields.user
        && !(spec.kind == Kind::Ikev2 && spec.psk)
        && (spec.user.trim().is_empty() || !clean(spec.user.trim(), 128))
    {
        return Err(tr("shell-vpn-user-invalid"));
    }
    if fields.group && (spec.group.trim().is_empty() || !clean(spec.group.trim(), 128)) {
        return Err(tr("shell-vpn-group-invalid"));
    }
    let needs_secret = fields.secret.is_some() || (spec.kind == Kind::Ikev2 && spec.psk);
    if needs_secret && (spec.secret.is_empty() || !clean(&spec.secret, 256)) {
        return Err(tr("shell-vpn-secret-invalid"));
    }
    // strongSwan refuses shorter pre-shared keys as guessable.
    if spec.kind == Kind::Ikev2 && spec.psk && spec.secret.chars().count() < 20 {
        return Err(tr("shell-vpn-psk-too-short"));
    }
    Ok(())
}

/// A Cisco VPN client profile (`.pcf`) as a form: NetworkManager's own
/// importer leaves out required settings, so the profile is read here. An
/// encrypted group password is decrypted with vpnc's `cisco-decrypt`.
///
/// # Errors
/// A sentence when the profile lacks a server or group.
pub fn from_pcf(
    text: &str,
    fallback_name: &str,
    decrypt: impl Fn(&str) -> Option<String>,
) -> Result<Spec, &'static str> {
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().trim_start_matches('!').to_ascii_lowercase();
            values.entry(key).or_insert_with(|| value.trim().to_owned());
        }
    }
    let get = |key: &str| values.get(key).cloned().unwrap_or_default();
    let secret = match (get("grouppwd"), get("enc_grouppwd")) {
        (plain, _) if !plain.is_empty() => plain,
        (_, encrypted)
            if !encrypted.is_empty() && encrypted.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            decrypt(&encrypted).unwrap_or_default()
        }
        _ => String::new(),
    };
    let name = match get("description") {
        description if !description.is_empty() => description,
        _ => fallback_name.to_owned(),
    };
    let spec = Spec {
        kind: Kind::Vpnc,
        name: name
            .chars()
            .filter(|c| !matches!(c, ',' | '=' | '\\' | '"' | '\''))
            .take(64)
            .collect(),
        gateway: get("host"),
        user: get("username"),
        group: get("groupname"),
        secret,
        psk: false,
    };
    if spec.gateway.is_empty() || spec.group.is_empty() {
        return Err(tr("shell-this-pcf-profile-has-no-server"));
    }
    Ok(spec)
}

/// A VPN profile ready for NetworkManager: its plugin's settings and the
/// secrets it keeps (after [`validate`]).
pub struct Profile<'a> {
    pub name: &'a str,
    pub plugin: &'static str,
    pub data: Vec<(&'static str, String)>,
    pub secrets: Vec<(&'static str, String)>,
}

#[must_use]
pub fn profile(spec: &Spec) -> Profile<'_> {
    let gateway = spec.gateway.trim();
    let user = spec.user.trim();
    // Secret flags: 1 = asked by the desktop at connect time, 0 = stored.
    type Pairs<'a> = Vec<(&'a str, String)>;
    let (data, secrets): (Pairs<'_>, Pairs<'_>) = match spec.kind {
        Kind::L2tp => (
            vec![
                ("gateway", gateway.into()),
                ("user", user.into()),
                ("password-flags", "1".into()),
                ("ipsec-enabled", "yes".into()),
                ("ipsec-psk-flags", "0".into()),
            ],
            vec![("ipsec-psk", spec.secret.clone())],
        ),
        Kind::Ikev2 if spec.psk => (
            vec![
                ("address", gateway.into()),
                ("method", "psk".into()),
                ("virtual", "yes".into()),
                ("encap", "yes".into()),
                ("password-flags", "0".into()),
            ],
            vec![("password", spec.secret.clone())],
        ),
        Kind::Ikev2 => (
            vec![
                ("address", gateway.into()),
                ("method", "eap".into()),
                ("user", user.into()),
                ("virtual", "yes".into()),
                ("encap", "yes".into()),
                ("password-flags", "1".into()),
            ],
            Vec::new(),
        ),
        Kind::Vpnc => (
            vec![
                ("IPSec gateway", gateway.into()),
                ("IPSec ID", spec.group.trim().into()),
                ("Xauth username", user.into()),
                ("IPSec secret-flags", "0".into()),
                ("Xauth password-flags", "1".into()),
            ],
            vec![("IPSec secret", spec.secret.clone())],
        ),
        Kind::Pptp => (
            vec![
                ("gateway", gateway.into()),
                ("user", user.into()),
                ("password-flags", "1".into()),
                ("require-mppe", "yes".into()),
            ],
            Vec::new(),
        ),
        Kind::AnyConnect | Kind::GlobalProtect | Kind::Fortinet | Kind::Pulse => (
            vec![
                ("gateway", gateway.into()),
                (
                    "protocol",
                    spec.kind
                        .openconnect_protocol()
                        .unwrap_or("anyconnect")
                        .into(),
                ),
                ("cookie-flags", "2".into()),
                ("gwcert-flags", "2".into()),
                ("gateway-flags", "2".into()),
            ],
            Vec::new(),
        ),
    };
    Profile {
        name: spec.name.trim(),
        plugin: spec.kind.plugin(),
        data,
        secrets,
    }
}

/// The settings NetworkManager's `AddConnection` takes for the VPN, secrets
/// included: sent over the system bus, never on a command line where other
/// accounts could read them from the process list.
#[must_use]
pub fn connection_settings(
    profile: &Profile<'_>,
) -> std::collections::HashMap<
    &'static str,
    std::collections::HashMap<&'static str, zbus::zvariant::OwnedValue>,
> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};
    let owned = |value: Value<'_>| OwnedValue::try_from(value).expect("plain values convert");
    let pairs = |items: &[(&'static str, String)]| -> HashMap<String, String> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    };
    let mut connection = HashMap::new();
    connection.insert("id", owned(Value::from(profile.name)));
    connection.insert("type", owned(Value::from("vpn")));
    connection.insert("autoconnect", owned(Value::from(false)));
    let mut vpn = HashMap::new();
    vpn.insert(
        "service-type",
        owned(Value::from(format!(
            "org.freedesktop.NetworkManager.{}",
            profile.plugin
        ))),
    );
    vpn.insert("data", owned(Value::from(pairs(&profile.data))));
    if !profile.secrets.is_empty() {
        vpn.insert("secrets", owned(Value::from(pairs(&profile.secrets))));
    }
    let mut ipv4 = HashMap::new();
    ipv4.insert("method", owned(Value::from("auto")));
    HashMap::from([("connection", connection), ("vpn", vpn), ("ipv4", ipv4)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_become_checked_networkmanager_settings() {
        let spec = Spec {
            kind: Kind::Vpnc,
            name: "Office".into(),
            gateway: "vpn.example.com".into(),
            user: "sam".into(),
            group: "staff".into(),
            secret: "groupsecret".into(),
            psk: false,
        };
        assert_eq!(validate(&spec), Ok(()));
        let built = profile(&spec);
        assert_eq!(built.plugin, "vpnc");
        assert!(
            built
                .data
                .contains(&("IPSec gateway", "vpn.example.com".into()))
        );
        assert!(built.data.contains(&("Xauth password-flags", "1".into())));
        assert_eq!(built.secrets, [("IPSec secret", "groupsecret".into())]);
        // The secret travels inside the bus settings, in vpn.secrets.
        let settings = connection_settings(&built);
        let secrets = std::collections::HashMap::<String, String>::try_from(
            settings["vpn"]["secrets"].clone(),
        )
        .unwrap();
        assert_eq!(secrets["IPSec secret"], "groupsecret");
        assert_eq!(
            String::try_from(settings["vpn"]["service-type"].clone()).unwrap(),
            "org.freedesktop.NetworkManager.vpnc"
        );
        // Separators cannot smuggle extra settings.
        for (field, bad) in [
            ("gateway", "a.com, user = x"),
            ("user", "sam, gateway=evil"),
            ("name", "x\ny"),
        ] {
            let mut bad_spec = spec.clone();
            match field {
                "gateway" => bad_spec.gateway = bad.into(),
                "user" => bad_spec.user = bad.into(),
                _ => bad_spec.name = bad.into(),
            }
            assert!(validate(&bad_spec).is_err(), "{field} {bad:?}");
        }
        let pcf = from_pcf("[main]\nDescription=Office\nHost=vpn.example.com\nGroupName=staff\nUsername=sam\nenc_GroupPwd=ABCDEF\n", "x", |hex| (hex == "ABCDEF").then(|| "secret".into())).unwrap();
        assert_eq!(
            (
                pcf.name.as_str(),
                pcf.gateway.as_str(),
                pcf.group.as_str(),
                pcf.secret.as_str()
            ),
            ("Office", "vpn.example.com", "staff", "secret")
        );
        assert!(
            from_pcf("[main]\nHost=\n", "x", |_| None).is_err(),
            "a profile without a server is refused"
        );
        let short = Spec {
            kind: Kind::Ikev2,
            name: "Home".into(),
            gateway: "h.example".into(),
            secret: "short".into(),
            psk: true,
            ..Spec::default()
        };
        assert!(validate(&short).is_err(), "strongSwan needs 20 characters");
        let fortinet = Spec {
            kind: Kind::Fortinet,
            name: "Work".into(),
            gateway: "fw.example.com:10443".into(),
            ..Spec::default()
        };
        assert_eq!(
            validate(&fortinet),
            Ok(()),
            "OpenConnect asks for everything at sign-in"
        );
        assert!(
            profile(&fortinet)
                .data
                .contains(&("protocol", "fortinet".into()))
        );
        let ikev2 = Spec {
            kind: Kind::Ikev2,
            name: "Home".into(),
            gateway: "home.example.net".into(),
            secret: "a-long-pre-shared-key-2026".into(),
            psk: true,
            ..Spec::default()
        };
        assert_eq!(
            validate(&ikev2),
            Ok(()),
            "a pre-shared key needs no user name"
        );
        assert!(profile(&ikev2).data.contains(&("method", "psk".into())));
    }
}
