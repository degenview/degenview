use super::*;
use crate::service::market::ports::cache::CacheStore;
use std::time::Duration;

#[test]
fn the_venues_are_built_once_and_shared() {
    assert!(std::ptr::eq(venues(), venues()));
}

#[test]
fn the_apps_lookup_cache_keeps_what_a_venue_puts_in_it() {
    let cache: Arc<dyn CacheStore> = context().cache;
    assert!(
        cache
            .get::<Vec<String>>("wiring_test", "list", Duration::from_secs(60))
            .is_none()
    );
    cache.put("wiring_test", "list", &vec!["BTCUSDT".to_owned()]);
    let hit = cache
        .get::<Vec<String>>("wiring_test", "list", Duration::from_secs(60))
        .expect("what was put");
    assert_eq!(hit.value, ["BTCUSDT"]);
    assert!(hit.fresh);
    // Another shape is no answer.
    assert!(
        cache
            .get::<u32>("wiring_test", "list", Duration::from_secs(60))
            .is_none()
    );
}

mod transport {
    use super::super::http::{limiter, send};
    use crate::service::market::ports::http::{Error, Pace, Request, Retry};
    use crate::service::net::http::{self, MAX_BODY_BYTES};
    use crate::service::net::rate_limit;
    use std::cell::RefCell;

    #[test]
    fn each_host_keeps_its_own_schedule() {
        assert!(std::ptr::eq(
            limiter(Pace::CoinGecko),
            rate_limit::coingecko()
        ));
        assert!(std::ptr::eq(
            limiter(Pace::GeckoTerminal),
            rate_limit::geckoterminal()
        ));
        assert!(std::ptr::eq(limiter(Pace::Kraken), rate_limit::kraken()));
        assert!(!std::ptr::eq(
            limiter(Pace::CoinGecko),
            limiter(Pace::GeckoTerminal)
        ));
        assert!(!std::ptr::eq(
            limiter(Pace::GeckoTerminal),
            limiter(Pace::Kraken)
        ));
        assert!(!std::ptr::eq(
            limiter(Pace::Kraken),
            limiter(Pace::CoinGecko)
        ));
    }

    /// What `send` handed to the transport, and when the wait for the host's turn came.
    #[derive(Debug, Eq, PartialEq)]
    enum Step {
        Waited(Pace),
        Got {
            url: String,
            headers: Vec<(String, String)>,
            retry: http::Retry,
            limit: u64,
        },
    }

    fn steps_of(
        request: &Request<'_>,
        answer: Result<String, http::Error>,
    ) -> (Vec<Step>, Result<String, Error>) {
        let steps = RefCell::new(Vec::new());
        let result = send(
            request,
            |pace| steps.borrow_mut().push(Step::Waited(pace)),
            |url, headers, retry, limit| {
                steps.borrow_mut().push(Step::Got {
                    url: url.to_owned(),
                    headers: headers
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                        .collect(),
                    retry,
                    limit,
                });
                answer
            },
        );
        (steps.into_inner(), result)
    }

    #[test]
    fn a_request_is_sent_as_it_was_made_under_the_transports_usual_limit() {
        let request = Request::get("https://example.test/a")
            .headers(&[("X-Key", "k")])
            .retry(Retry::Never);
        let (steps, result) = steps_of(&request, Ok("body".to_owned()));
        assert_eq!(result, Ok("body".to_owned()));
        assert_eq!(
            steps,
            [Step::Got {
                url: "https://example.test/a".to_owned(),
                headers: vec![("X-Key".to_owned(), "k".to_owned())],
                retry: http::Retry::Never,
                limit: MAX_BODY_BYTES,
            }],
            "no pace: no wait"
        );
        let (steps, _) = steps_of(&Request::get("https://example.test/b"), Ok(String::new()));
        assert!(matches!(
            steps[..],
            [Step::Got {
                retry: http::Retry::Transient,
                ..
            }]
        ));
    }

    #[test]
    fn a_body_limit_the_request_asks_for_is_the_one_the_transport_gets() {
        let request = Request::get("https://example.test/big").max_body_bytes(16 * 1024 * 1024);
        let (steps, _) = steps_of(&request, Ok(String::new()));
        assert!(matches!(
            steps[..],
            [Step::Got {
                limit: 16_777_216,
                ..
            }]
        ));
    }

    #[test]
    fn a_host_with_a_schedule_is_waited_for_before_the_request_is_sent() {
        let request = Request::get("https://example.test/c").paced(Pace::Kraken);
        let (steps, _) = steps_of(&request, Ok(String::new()));
        assert_eq!(steps[0], Step::Waited(Pace::Kraken));
        assert!(matches!(steps[1], Step::Got { .. }));
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn a_failure_reaches_the_venue_as_the_ports_own_error() {
        for (failure, told) in [
            (http::Error::Status(503), Error::Status(503)),
            (http::Error::BodyTooLarge(9), Error::BodyTooLarge(9)),
            (
                http::Error::Network("reset".to_owned()),
                Error::Network("reset".to_owned()),
            ),
        ] {
            let (_, result) = steps_of(&Request::get("https://example.test/d"), Err(failure));
            assert_eq!(result, Err(told));
        }
    }

    #[test]
    fn the_port_and_the_transport_agree_on_which_failures_may_pass() {
        let failures = (0..=700u16).map(http::Error::Status).chain([
            http::Error::BodyTooLarge(1),
            http::Error::Network("reset".to_owned()),
        ]);
        for failure in failures {
            let transient = failure.is_transient();
            assert_eq!(
                Error::from(failure.clone()).is_transient(),
                transient,
                "{failure:?}"
            );
        }
    }
}

mod credentials {
    use super::super::credentials::KeychainCredentials;
    use crate::model::market::oanda::{OandaCredentials, OandaEnvironment};
    use crate::service::keychain::api_keys::{
        AlpacaCredentials, AlpacaCredentialsStore, CoinMarketCapKeyStore, MemoryBackend,
        OandaCredentialsStore,
    };
    use crate::service::market::ports::credentials::Credentials;

    struct Stores {
        alpaca: AlpacaCredentialsStore,
        oanda: OandaCredentialsStore,
        cmc: CoinMarketCapKeyStore,
    }

    /// Stores in memory, none shared with the app's.
    fn stores() -> Stores {
        Stores {
            alpaca: AlpacaCredentialsStore::with_backend(Box::new(MemoryBackend::default())),
            oanda: OandaCredentialsStore::with_backend(Box::new(MemoryBackend::default())),
            cmc: CoinMarketCapKeyStore::with_backend(Box::new(MemoryBackend::default())),
        }
    }

    fn over(stores: &Stores, env: fn(&str) -> Option<String>) -> KeychainCredentials<'_> {
        KeychainCredentials::over(&stores.alpaca, &stores.oanda, &stores.cmc, env)
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn each_venue_is_given_the_login_saved_for_it() {
        let stores = stores();
        stores
            .alpaca
            .save(&AlpacaCredentials {
                key_id: "alpaca-id".to_owned(),
                secret_key: "alpaca-secret".to_owned(),
            })
            .unwrap();
        stores
            .oanda
            .save(&OandaCredentials {
                account_id: "101-001".to_owned(),
                access_token: "oanda-token".to_owned(),
                environment: OandaEnvironment::Practice,
            })
            .unwrap();
        stores.cmc.save("cmc-key").unwrap();
        let credentials = over(&stores, no_env);
        assert_eq!(
            credentials.alpaca(),
            Some(("alpaca-id".to_owned(), "alpaca-secret".to_owned()))
        );
        assert_eq!(
            credentials.oanda(),
            Some(OandaCredentials {
                account_id: "101-001".to_owned(),
                access_token: "oanda-token".to_owned(),
                environment: OandaEnvironment::Practice,
            })
        );
        assert_eq!(credentials.cmc().as_deref(), Some("cmc-key"));
    }

    #[test]
    fn a_venue_with_nothing_saved_is_not_given_another_ones_login() {
        let stores = stores();
        stores.cmc.save("cmc-key").unwrap();
        let credentials = over(&stores, no_env);
        assert_eq!(credentials.alpaca(), None);
        assert_eq!(credentials.oanda(), None);
        assert_eq!(credentials.cmc().as_deref(), Some("cmc-key"));
    }

    #[test]
    fn without_a_saved_login_each_venue_reads_its_own_environment_variables() {
        fn env(name: &str) -> Option<String> {
            Some(
                match name {
                    "DEGENVIEW_ALPACA_KEY_ID" => "env-alpaca-id",
                    "DEGENVIEW_ALPACA_SECRET_KEY" => "env-alpaca-secret",
                    "DEGENVIEW_OANDA_ACCOUNT_ID" => "env-account",
                    "DEGENVIEW_OANDA_ACCESS_TOKEN" => "env-token",
                    "DEGENVIEW_CMC_API_KEY" => "env-cmc",
                    _ => return None,
                }
                .to_owned(),
            )
        }
        let stores = stores();
        let credentials = over(&stores, env);
        assert_eq!(
            credentials.alpaca(),
            Some(("env-alpaca-id".to_owned(), "env-alpaca-secret".to_owned()))
        );
        let oanda = credentials.oanda().expect("a login");
        assert_eq!(
            (oanda.account_id.as_str(), oanda.access_token.as_str()),
            ("env-account", "env-token")
        );
        assert_eq!(credentials.cmc().as_deref(), Some("env-cmc"));
    }
}
