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

pub(super) fn manual(host: &str, port: Option<u16>) -> ProxyConfig {
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

/// Credentials are percent-encoded by `ureq` when the proxy is built, so no character in a
/// password can change which server the proxy is.
#[test]
fn a_password_with_url_characters_changes_nothing_else() {
    for password in [
        "with space",
        "has#hash",
        "a/b",
        "q?x",
        "c:o:l",
        "at@sign",
        "100%",
        "p%40ss",
        "p:ss ?#é",
    ] {
        let config = ProxyConfig {
            username: "us/er".to_owned(),
            ..manual("proxy.example", Some(8080))
        };
        let found = resolve(&config, Some(password), &no_env, &nothing)
            .unwrap_or_else(|| panic!("{password:?} made the proxy unusable"));
        assert_eq!(found.proxy.host(), "proxy.example", "{password:?}");
        assert_eq!(found.proxy.port(), 8080, "{password:?}");
        assert_eq!(found.proxy.username(), Some("us/er"), "{password:?}");
        assert_eq!(found.proxy.password(), Some(password), "{password:?}");
    }
}

#[test]
fn an_ipv6_proxy_is_kept_without_brackets_however_it_is_written() {
    let parsed = Spec::parse("http://[::1]:8080").unwrap();
    assert_eq!(parsed.host, "::1");
    for host in ["::1", "[::1]", " [fe80::2] "] {
        let found = resolve(&manual(host, Some(7890)), None, &no_env, &nothing)
            .unwrap_or_else(|| panic!("{host:?} did not resolve"));
        assert!(!found.spec.host.contains('['), "{host:?}");
        assert!(found.spec.host.contains(':'), "{host:?}");
        assert_eq!(found.spec.port, 7890);
        assert!(
            found.spec.describe().starts_with('['),
            "the status line brackets it again"
        );
    }
}

#[test]
fn a_complete_manual_proxy_that_cannot_be_built_is_reported_not_bypassed() {
    let complete = manual("proxy.example", Some(8080));
    let working = resolve(&complete, None, &no_env, &nothing);
    assert!(working.is_some());
    assert_eq!(current::problem(&complete, working.as_ref()), None);
    // Complete, yet nothing came of it: traffic would go direct without a word.
    assert_eq!(
        current::problem(&complete, None),
        Some(current::ProxyProblem::Unusable)
    );
    // Not finished yet, or not Manual: nothing to report.
    assert_eq!(current::problem(&manual("", None), None), None);
    assert_eq!(current::problem(&ProxyConfig::default(), None), None);
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
    assert!(manual("::1", Some(1)).validate().is_ok());
    assert!(manual("[::1", Some(1)).validate().is_err());
    assert!(manual("a[b]", Some(1)).validate().is_err());
    assert!(ProxyConfig::default().validate().is_ok());
}

#[test]
fn an_address_range_in_the_no_proxy_list_is_refused_unless_the_proxy_is_off() {
    let config = ProxyConfig {
        no_proxy: "example.org, 10.0.0.0/8".to_owned(),
        ..ProxyConfig::default()
    };
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("10.0.0.0/8"), "{error}");
    // Off never reads the list (and its field is hidden), so it cannot hold up turning it off.
    let off = ProxyConfig {
        mode: ProxyMode::Off,
        ..config
    };
    assert_eq!(off.validate(), Ok(()));
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

/// What a host *is* decides, not how it is spelled: a public name that starts like a private
/// address (`10.example.com`) goes through the proxy.
#[test]
fn local_means_a_local_address_not_a_name_that_starts_like_one() {
    use super::bypass::is_local;
    for local in [
        "localhost",
        "LOCALHOST",
        "app.localhost",
        "printer.local",
        "nas.local.",
        "127.0.0.1",
        "127.1.2.3",
        "10.0.0.1",
        "10.255.255.255",
        "172.16.0.1",
        "172.31.255.1",
        "192.168.1.20",
        "169.254.10.10",
        "0.0.0.0",
        "::1",
        "[::1]",
        "fe80::1",
        "fe80::1%en0",
        "[fe80::abcd]",
        "febf::1",
        "fc00::1",
        "fd12:3456::1",
        "::ffff:10.0.0.1",
        "[::ffff:192.168.0.9]",
    ] {
        assert!(is_local(local), "{local} is local");
    }
    for public in [
        "10.example.com",
        "127.example.com",
        "192.168.example.com",
        "172.16.evil.test",
        "169.254.example.org",
        "localhost.example.com",
        "mylocalhost",
        "notlocal",
        "api.binance.com",
        "11.0.0.1",
        "172.15.0.1",
        "172.32.0.1",
        "192.169.0.1",
        "169.253.0.1",
        "8.8.8.8",
        "2606:4700::1111",
        "fec0::1",
        "::ffff:8.8.8.8",
        "fdexample.com",
        "fe80.example.com",
    ] {
        assert!(!is_local(public), "{public} is public");
    }
}

#[test]
fn a_proxy_is_skipped_for_local_hosts_only() {
    let found = resolve(
        &manual("proxy.example", Some(8080)),
        None,
        &no_env,
        &nothing,
    )
    .unwrap();
    for local in [
        "10.1.2.3",
        "127.0.0.1",
        "[::1]",
        "::1",
        "printer.local",
        "localhost",
    ] {
        assert!(found.skips(local), "{local}");
    }
    for public in [
        "10.example.com",
        "127.example.com",
        "192.168.example.com",
        "api.binance.com",
    ] {
        assert!(!found.skips(public), "{public}");
    }
    // The user's own entries still work, by `ureq`'s expressions.
    let own = ProxyConfig {
        no_proxy: "corp.test, 100.64.*".to_owned(),
        ..manual("proxy.example", Some(8080))
    };
    let found = resolve(&own, None, &no_env, &nothing).unwrap();
    assert!(found.skips("corp.test") && found.skips("a.corp.test") && found.skips("100.64.1.1"));
}

/// A request to a public name that starts like a private address must reach the proxy, and one
/// to a real private address must not.
#[test]
fn requests_are_routed_by_what_the_host_is() {
    use crate::service::net::http::{self, Retry};
    let (port, proxy_thread) = fake_http_proxy();
    let _applied = AppliedProxy::new(manual("127.0.0.1", Some(port)));
    let body = http::get_text("http://10.example.test/ping", &[], Retry::Never).unwrap();
    assert_eq!(body, "proxy");
    let heads = proxy_thread.join().unwrap();
    assert!(
        heads[0].starts_with("CONNECT 10.example.test:80 HTTP/1.1"),
        "{}",
        heads[0]
    );

    // A private address is dialled directly: nothing listens there, and the (consumed) fake
    // proxy is not asked, so this fails to connect rather than tunnelling.
    let direct = http::get_text("http://127.0.0.1:9/ping", &[], Retry::Never);
    assert!(matches!(direct, Err(http::Error::Network(_))), "{direct:?}");
}

#[test]
fn a_helper_process_follows_the_saved_setting() {
    use crate::service::keychain::api_keys::MemoryBackend;
    use std::time::{Duration, Instant};
    let _applied = AppliedProxy::new(ProxyConfig {
        mode: ProxyMode::Off,
        ..ProxyConfig::default()
    });
    let store = crate::store::Store::open_in_memory().unwrap();
    let passwords = crate::service::keychain::proxy_password::ProxyPasswordStore::with_backend(
        Box::new(MemoryBackend::default()),
    );
    let start = Instant::now() + Duration::from_secs(10);
    let at = |seconds| start + Duration::from_secs(seconds);
    let follow = |seconds| current::follow_with(&store, &passwords, at(seconds));

    // Nothing saved reads as the default, System; the helper was on Off.
    assert!(follow(0));
    assert_eq!(status().mode, ProxyMode::System);
    assert!(!follow(1), "looked at a moment ago");

    let saved = ProxyConfig {
        username: "me".to_owned(),
        ..manual("127.0.0.1", Some(7890))
    };
    saved.save(&store).unwrap();
    assert!(follow(3), "the app saved a manual proxy");
    assert_eq!(status().mode, ProxyMode::Manual);
    let generation = snapshot().generation;
    assert!(!follow(6), "unchanged: nothing applied");
    assert_eq!(snapshot().generation, generation);

    passwords.save("pw").unwrap();
    assert!(follow(40), "the password changed behind the helper's back");
    assert_eq!(
        snapshot().proxy.unwrap().spec.password.as_deref(),
        Some("pw")
    );
    assert!(!follow(80), "the same password again changes nothing");
}

/// Applies a setting to the process-wide proxy for one test, and puts "no proxy" back after.
pub(crate) struct AppliedProxy {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl AppliedProxy {
    pub(crate) fn new(config: ProxyConfig) -> Self {
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

#[test]
fn a_password_never_shows_in_debug_output() {
    let config = ProxyConfig {
        username: "me".to_owned(),
        ..manual("proxy.example", Some(8080))
    };
    let resolved = resolve(&config, Some("hunter2"), &no_env, &nothing).unwrap();
    assert_eq!(resolved.spec.password.as_deref(), Some("hunter2"));
    let text = format!("{resolved:?}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
}

#[test]
fn an_ipv6_host_is_bracketed_only_where_text_needs_it() {
    assert_eq!(super::spec::bracketed("::1"), "[::1]");
    assert_eq!(super::spec::bracketed("[::1]"), "[::1]");
    assert_eq!(super::spec::bracketed("proxy.example"), "proxy.example");
}

/// Building an AI client happens on the frame (Send, Load models); the system's proxy is read on
/// the worker that makes the request, never there.
#[test]
fn building_an_ai_client_does_not_read_the_system_proxy() {
    use crate::service::llm::{HttpProvider, ProviderKind};
    let _applied = AppliedProxy::new(ProxyConfig::default());
    current::age_system_read();
    let before = current::system_reads();
    // No environment proxy is assumed: the read is only skipped when one is set, so a test
    // machine with HTTPS_PROXY still passes, and one without fails if `new` reads.
    HttpProvider::new(ProviderKind::Compatible, "https://example.test/v1", None).unwrap();
    assert_eq!(current::system_reads(), before, "new() blocked on the OS");
}

/// Windows' `<local>` (and macOS's "Exclude simple hostnames"): a name without a dot is on the
/// intranet, so it skips the proxy, but only when the rule is there.
#[test]
fn the_local_rule_skips_names_without_a_dot() {
    let with = |no_proxy: &str| {
        let config = ProxyConfig {
            no_proxy: no_proxy.to_owned(),
            ..manual("proxy.example", Some(8080))
        };
        resolve(&config, None, &no_env, &nothing).unwrap()
    };
    let plain = with("");
    let rule = with("<local>");
    for simple in ["intranet", "nas", "INTRANET", "build-box"] {
        assert!(
            !plain.skips(simple),
            "{simple} goes through the proxy without the rule"
        );
        assert!(rule.skips(simple), "{simple}");
    }
    for dotted in [
        "intranet.example.com",
        "10.example.com",
        "api.binance.com",
        "8.8.8.8",
        "2001:db8::1",
        "[2001:db8::1]",
        "",
    ] {
        assert!(!rule.skips(dotted), "{dotted:?} is not a simple name");
    }
    // The rule is not an expression for `ureq`, whose matcher would treat it as a host name.
    assert!(
        !rule
            .proxy
            .is_no_proxy(&"https://intranet/".parse().unwrap())
    );
    assert_eq!(with("<LOCAL>"), rule, "any case");
    assert!(
        ProxyConfig {
            no_proxy: "<local>".to_owned(),
            ..ProxyConfig::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn the_local_rule_comes_from_every_source() {
    let found = |bypass: &[&str]| {
        let system = || {
            Some(SystemProxy {
                spec: Spec::parse("127.0.0.1:7890").unwrap(),
                bypass: bypass.iter().map(|entry| (*entry).to_owned()).collect(),
            })
        };
        resolve(&ProxyConfig::default(), None, &no_env, &system).unwrap()
    };
    assert!(found(&["<local>"]).skips("intranet"));
    assert!(!found(&["corp.example"]).skips("intranet"));
    let from_env = resolve(
        &ProxyConfig::default(),
        None,
        &env(&[("HTTPS_PROXY", "127.0.0.1:7890"), ("NO_PROXY", "<local>")]),
        &nothing,
    )
    .unwrap();
    assert!(from_env.skips("intranet"));
}

// MARK: Redirects

/// Answers one request per entry of `replies` on a local port, returning what it was asked, and
/// stops waiting for more after two seconds.
fn origin(replies: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let mut heads = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        for reply in replies {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => return heads,
                }
            };
            stream.set_nonblocking(false).unwrap();
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n > 0) {
                head.push(byte[0]);
            }
            heads.push(String::from_utf8_lossy(&head).into_owned());
            let _ = stream.write_all(reply.as_bytes());
        }
        heads
    });
    (port, thread)
}

fn redirect(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn answer(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A fake HTTP proxy that tunnels every `CONNECT` and answers the request inside with
/// `redirect`, for two seconds; returns every request head it saw.
fn redirecting_proxy(location: String) -> (u16, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let mut heads = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            for reply in [
                "HTTP/1.1 200 Connection established\r\n\r\n".to_owned(),
                redirect(&location),
            ] {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n > 0) {
                    head.push(byte[0]);
                }
                heads.push(String::from_utf8_lossy(&head).into_owned());
                let _ = stream.write_all(reply.as_bytes());
            }
        }
        heads
    });
    (port, thread)
}

/// A public page that redirects to this machine must not carry that request to the proxy: the
/// route is decided again for every hop, not once for the first URL.
#[test]
fn a_redirect_to_a_local_address_is_not_sent_through_the_proxy() {
    use crate::service::net::http::{self, Retry};
    let (origin_port, origin_thread) = origin(vec![answer("internal")]);
    let (proxy_port, proxy_thread) =
        redirecting_proxy(format!("http://127.0.0.1:{origin_port}/secret"));
    let _applied = AppliedProxy::new(manual("127.0.0.1", Some(proxy_port)));
    let body = http::get_text(
        "http://public.test/start",
        &[("X-Api-Key", "k")],
        Retry::Never,
    )
    .unwrap();
    assert_eq!(body, "internal");
    let proxied = proxy_thread.join().unwrap();
    assert_eq!(proxied.len(), 2, "one CONNECT and its GET: {proxied:?}");
    assert!(
        proxied[0].starts_with("CONNECT public.test:80"),
        "{proxied:?}"
    );
    let asked = origin_thread.join().unwrap();
    assert!(asked[0].starts_with("GET /secret HTTP/1.1"), "{asked:?}");
    assert!(
        !asked[0].to_ascii_lowercase().contains("x-api-key"),
        "a header meant for public.test went to another host: {asked:?}"
    );
}

/// The other way round: a local first hop connects directly, and a redirect to a public name goes
/// through the proxy.
#[test]
fn a_redirect_from_a_local_address_to_a_public_host_uses_the_proxy() {
    use crate::service::net::http::{self, Retry};
    let (proxy_port, proxy_thread) = fake_http_proxy();
    let (origin_port, origin_thread) = origin(vec![redirect("http://public.test/after")]);
    let _applied = AppliedProxy::new(manual("127.0.0.1", Some(proxy_port)));
    let body = http::get_text(
        &format!("http://127.0.0.1:{origin_port}/start"),
        &[],
        Retry::Never,
    )
    .unwrap();
    assert_eq!(body, "proxy");
    let heads = proxy_thread.join().unwrap();
    assert!(heads[0].starts_with("CONNECT public.test:80"), "{heads:?}");
    assert!(origin_thread.join().unwrap()[0].starts_with("GET /start"));
}

#[test]
fn redirects_are_followed_up_to_a_limit_and_headers_stay_on_their_host() {
    use crate::service::net::http::{self, Error, Retry};
    let _applied = AppliedProxy::new(ProxyConfig {
        mode: ProxyMode::Off,
        ..ProxyConfig::default()
    });
    // A relative Location stays on the same host and keeps the header.
    let (port, thread) = origin(vec![redirect("next"), answer("done")]);
    let url = format!("http://127.0.0.1:{port}/a/start");
    assert_eq!(
        http::get_text(&url, &[("X-Api-Key", "k")], Retry::Never).unwrap(),
        "done"
    );
    let asked = thread.join().unwrap();
    assert!(asked[1].starts_with("GET /a/next HTTP/1.1"), "{asked:?}");
    assert!(
        asked[1].to_ascii_lowercase().contains("x-api-key"),
        "{asked:?}"
    );
    // Six hops are too many.
    let (port, thread) = origin(vec![redirect("/loop"); 8]);
    let result = http::get_text(&format!("http://127.0.0.1:{port}/loop"), &[], Retry::Never);
    assert!(
        matches!(&result, Err(Error::Network(text)) if text.contains("redirect")),
        "{result:?}"
    );
    thread.join().unwrap();
}
