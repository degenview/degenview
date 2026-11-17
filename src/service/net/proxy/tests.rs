use super::config::ProxyError;
use super::*;
use os::SystemProxy;
use ureq::ProxyProtocol;

fn no_env(_: &str) -> Option<String> {
    None
}

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    }
}

fn system_proxy() -> Option<SystemProxy> {
    Some(SystemProxy {
        spec: Spec::parse("127.0.0.1:7890").unwrap(),
        bypass: vec!["corp.example".to_owned()],
    })
}

fn nothing() -> Option<SystemProxy> {
    None
}

fn manual(host: &str, port: Option<u16>) -> ProxyConfig {
    ProxyConfig {
        mode: ProxyMode::Manual,
        host: host.to_owned(),
        port,
        ..ProxyConfig::default()
    }
}

#[test]
fn the_default_follows_the_system() {
    assert_eq!(ProxyConfig::default().mode, ProxyMode::System);
}

#[test]
fn off_uses_no_proxy_whatever_the_environment_and_system_say() {
    let config = ProxyConfig {
        mode: ProxyMode::Off,
        ..ProxyConfig::default()
    };
    let vars = [("HTTPS_PROXY", "http://env:1")];
    assert!(resolve(&config, None, &env(&vars), &system_proxy).is_none());
}

#[test]
fn the_environment_comes_before_the_system() {
    let vars = [("HTTPS_PROXY", "http://envproxy:3128")];
    let found = resolve(&ProxyConfig::default(), None, &env(&vars), &system_proxy).unwrap();
    assert_eq!(found.source, Source::Environment);
    assert_eq!(
        (found.spec.host.as_str(), found.spec.port),
        ("envproxy", 3128)
    );
}

#[test]
fn all_proxy_wins_over_https_proxy_and_lowercase_is_read() {
    let vars = [("https_proxy", "http://b:2"), ("all_proxy", "socks5://a:1")];
    let found = resolve(&ProxyConfig::default(), None, &env(&vars), &nothing).unwrap();
    assert_eq!(found.spec.protocol, ProxyProtocol::Socks5);
    assert_eq!(found.spec.host, "a");
}

#[test]
fn an_unreadable_variable_is_skipped() {
    let vars = [("HTTPS_PROXY", "http://"), ("HTTP_PROXY", "http://ok:80")];
    let found = resolve(&ProxyConfig::default(), None, &env(&vars), &nothing).unwrap();
    assert_eq!(found.spec.host, "ok");
}

#[test]
fn the_system_proxy_is_used_when_the_environment_names_none() {
    let found = resolve(&ProxyConfig::default(), None, &no_env, &system_proxy).unwrap();
    assert_eq!(found.source, Source::System);
    assert_eq!(found.spec.port, 7890);
    assert!(found.skips("corp.example"));
    assert!(found.skips("www.corp.example"));
    assert!(!found.skips("api.binance.com"));
}

#[test]
fn no_proxy_anywhere_means_direct() {
    assert!(resolve(&ProxyConfig::default(), None, &no_env, &nothing).is_none());
}

#[test]
fn the_system_is_not_asked_when_the_environment_answers() {
    let vars = [("HTTP_PROXY", "http://env:80")];
    let asked = std::cell::Cell::new(false);
    let system = || {
        asked.set(true);
        None
    };
    resolve(&ProxyConfig::default(), None, &env(&vars), &system).unwrap();
    assert!(!asked.get());
}

#[test]
fn manual_ignores_the_environment_and_the_system() {
    let vars = [("HTTPS_PROXY", "http://env:1")];
    let found = resolve(
        &manual("my.proxy", Some(8080)),
        None,
        &env(&vars),
        &system_proxy,
    )
    .unwrap();
    assert_eq!(found.source, Source::Manual);
    assert_eq!(found.spec.protocol, ProxyProtocol::Http);
    assert_eq!(
        (found.spec.host.as_str(), found.spec.port),
        ("my.proxy", 8080)
    );
}

#[test]
fn a_manual_socks5_proxy_resolves_names_itself() {
    let config = ProxyConfig {
        protocol: Protocol::Socks5,
        ..manual("127.0.0.1", Some(1080))
    };
    let found = resolve(&config, None, &no_env, &nothing).unwrap();
    assert_eq!(found.spec.protocol, ProxyProtocol::Socks5h);
}

#[test]
fn a_manual_login_carries_the_password() {
    let config = ProxyConfig {
        username: " me ".to_owned(),
        ..manual("p", Some(1))
    };
    let found = resolve(&config, Some("s3cret!"), &no_env, &nothing).unwrap();
    assert_eq!(found.spec.username.as_deref(), Some("me"));
    assert_eq!(found.spec.password.as_deref(), Some("s3cret!"));
    let found = resolve(&manual("p", Some(1)), Some("ignored"), &no_env, &nothing).unwrap();
    assert_eq!(found.spec.username, None);
    assert_eq!(found.spec.password, None);
}

#[test]
fn an_incomplete_manual_proxy_resolves_to_nothing() {
    assert!(resolve(&manual("", Some(1)), None, &no_env, &nothing).is_none());
    assert!(resolve(&manual("p", None), None, &no_env, &nothing).is_none());
}

#[test]
fn validation_says_what_is_missing() {
    assert_eq!(manual("", Some(1)).validate(), Err(ProxyError::MissingHost));
    assert_eq!(manual("p", None).validate(), Err(ProxyError::MissingPort));
    assert_eq!(
        manual("http://p", Some(1)).validate(),
        Err(ProxyError::BadHost("http://p".to_owned()))
    );
    assert!(manual("p.example", Some(1)).validate().is_ok());
    assert!(manual("[::1]", Some(1)).validate().is_ok());
    assert!(ProxyConfig::default().validate().is_ok());
}

#[test]
fn an_address_range_in_the_no_proxy_list_is_refused_in_every_mode() {
    let config = ProxyConfig {
        no_proxy: "example.org, 10.0.0.0/8".to_owned(),
        ..ProxyConfig::default()
    };
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("10.0.0.0/8"), "{error}");
}

#[test]
fn the_users_no_proxy_list_adds_to_every_source() {
    let config = ProxyConfig {
        no_proxy: "intranet.test; *.corp.test".to_owned(),
        ..ProxyConfig::default()
    };
    let found = resolve(&config, None, &no_env, &system_proxy).unwrap();
    assert!(found.skips("intranet.test"));
    assert!(found.skips("wiki.intranet.test"));
    assert!(found.skips("a.corp.test"));
    assert!(found.skips("corp.example"), "the system's own list stays");
    let found = resolve(
        &ProxyConfig {
            no_proxy: "intranet.test".to_owned(),
            ..manual("p", Some(1))
        },
        None,
        &no_env,
        &nothing,
    )
    .unwrap();
    assert!(found.skips("intranet.test"));
    let vars = [
        ("HTTPS_PROXY", "http://p:1"),
        ("NO_PROXY", "a.test,.b.test"),
    ];
    let found = resolve(&ProxyConfig::default(), None, &env(&vars), &nothing).unwrap();
    assert!(found.skips("a.test") && found.skips("x.b.test") && !found.skips("c.test"));
}

#[test]
fn local_and_private_hosts_always_connect_directly() {
    let found = resolve(&manual("p", Some(1)), None, &no_env, &nothing).unwrap();
    for host in [
        "localhost",
        "127.0.0.1",
        "127.1.2.3",
        "10.1.2.3",
        "192.168.0.10",
        "172.16.0.1",
        "172.31.255.1",
        "169.254.1.1",
        "printer.local",
        "::1",
        "fe80::1",
        "fd12:3456::1",
    ] {
        assert!(found.skips(host), "{host} should bypass");
    }
    for host in [
        "api.binance.com",
        "172.32.0.1",
        "172.15.0.1",
        "11.0.0.1",
        "8.8.8.8",
        "fcbarcelona.com",
    ] {
        assert!(!found.skips(host), "{host} should use the proxy");
    }
}

#[test]
fn the_setting_round_trips_through_the_store_and_stays_local() {
    let store = crate::store::Store::open_in_memory().unwrap();
    assert_eq!(ProxyConfig::load(&store), ProxyConfig::default());
    let config = ProxyConfig {
        username: "me".to_owned(),
        no_proxy: "a.test".to_owned(),
        protocol: Protocol::Socks5,
        ..manual("p", Some(1080))
    };
    config.save(&store).unwrap();
    assert_eq!(ProxyConfig::load(&store), config);
    assert_eq!(
        crate::sync::record::policy::setting_policy(ProxyConfig::KEY),
        crate::sync::record::policy::SettingPolicy::Local
    );
}

#[test]
fn a_partial_saved_setting_decodes_to_defaults() {
    let config: ProxyConfig = serde_json::from_str(r#"{"mode":"manual","host":"p"}"#).unwrap();
    assert_eq!(config.mode, ProxyMode::Manual);
    assert_eq!(config.protocol, Protocol::Http);
    assert_eq!(config.port, None);
}

#[test]
fn describe_names_the_server_and_a_non_http_protocol() {
    assert_eq!(
        Spec::parse("127.0.0.1:7890").unwrap().describe(),
        "127.0.0.1:7890"
    );
    assert_eq!(
        Spec::parse("socks5://127.0.0.1:1080").unwrap().describe(),
        "127.0.0.1:1080 (SOCKS5)"
    );
}

/// The agents every request uses are built from `net::config_builder`, so none ignores the
/// proxy setting: nothing outside `service/net` may start from `ureq`'s own builder.
#[test]
fn no_agent_is_built_without_the_proxy_setting() {
    fn visit(dir: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                let code = text.split("#[cfg(test)]").next().unwrap_or_default();
                let name = path.to_string_lossy();
                let exempt = name.contains("service/net/proxy/")
                    || name.ends_with("service/net/agent.rs")
                    || name.contains("/tests/")
                    || name.ends_with("tests.rs");
                if !exempt
                    && (code.contains("Agent::config_builder()")
                        || code.contains("Agent::new_with_defaults()"))
                {
                    found.push(path.display().to_string());
                }
            }
        }
    }
    let mut found = Vec::new();
    visit(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    assert!(found.is_empty(), "built without the proxy: {found:?}");
}

/// Applies a setting to the process-wide proxy for one test, and puts "no proxy" back after.
struct AppliedProxy {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl AppliedProxy {
    fn new(config: ProxyConfig) -> Self {
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = crate::lock::lock(&ONE_AT_A_TIME);
        apply(config, None);
        Self { _serial: guard }
    }
}

impl Drop for AppliedProxy {
    fn drop(&mut self) {
        apply(
            ProxyConfig {
                mode: ProxyMode::Off,
                ..ProxyConfig::default()
            },
            None,
        );
    }
}

/// A fake HTTP proxy for one connection: it tunnels `CONNECT` and answers the request inside it.
fn fake_http_proxy() -> (u16, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut heads = Vec::new();
        for reply in [
            "HTTP/1.1 200 Connection established\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nproxy",
        ] {
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap() == 0 {
                    return heads;
                }
                head.push(byte[0]);
            }
            heads.push(String::from_utf8(head).unwrap());
            stream.write_all(reply.as_bytes()).unwrap();
        }
        heads
    });
    (port, thread)
}

#[test]
fn requests_follow_the_proxy_setting_as_it_changes() {
    use crate::service::net::http::{self, Retry};
    let (port, proxy_thread) = fake_http_proxy();
    let _applied = AppliedProxy::new(manual("127.0.0.1", Some(port)));
    // A host that does not exist can only be reached through the proxy.
    let body = http::get_text("http://example.test/ping", &[], Retry::Never).unwrap();
    assert_eq!(body, "proxy");
    let heads = proxy_thread.join().unwrap();
    assert!(
        heads[0].starts_with("CONNECT example.test:80 HTTP/1.1"),
        "{}",
        heads[0]
    );
    assert!(heads[1].starts_with("GET /ping HTTP/1.1"), "{}", heads[1]);

    apply(
        ProxyConfig {
            mode: ProxyMode::Off,
            ..ProxyConfig::default()
        },
        None,
    );
    let direct = http::get_text("http://example.test/ping", &[], Retry::Never);
    assert!(matches!(direct, Err(http::Error::Network(_))), "{direct:?}");
}

#[test]
fn the_status_names_the_proxy_and_where_it_came_from() {
    let _applied = AppliedProxy::new(manual("proxy.example", Some(8080)));
    let status = status();
    assert_eq!(status.mode, ProxyMode::Manual);
    let active = status.active.unwrap();
    assert_eq!(active.source, Source::Manual);
    assert_eq!(active.server, "proxy.example:8080");
    apply(manual("", None), None);
    assert_eq!(super::status().active, None);
}
