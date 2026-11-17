//! `scutil --proxy`, which prints the proxy settings of the active network service.

use super::super::spec::Spec;
use super::SystemProxy;
#[cfg(target_os = "macos")]
use std::process::Command;
use ureq::ProxyProtocol;

#[cfg(target_os = "macos")]
pub fn read() -> Option<SystemProxy> {
    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// The enabled server (HTTPS, else HTTP, else SOCKS) and the exceptions list.
pub fn parse(text: &str) -> Option<SystemProxy> {
    let mut fields = std::collections::HashMap::new();
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines().map(str::trim) {
        if in_exceptions {
            if line.starts_with('}') {
                in_exceptions = false;
            } else if let Some((_, value)) = line.split_once(" : ") {
                exceptions.push(value.trim().to_owned());
            }
        } else if line.starts_with("ExceptionsList") {
            in_exceptions = true;
        } else if let Some((key, value)) = line.split_once(" : ") {
            fields.insert(key.trim(), value.trim());
        }
    }
    let server = |prefix: &str, protocol: ProxyProtocol| {
        if fields.get(format!("{prefix}Enable").as_str()) != Some(&"1") {
            return None;
        }
        let host = fields.get(format!("{prefix}Proxy").as_str())?;
        let port = fields.get(format!("{prefix}Port").as_str())?.parse().ok()?;
        Some(Spec {
            protocol,
            host: (*host).to_owned(),
            port,
            username: None,
            password: None,
        })
    };
    let spec = server("HTTPS", ProxyProtocol::Http)
        .or_else(|| server("HTTP", ProxyProtocol::Http))
        .or_else(|| server("SOCKS", ProxyProtocol::Socks5h))?;
    Some(SystemProxy {
        spec,
        bypass: exceptions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
    2 : example.org
  }
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7891
  HTTPSProxy : 127.0.0.2
  SOCKSEnable : 0
}";

    #[test]
    fn the_https_server_and_the_exceptions_are_read() {
        let system = parse(SAMPLE).unwrap();
        assert_eq!(system.spec.host, "127.0.0.2");
        assert_eq!(system.spec.port, 7891);
        assert_eq!(system.spec.protocol, ProxyProtocol::Http);
        assert_eq!(system.bypass, ["*.local", "169.254/16", "example.org"]);
    }

    #[test]
    fn a_disabled_server_is_skipped() {
        let text = SAMPLE.replace("HTTPSEnable : 1", "HTTPSEnable : 0");
        assert_eq!(parse(&text).unwrap().spec.port, 7890);
    }

    #[test]
    fn socks_is_used_when_it_is_all_there_is() {
        let text =
            "<dictionary> {\n  SOCKSEnable : 1\n  SOCKSPort : 1080\n  SOCKSProxy : 10.0.0.5\n}";
        let system = parse(text).unwrap();
        assert_eq!(system.spec.protocol, ProxyProtocol::Socks5h);
        assert_eq!(system.spec.port, 1080);
    }

    #[test]
    fn no_enabled_server_gives_none() {
        assert!(parse("<dictionary> {\n  HTTPEnable : 0\n}").is_none());
        assert!(parse("").is_none());
    }
}
