//! The Windows `ProxyServer` value: either one `host:port` for everything, or a list such as
//! `http=a:80;https=b:443;socks=c:1080` that names a server per protocol.

use super::super::spec::Spec;
use ureq::ProxyProtocol;

/// The server to use for HTTPS and WebSocket traffic: the `https` entry, else `http`, else the
/// `socks` one (SOCKS5), else the single address.
pub fn choose(value: &str) -> Option<Spec> {
    let value = value.trim();
    if !value.contains('=') {
        return Spec::parse(value);
    }
    let entry = |name: &str| {
        value.split(';').find_map(|part| {
            let (key, address) = part.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| address.trim())
        })
    };
    entry("https")
        .or_else(|| entry("http"))
        .and_then(Spec::parse)
        .or_else(|| {
            let mut spec = Spec::parse(entry("socks")?)?;
            spec.protocol = ProxyProtocol::Socks5h;
            Some(spec)
        })
}

/// `ProxyOverride`: entries separated by semicolons. `<local>` (every name without a dot) stays
/// in the list as the rule the resolver applies (`bypass::LOCAL`).
pub fn bypass(value: &str) -> Vec<String> {
    super::super::bypass::split(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_address_is_an_http_proxy() {
        let spec = choose("127.0.0.1:7890").unwrap();
        assert_eq!(spec.protocol, ProxyProtocol::Http);
        assert_eq!((spec.host.as_str(), spec.port), ("127.0.0.1", 7890));
    }

    #[test]
    fn a_per_protocol_list_prefers_https_then_http_then_socks() {
        let spec = choose("http=a:80;https=b:443;socks=c:1080").unwrap();
        assert_eq!((spec.host.as_str(), spec.port), ("b", 443));
        let spec = choose("http=a:80;socks=c:1080").unwrap();
        assert_eq!((spec.host.as_str(), spec.port), ("a", 80));
        let spec = choose("socks=c:1080").unwrap();
        assert_eq!(spec.protocol, ProxyProtocol::Socks5h);
        assert_eq!((spec.host.as_str(), spec.port), ("c", 1080));
    }

    #[test]
    fn a_scheme_in_the_value_is_honoured() {
        let spec = choose("socks5://127.0.0.1:1080").unwrap();
        assert_eq!(spec.protocol, ProxyProtocol::Socks5);
    }

    #[test]
    fn nothing_usable_gives_none() {
        assert!(choose("").is_none());
        assert!(choose("ftp=a:21").is_none());
    }

    #[test]
    fn the_override_list_keeps_local() {
        assert_eq!(
            bypass("*.corp.example;<local>; 10.1.*"),
            ["*.corp.example", "<local>", "10.1.*"]
        );
    }
}
