//! Validates VPN imports, rejecting external commands, file reads and plaintext credential
//! files.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
/// Larger files are not VPN profiles.
pub const MAX_PROFILE: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    WireGuard,
    OpenVpn,
    /// Cisco IPsec client profiles (`.pcf`).
    Vpnc,
}

impl Kind {
    #[must_use]
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("conf") => Some(Self::WireGuard),
            Some("ovpn") => Some(Self::OpenVpn),
            Some("pcf") => Some(Self::Vpnc),
            _ => None,
        }
    }
    /// The `nmcli connection import type` name.
    pub fn nm(self) -> &'static str {
        match self {
            Self::WireGuard => "wireguard",
            Self::OpenVpn => "openvpn",
            Self::Vpnc => "vpnc",
        }
    }
}

// Reject OpenVPN directives that execute code or read external files.
const OPENVPN_REFUSED: &[&str] = &[
    "up",
    "down",
    "ipchange",
    "route-up",
    "route-pre-down",
    "tls-verify",
    "auth-user-pass-verify",
    "client-connect",
    "client-disconnect",
    "learn-address",
    "plugin",
    "script-security",
    "up-restart",
    "config",
    "management",
    "management-external-key",
    "management-external-cert",
    "tls-crypt-v2-verify",
    "iproute",
    "daemon",
    "log",
    "log-append",
    "writepid",
    "cd",
    "chroot",
];

/// Why `text` must not be imported as a `kind` profile, if it must not.
pub fn problem(kind: Kind, text: &str) -> Option<String> {
    if text.contains('\0') {
        return Some(tr("shell-this-is-not-a-text-vpn-profile").into());
    }
    match kind {
        Kind::WireGuard => {
            let mut interface = false;
            let mut peer = false;
            let mut key = false;
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                if line.eq_ignore_ascii_case("[Interface]") {
                    interface = true;
                } else if line.eq_ignore_ascii_case("[Peer]") {
                    peer = true;
                } else if let Some((name, _)) = line.split_once('=') {
                    let name = name.trim().to_ascii_lowercase();
                    if matches!(name.as_str(), "preup" | "postup" | "predown" | "postdown") {
                        return Some(
                            tr!("shell-this-profile-runs-commands-which-is-not-imported", "arg0" => name_case(&name).to_owned()),
                        );
                    }
                    if name == "privatekey" {
                        key = true;
                    }
                }
            }
            if !interface || !peer || !key {
                return Some(tr("shell-this-is-not-a-complete-wireguard-profile-it-needs-interface-with-a-privateke").into());
            }
            None
        }
        // A Cisco client profile: an INI with the gateway in `Host`. It runs
        // nothing; the vpnc plugin reads only the connection keys.
        Kind::Vpnc => {
            let host = text.lines().any(|line| {
                line.split_once('=').is_some_and(|(key, value)| {
                    key.trim().eq_ignore_ascii_case("host") && !value.trim().is_empty()
                })
            });
            (!host).then(|| tr("shell-this-pcf-profile-has-no-server").to_owned())
        }
        Kind::OpenVpn => {
            let mut remote = false;
            let mut inline = None::<String>;
            for line in text.lines() {
                let line = line.trim();
                if let Some(tag) = inline.as_deref() {
                    if line.eq_ignore_ascii_case(&format!("</{tag}>")) {
                        inline = None;
                    }
                    continue;
                }
                if let Some(tag) = line
                    .strip_prefix('<')
                    .and_then(|rest| rest.strip_suffix('>'))
                {
                    inline = Some(tag.to_owned());
                    continue;
                }
                if line.starts_with('#') || line.starts_with(';') || line.is_empty() {
                    continue;
                }
                let mut words = line.split_whitespace();
                let directive = words
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("--")
                    .to_ascii_lowercase();
                if OPENVPN_REFUSED.contains(&directive.as_str()) {
                    return Some(
                        tr!("shell-this-profile-uses-which-can-run-commands-or-read-other-files-so-it-is", "arg0" => directive),
                    );
                }
                // A password saved in a file next to the profile stays out.
                if directive == "auth-user-pass" && words.next().is_some() {
                    return Some(tr("shell-this-profile-reads-its-password-from-a-file-remove-the-file-name-from-auth-u").into());
                }
                if directive == "remote" {
                    remote = true;
                }
            }
            if !remote {
                return Some(
                    tr("shell-this-is-not-an-openvpn-client-profile-it-names-no-server-remote")
                        .into(),
                );
            }
            None
        }
    }
}

fn name_case(name: &str) -> &'static str {
    match name {
        "preup" => "PreUp",
        "postup" => "PostUp",
        "predown" => "PreDown",
        _ => "PostDown",
    }
}

/// The UUID nmcli prints after an import.
#[must_use]
pub fn imported_uuid(output: &str) -> Option<String> {
    let start = output.find('(')? + 1;
    let end = output.get(start..)?.find(')')? + start;
    let uuid = output.get(start..end)?;
    (uuid.len() == 36 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        .then(|| uuid.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_that_run_or_read_anything_else_are_refused() {
        let good = "[Interface]\nPrivateKey = abc=\nAddress = 10.0.0.2/32\nDNS = 10.0.0.1\n\n[Peer]\nPublicKey = def=\nEndpoint = vpn.example.org:51820\nAllowedIPs = 0.0.0.0/0\n";
        assert_eq!(problem(Kind::WireGuard, good), None);
        assert!(
            problem(
                Kind::WireGuard,
                &good.replace("DNS", "PostUp = curl x | sh\nDNS")
            )
            .unwrap()
            .contains("PostUp")
        );
        assert!(problem(Kind::WireGuard, "[Interface]\nAddress = 1\n").is_some());
        let ovpn = "client\ndev tun\nremote vpn.example.org 1194\nauth-user-pass\n<ca>\nup /not/a/directive\n</ca>\n";
        assert_eq!(
            problem(Kind::OpenVpn, ovpn),
            None,
            "inline blocks are data, not directives"
        );
        for bad in [
            "up /tmp/x.sh",
            "--script-security 2",
            "plugin /lib/x.so",
            "config other.ovpn",
            "auth-user-pass creds.txt",
        ] {
            assert!(
                problem(Kind::OpenVpn, &format!("{ovpn}{bad}\n")).is_some(),
                "{bad}"
            );
        }
        assert!(problem(Kind::OpenVpn, "client\ndev tun\n").is_some());
        assert_eq!(
            Kind::from_path(std::path::Path::new("/x/Office.OVPN")),
            Some(Kind::OpenVpn)
        );
        assert_eq!(
            imported_uuid(
                "Connection 'office' (6f2b1c1e-6d7c-4a39-9a2e-1d6f3c9b2a11) successfully added."
            )
            .as_deref(),
            Some("6f2b1c1e-6d7c-4a39-9a2e-1d6f3c9b2a11")
        );
    }
}
