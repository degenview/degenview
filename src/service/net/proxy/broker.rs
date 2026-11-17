//! The proxy setting as the `degenview-brokers` crate sees it. The crate knows how to connect
//! through a route but not which route to use: [`BrokerNetwork`] answers that from the same
//! decisions every other connection in the app is routed by (mode, bypass list, the system's
//! proxy), asked again for every request, so live trading follows a change at once.

use super::spec::Spec;
use super::tunnel::TunnelError;
use super::{dial, route_for_url};
use degenview_brokers::{Network, Proxy, ProxyCredentials, ProxyProtocol, Route};
use std::io;
use std::net::TcpStream;
use std::time::Duration;
use ureq::ProxyProtocol as Protocol;

/// The [`Network`] live trading runs on. Stateless: every answer comes from the proxy in force.
#[derive(Clone, Copy, Debug, Default)]
pub struct BrokerNetwork;

impl Network for BrokerNetwork {
    fn route(&self, url: &str) -> Route {
        route_for_url(url).map_or(Route::Direct, |resolved| to_broker_route(&resolved.spec))
    }

    fn dial(&self, host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
        dial(host, port, timeout).map_err(into_io_error)
    }
}

/// The route through the proxy `spec` describes.
fn to_broker_route(spec: &Spec) -> Route {
    let proxy = Proxy::new(broker_protocol(spec.protocol), &spec.host, spec.port);
    Route::Proxy(match &spec.username {
        Some(username) => proxy.with_credentials(ProxyCredentials::new(
            username,
            spec.password.as_deref().unwrap_or_default(),
        )),
        None => proxy,
    })
}

/// SOCKS4a is the crate's SOCKS4 and SOCKS5h its SOCKS5: both let the proxy resolve the name.
fn broker_protocol(protocol: Protocol) -> ProxyProtocol {
    match protocol {
        Protocol::Https => ProxyProtocol::Https,
        Protocol::Socks4 | Protocol::Socks4A => ProxyProtocol::Socks4,
        Protocol::Socks5 | Protocol::Socks5h => ProxyProtocol::Socks5,
        // `Http`, and any protocol a later `ureq` adds: still a proxy route, so a request that
        // cannot be spoken fails instead of going out directly.
        _ => ProxyProtocol::Http,
    }
}

/// A tunnel failure as an I/O error with the same message, which the crate shows on the order
/// screen and in its logs.
fn into_io_error(error: TunnelError) -> io::Error {
    let kind = match &error {
        TunnelError::Connect(inner) | TunnelError::Proxy(inner) => inner.kind(),
        TunnelError::Unsupported(_) => io::ErrorKind::Unsupported,
        TunnelError::Refused(_) => io::ErrorKind::ConnectionRefused,
        TunnelError::Protocol(_) => io::ErrorKind::InvalidData,
    };
    io::Error::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::tests::manual;
    use super::super::{AppliedProxy, ProxyConfig, ProxyMode};
    use super::*;

    fn spec(text: &str) -> Spec {
        Spec::parse(text).expect("a proxy address")
    }

    fn proxy_of(route: Route) -> Proxy {
        match route {
            Route::Proxy(proxy) => proxy,
            Route::Direct => panic!("expected a proxy route"),
        }
    }

    #[test]
    fn each_protocol_maps_to_the_crates_equivalent() {
        let cases = [
            ("http://p.example:8080", ProxyProtocol::Http),
            ("https://p.example:8080", ProxyProtocol::Https),
            ("socks4://p.example:1080", ProxyProtocol::Socks4),
            ("socks4a://p.example:1080", ProxyProtocol::Socks4),
            ("socks5://p.example:1080", ProxyProtocol::Socks5),
            ("socks5h://p.example:1080", ProxyProtocol::Socks5),
        ];
        for (text, expected) in cases {
            let proxy = proxy_of(to_broker_route(&spec(text)));
            assert_eq!(proxy.protocol(), expected, "{text}");
            assert_eq!(proxy.host(), "p.example");
            assert!(proxy.credentials().is_none());
        }
    }

    #[test]
    fn the_address_and_credentials_carry_over() {
        let proxy = proxy_of(to_broker_route(&spec("http://me:s3cret@p.example:3128")));
        assert_eq!((proxy.host(), proxy.port()), ("p.example", 3128));
        let credentials = proxy.credentials().expect("credentials");
        assert_eq!(
            (credentials.username(), credentials.password()),
            ("me", "s3cret")
        );
    }

    #[test]
    fn a_user_name_without_a_password_sends_an_empty_one_and_an_ipv6_host_stays_bare() {
        let proxy = proxy_of(to_broker_route(&spec("socks5://me@[::1]:1080")));
        assert_eq!(proxy.host(), "::1");
        assert_eq!(proxy.credentials().expect("credentials").password(), "");
    }

    #[test]
    fn the_debug_output_of_a_mapped_route_hides_the_password() {
        let route = to_broker_route(&spec("http://me:s3cret@p.example:3128"));
        assert!(!format!("{route:?}").contains("s3cret"));
    }

    #[test]
    fn a_tunnel_failure_keeps_its_message() {
        let error = into_io_error(TunnelError::Refused("HTTP/1.1 403 Forbidden".to_owned()));
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert!(error.to_string().contains("403 Forbidden"), "{error}");
        let error = into_io_error(TunnelError::Unsupported("SOCKS4"));
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("SOCKS4"), "{error}");
    }

    #[test]
    fn a_manual_proxy_routes_a_broker_host_through_it_and_a_loopback_mock_directly() {
        let _applied = AppliedProxy::new(manual("proxy.example", Some(8080)));
        let through = proxy_of(BrokerNetwork.route("https://api.broker.example/v1"));
        assert_eq!((through.host(), through.port()), ("proxy.example", 8080));
        // The websocket address is routed the same way.
        let socket = proxy_of(BrokerNetwork.route("wss://stream.broker.example"));
        assert_eq!(socket.host(), "proxy.example");

        for local in [
            "http://127.0.0.1:4000",
            "ws://localhost:4001",
            "http://[::1]:9",
        ] {
            assert_eq!(BrokerNetwork.route(local), Route::Direct, "{local}");
        }
    }

    #[test]
    fn with_the_proxy_off_every_broker_route_is_direct() {
        let _applied = AppliedProxy::new(ProxyConfig {
            mode: ProxyMode::Off,
            ..ProxyConfig::default()
        });
        assert_eq!(
            BrokerNetwork.route("https://api.broker.example/v1"),
            Route::Direct
        );
    }

    #[test]
    fn a_change_of_setting_is_followed_by_the_next_answer() {
        let applied = AppliedProxy::new(manual("proxy.example", Some(8080)));
        assert!(matches!(
            BrokerNetwork.route("https://api.broker.example"),
            Route::Proxy(_)
        ));
        drop(applied);
        assert_eq!(
            BrokerNetwork.route("https://api.broker.example"),
            Route::Direct
        );
    }
}
