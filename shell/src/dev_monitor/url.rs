use super::Port;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::net::IpAddr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlLaunch(String);

impl UrlLaunch {
    pub fn from_port(port: &Port) -> Result<Self, String> {
        if port.protocol != "tcp" || port.port == 0 {
            return Err(tr("shell-only-tcp-url-suggestions-can-be-opened").into());
        }
        let address: IpAddr = port
            .address
            .parse()
            .map_err(|_| tr("shell-the-listening-address-is-not-an-ip-address"))?;
        let host = match address {
            IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_owned(),
            IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_owned(),
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        let expected = format!("http://{host}:{}", port.port);
        if port.url.as_deref() != Some(expected.as_str()) {
            return Err(
                tr("shell-the-url-suggestion-does-not-match-this-listening-address").into(),
            );
        }
        Ok(Self(expected))
    }
    pub fn url(&self) -> &str {
        &self.0
    }
    pub fn start(self, token: Option<String>) -> Result<(), String> {
        if crate::desktop_session::mode().nested() {
            return Err(tr("shell-developer-launches-are-not-enabled-in-this-preview").into());
        }
        let mut command = std::process::Command::new("/usr/bin/xdg-open");
        command
            .arg(self.0)
            .current_dir("/")
            .env_remove("XDG_ACTIVATION_TOKEN")
            .env_remove("DESKTOP_STARTUP_ID");
        if let Some(token) = token {
            command.env("XDG_ACTIVATION_TOKEN", token);
        }
        let mut child = command.spawn().map_err(
            |error| tr!("shell-could-not-start-the-url-handler", "arg0" => format!("{:}", error)),
        )?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn port(address: &str, url: &str) -> Port {
        serde_json::from_value(serde_json::json!({"protocol":"tcp","address":address,"port":8080,"exposure":"all","own":false,"pid":null,"start_time":null,"source":"published","url":url})).unwrap()
    }
    #[test]
    fn only_exact_http_ip_suggestions_are_launchable_without_a_probe() {
        for (address, url) in [
            ("0.0.0.0", "http://127.0.0.1:8080"),
            ("::", "http://[::1]:8080"),
            ("127.0.0.2", "http://127.0.0.2:8080"),
            ("2001:db8::1", "http://[2001:db8::1]:8080"),
        ] {
            assert_eq!(
                UrlLaunch::from_port(&port(address, url)).unwrap().url(),
                url
            );
        }
        for url in [
            "https://127.0.0.1:8080",
            "file:///tmp",
            "http://user@127.0.0.1:8080",
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8081",
            "--help",
            "javascript:alert(1)",
        ] {
            assert!(UrlLaunch::from_port(&port("127.0.0.1", url)).is_err());
        }
        assert!(UrlLaunch::from_port(&port("localhost", "http://localhost:8080")).is_err());
        let mut udp = port("127.0.0.1", "http://127.0.0.1:8080");
        udp.protocol = "udp".into();
        assert!(UrlLaunch::from_port(&udp).is_err());
        udp.protocol = "tcp".into();
        udp.port = 0;
        assert!(UrlLaunch::from_port(&udp).is_err());
    }
}
