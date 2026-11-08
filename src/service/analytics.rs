//! Anonymous usage counts: how many people run DegenView, on what, and (from the request's IP,
//! worked out by Google, never sent by us) from which country. See `docs/analytics.md`.
//!
//! One Google Analytics 4 Measurement Protocol POST per launch. This module only builds the request
//! and decides whether the process may send at all; whether the *user* allows it, the client id and
//! when to send are `host::analytics_host`.

use super::http;
use super::update_check::Channel;
use serde_json::{Value, json};

/// The Measurement Protocol collection endpoint.
pub const ENDPOINT: &str = "https://www.google-analytics.com/mp/collect";

/// Sent once per launch.
pub const EVENT_APP_OPEN: &str = "app_open";
/// Sent until it has been accepted once on this install. (GA reserves `first_open`.)
pub const EVENT_NEW_INSTALL: &str = "new_install";

/// The GA4 data stream this build reports to. Set at build time (`DEGENVIEW_GA_MEASUREMENT_ID`,
/// `DEGENVIEW_GA_API_SECRET`); a build without both sends nothing. Both ship inside the binary, so
/// they are identifiers, not secrets: anyone can read them and post fake events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    pub measurement_id: &'static str,
    pub api_secret: &'static str,
}

impl Config {
    /// The IDs compiled in, if both are present and look like what GA hands out.
    pub fn from_build() -> Option<Self> {
        Self::new(
            option_env!("DEGENVIEW_GA_MEASUREMENT_ID")?,
            option_env!("DEGENVIEW_GA_API_SECRET")?,
        )
    }

    /// `None` unless both parts are non-empty and made of URL-safe characters, so they can go in
    /// the query string as they are.
    pub fn new(measurement_id: &'static str, api_secret: &'static str) -> Option<Self> {
        let safe = |text: &str| {
            !text.is_empty()
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        };
        (safe(measurement_id) && safe(api_secret)).then_some(Self {
            measurement_id,
            api_secret,
        })
    }

    pub fn url(&self) -> String {
        format!(
            "{ENDPOINT}?measurement_id={}&api_secret={}",
            self.measurement_id, self.api_secret
        )
    }
}

/// Everything about this copy that is ever sent. Adding a field here is a privacy decision: update
/// `docs/analytics.md` and the About page's sentence with it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Context {
    pub app_version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    /// `stable` or `rc`: the update channel the user is on.
    pub update_channel: &'static str,
    /// `flatpak` or `native`. Nothing finer is knowable: a dmg, a zip and Homebrew are the same app.
    pub packaging: &'static str,
}

impl Context {
    pub fn current(channel: Channel) -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            update_channel: channel.key(),
            packaging: if super::sandbox::is_flatpak() {
                "flatpak"
            } else {
                "native"
            },
        }
    }
}

/// The request body: one `app_open`, preceded by `new_install` when this install has not been
/// counted yet. `client_id` is the random per-install id; nothing else identifies anyone.
pub fn payload(client_id: &str, new_install: bool, context: &Context) -> Value {
    let params = json!({
        "app_version": context.app_version,
        "os": context.os,
        "arch": context.arch,
        "update_channel": context.update_channel,
        "packaging": context.packaging,
        // Without an engagement time GA4 drops the event from its real-time and user reports.
        "engagement_time_msec": 1,
    });
    let mut events = Vec::new();
    if new_install {
        events.push(json!({ "name": EVENT_NEW_INSTALL, "params": params }));
    }
    events.push(json!({ "name": EVENT_APP_OPEN, "params": params }));
    json!({ "client_id": client_id, "events": events })
}

/// Blocking: one POST. Call it from a worker thread. A failure is for the log only.
pub fn send(config: &Config, body: &Value) -> Result<(), http::Error> {
    http::post_json(&config.url(), &body.to_string()).map(|_| ())
}

/// Whether the environment asks not to be counted: `DEGENVIEW_NO_ANALYTICS` or the community
/// `DO_NOT_TRACK` convention set to anything but empty or `0`.
pub fn environment_opts_out(get: impl Fn(&str) -> Option<String>) -> bool {
    ["DEGENVIEW_NO_ANALYTICS", "DO_NOT_TRACK"]
        .iter()
        .any(|name| {
            get(name).is_some_and(|value| {
                let value = value.trim();
                !value.is_empty() && value != "0"
            })
        })
}

/// The config this process may use: the compiled-in one, unless this is a debug or test build
/// (a developer's runs must not count as users) or the environment opts out.
pub fn process_config() -> Option<Config> {
    if cfg!(debug_assertions) || cfg!(test) {
        return None;
    }
    if environment_opts_out(|name| std::env::var(name).ok()) {
        return None;
    }
    Config::from_build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            app_version: "1.2.3",
            os: "macos",
            arch: "aarch64",
            update_channel: "stable",
            packaging: "native",
        }
    }

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn the_request_carries_only_the_id_and_the_listed_parameters() {
        let body = payload("abc123", false, &context());
        assert_eq!(keys(&body), ["client_id", "events"]);
        let events = body["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["name"], "app_open");
        assert_eq!(keys(&events[0]), ["name", "params"]);
        // The privacy guard: a new parameter must be added here on purpose, and documented.
        assert_eq!(
            keys(&events[0]["params"]),
            [
                "app_version",
                "arch",
                "engagement_time_msec",
                "os",
                "packaging",
                "update_channel"
            ]
        );
        assert_eq!(body["client_id"], "abc123");
    }

    #[test]
    fn a_new_install_is_counted_once_before_the_launch() {
        let body = payload("abc123", true, &context());
        let names: Vec<_> = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["new_install", "app_open"]);
    }

    #[test]
    fn nothing_about_the_user_is_in_the_body() {
        let text = payload("abc123", true, &Context::current(Channel::Stable)).to_string();
        for forbidden in ["build", "user", "path", "host", "symbol", "ip"] {
            assert!(
                !text.contains(&format!("\"{forbidden}")),
                "{forbidden} in {text}"
            );
        }
    }

    #[test]
    fn ids_must_be_present_and_url_safe() {
        assert!(Config::new("G-ABC123", "s3cret_-x").is_some());
        assert!(Config::new("", "x").is_none());
        assert!(Config::new("G-ABC", "").is_none());
        assert!(Config::new("G-ABC", "a&b").is_none());
        assert!(Config::new("G ABC", "x").is_none());
        let config = Config::new("G-ABC", "secret").unwrap();
        assert_eq!(
            config.url(),
            "https://www.google-analytics.com/mp/collect?measurement_id=G-ABC&api_secret=secret"
        );
    }

    #[test]
    fn the_environment_can_opt_out() {
        let with = |name: &'static str, value: &'static str| {
            move |asked: &str| (asked == name).then(|| value.to_owned())
        };
        assert!(!environment_opts_out(|_| None));
        assert!(environment_opts_out(with("DO_NOT_TRACK", "1")));
        assert!(environment_opts_out(with("DEGENVIEW_NO_ANALYTICS", "yes")));
        assert!(!environment_opts_out(with("DO_NOT_TRACK", "0")));
        assert!(!environment_opts_out(with("DO_NOT_TRACK", " ")));
    }

    #[test]
    fn a_test_or_debug_build_never_has_a_config() {
        assert_eq!(process_config(), None);
    }
}
