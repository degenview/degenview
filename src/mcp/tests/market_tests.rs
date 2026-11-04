//! Symbol search, quotes and watchlists over a real loopback socket. Search runs against canned
//! results and quotes against a recorded provider, so nothing here touches the network.

use super::{Harness, call, error_code, rpc, tool_names};
use crate::mcp::state::QuoteAccess;
use crate::model::DataSource;
use crate::model::mcp::Permission;
use crate::model::watchlist_quote::SourceQuote;
use crate::service::SearchResult;
use crate::service::watchlist_quotes::{
    Config, Outcome, QuoteFailure, QuoteProvider, QuoteRequest, WatchlistQuoteBook,
    WatchlistQuoteCoordinator,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const SEARCH: &[Permission] = &[Permission::MarketsRead];
const LISTS: &[Permission] = &[Permission::WatchlistsRead];

/// Prices BTCUSDT and nothing else, like a provider that does not know a symbol.
struct OneMarket;

impl QuoteProvider for OneMarket {
    fn quotes(
        &self,
        _source: DataSource,
        requests: &[QuoteRequest],
    ) -> Result<Outcome, QuoteFailure> {
        let quotes: HashMap<String, SourceQuote> = requests
            .iter()
            .filter(|request| request.symbol == "BTCUSDT")
            .map(|request| {
                (
                    request.symbol.clone(),
                    SourceQuote {
                        price: 64_250.5,
                        previous_day_price: Some(63_000.0),
                        volume_24h: Some(1.5e9),
                        volume_kind: Some(crate::model::watchlist_quote::VolumeKind::QuoteCurrency),
                        timestamp_ms: None,
                    },
                )
            })
            .collect();
        Ok(Outcome::from_quotes(quotes))
    }

    fn resolve_chain(&self, _address: &str) -> Option<String> {
        None
    }
}

/// Binance markets only in the tests that read quotes: a Coinbase market would open the feed's
/// real websocket and be priced from the network.
pub(super) fn feed() -> QuoteAccess {
    let book = WatchlistQuoteBook::default();
    let coordinator = WatchlistQuoteCoordinator::with_provider(
        book.clone(),
        Arc::new(|| {}),
        Arc::new(OneMarket),
        Config {
            poll_interval_ms: 50,
            ..Config::default()
        },
    );
    QuoteAccess { book, coordinator }
}

fn hit(source: DataSource, display: &str, full: &str) -> SearchResult {
    SearchResult {
        symbol: display.into(),
        full_symbol: full.into(),
        source,
        price: None,
        event: None,
    }
}

fn structured(result: &Value) -> &Value {
    assert_eq!(result["isError"], json!(false), "{result}");
    &result["structuredContent"]
}

fn rejected(url: &str, token: &str, name: &str, arguments: Value) -> bool {
    let reply = rpc(
        url,
        Some(token),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    let json = reply.json();
    json.get("error").is_some() || json["result"]["isError"] == json!(true)
}

#[test]
fn the_new_tools_are_listed_only_for_the_permissions_that_unlock_them() {
    let harness = Harness::start(&[
        ("markets", "m-tok", SEARCH),
        ("lists", "l-tok", LISTS),
        ("write", "w-tok", &[Permission::WatchlistsWrite]),
        ("delete", "d-tok", &[Permission::WatchlistsDelete]),
    ]);
    let url = harness.url();
    assert_eq!(
        tool_names(&url, "m-tok"),
        [
            "degenview_get_market_candles",
            "degenview_get_quotes",
            "degenview_search_symbols"
        ]
    );
    assert_eq!(
        tool_names(&url, "l-tok"),
        ["degenview_get_watchlist", "degenview_list_watchlists"]
    );
    assert_eq!(
        tool_names(&url, "w-tok"),
        ["degenview_create_watchlist", "degenview_update_watchlist"]
    );
    assert_eq!(tool_names(&url, "d-tok"), ["degenview_delete_watchlist"]);
}

#[test]
fn search_returns_the_ids_other_tools_take_and_reports_a_failing_provider() {
    let harness = Harness::start(&[("c", "tok", SEARCH)]);
    harness.shared.set_market_search(Arc::new(|source, query| {
        assert_eq!(query, "btc");
        match source {
            DataSource::Binance => Ok(vec![
                hit(DataSource::Binance, "BTC/USDT", "BTCUSDT"),
                hit(DataSource::Binance, "BTC/USDC", "BTCUSDC"),
                // Not this provider's: never reported under it.
                hit(DataSource::Polymarket, "Will BTC...", "0xabc"),
            ]),
            DataSource::Coinbase => Err("Coinbase is down".into()),
            _ => Ok(Vec::new()),
        }
    }));
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_search_symbols",
        json!({"query": " btc ", "sources": ["binance", "coinbase", "binance"], "limit": 2}),
    );
    let found = structured(&result);
    assert_eq!(found["query"], "btc");
    let results = found["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{found}");
    assert_eq!(results[0]["source"], "binance");
    assert_eq!(results[0]["symbol"], "BTCUSDT");
    assert_eq!(results[0]["display"], "BTC/USDT");
    assert_eq!(results[0]["instrument"], "BINANCE:BTCUSDT");
    assert_eq!(found["truncated"], false);
    assert_eq!(found["source_errors"][0]["source"], "coinbase");
    assert_eq!(found["source_errors"][0]["message"], "Coinbase is down");

    // Fewer than the hits: say so.
    let result = call(
        &url,
        "tok",
        "degenview_search_symbols",
        json!({"query": "btc", "sources": ["binance"], "limit": 1}),
    );
    assert_eq!(structured(&result)["truncated"], true);
}

#[test]
fn bad_search_and_quote_arguments_are_refused() {
    let harness = Harness::start(&[("c", "tok", &[Permission::MarketsRead])]);
    harness.shared.attach_quotes(feed());
    harness
        .shared
        .set_market_search(Arc::new(|_, _| panic!("a refused search must not run")));
    let url = harness.url();
    let search = "degenview_search_symbols";
    for arguments in [
        json!({"query": ""}),
        json!({"query": "   "}),
        json!({"query": "a\nb"}),
        json!({"query": "x".repeat(65)}),
        json!({"query": "btc", "limit": 0}),
        json!({"query": "btc", "limit": 26}),
        json!({"query": "btc", "sources": []}),
        json!({"query": "btc", "sources": ["polymarket"]}),
        json!({"query": "btc", "extra": 1}),
    ] {
        assert!(
            rejected(&url, "tok", search, arguments.clone()),
            "{arguments}"
        );
    }
    let quotes = "degenview_get_quotes";
    let many: Vec<Value> = (0..51)
        .map(|n| json!({"source": "alpaca", "symbol": format!("S{n}")}))
        .collect();
    for arguments in [
        json!({"markets": []}),
        json!({"markets": many}),
        json!({"markets": [{"source": "binance", "symbol": "BTC-USD"}]}),
        json!({"markets": [{"source": "coinbase", "symbol": "BTCUSD"}]}),
        json!({"markets": [{"source": "binance", "symbol": "BTCUSDT"}], "wait_seconds": 16}),
        json!({"markets": [{"source": "binance"}]}),
    ] {
        assert!(
            rejected(&url, "tok", quotes, arguments.clone()),
            "{arguments}"
        );
    }
}

#[test]
fn quotes_come_from_the_live_feed_and_the_demand_ends_with_the_call() {
    let harness = Harness::start(&[("c", "tok", SEARCH)]);
    let access = feed();
    harness.shared.attach_quotes(access.clone());
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_get_quotes",
        json!({
            "markets": [
                {"source": "binance", "symbol": "btcusdt"},
                {"source": "binance", "symbol": "ETHUSDT"},
            ],
            "wait_seconds": 1,
        }),
    );
    let quotes = structured(&result);
    let rows = quotes["quotes"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["instrument"], "BINANCE:BTCUSDT");
    assert_eq!(rows[0]["last"], 64_250.5);
    assert_eq!(rows[0]["status"], "live");
    assert_eq!(rows[0]["change_basis"], "rolling_24h");
    assert_eq!(rows[0]["volume_unit"], "quote_currency");
    assert!((rows[0]["change"].as_f64().unwrap() - 1_250.5).abs() < 1e-6);
    // The provider did not know this one: still reported, as unavailable, with its reason, and
    // not left pending since waiting longer will not help.
    assert_eq!(rows[1]["instrument"], "BINANCE:ETHUSDT");
    assert_eq!(rows[1]["last"], Value::Null);
    assert_eq!(rows[1]["status"], "unavailable");
    assert!(rows[1]["note"].is_string(), "{}", rows[1]);
    assert_eq!(quotes["pending"], 0);
    assert!(
        !access.coordinator.has_subscriptions(),
        "the call must not leave its demand on the feed"
    );
}

#[test]
fn quotes_are_unavailable_until_the_app_has_a_feed() {
    let harness = Harness::start(&[("c", "tok", SEARCH)]);
    let result = call(
        &harness.url(),
        "tok",
        "degenview_get_quotes",
        json!({"markets": [{"source": "binance", "symbol": "BTCUSDT"}], "wait_seconds": 0}),
    );
    assert_eq!(error_code(&result), "APP_UNAVAILABLE");
}

#[test]
fn a_watchlist_can_be_created_edited_read_with_quotes_and_deleted() {
    let harness = Harness::start(&[(
        "c",
        "tok",
        &[
            Permission::WatchlistsRead,
            Permission::WatchlistsWrite,
            Permission::WatchlistsDelete,
            Permission::MarketsRead,
        ],
    )]);
    harness.shared.attach_quotes(feed());
    let url = harness.url();

    let listed = call(&url, "tok", "degenview_list_watchlists", json!({}));
    let lists = structured(&listed)["watchlists"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert_eq!(lists[0]["is_favorites"], true);
    assert_eq!(lists[0]["market_count"], 0);

    let created = call(
        &url,
        "tok",
        "degenview_create_watchlist",
        json!({"name": "  Majors ", "markets": [
            {"source": "binance", "symbol": "BTCUSDT"},
            {"source": "binance", "symbol": "btcusdt"},
        ]}),
    );
    let created = structured(&created);
    assert_eq!(created["status"], "created");
    assert_eq!(created["added"], json!(["BINANCE:BTCUSDT"]));
    assert_eq!(created["watchlist"]["name"], "Majors");
    let id = created["watchlist"]["watchlist_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let updated = call(
        &url,
        "tok",
        "degenview_update_watchlist",
        json!({
            "watchlist_id": id,
            "rename": "Core",
            "add": [
                {"source": "binance", "symbol": "BTCUSDT"},
                {"source": "binance", "symbol": "ETHUSDT"},
            ],
            "remove": [{"source": "binance", "symbol": "SOLUSDT"}],
        }),
    );
    let updated = structured(&updated);
    assert_eq!(updated["status"], "updated");
    assert_eq!(updated["added"], json!(["BINANCE:ETHUSDT"]));
    assert_eq!(updated["already_present"], json!(["BINANCE:BTCUSDT"]));
    assert_eq!(updated["not_present"], json!(["BINANCE:SOLUSDT"]));
    assert_eq!(updated["renamed_to"], "Core");
    assert_eq!(updated["watchlist"]["market_count"], 2);

    // The same request again changes nothing.
    let again = call(
        &url,
        "tok",
        "degenview_update_watchlist",
        json!({"watchlist_id": id, "rename": "Core", "add": [{"source": "binance", "symbol": "BTCUSDT"}]}),
    );
    assert_eq!(structured(&again)["status"], "unchanged");

    let read = call(
        &url,
        "tok",
        "degenview_get_watchlist",
        json!({"watchlist_id": id, "include_quotes": true, "wait_seconds": 1}),
    );
    let read = structured(&read);
    let entries = read["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["kind"], "market");
    assert_eq!(entries[0]["source"], "binance");
    assert_eq!(entries[0]["symbol"], "BTCUSDT");
    assert_eq!(entries[0]["quote"]["last"], 64_250.5);
    assert_eq!(entries[1]["instrument"], "BINANCE:ETHUSDT");
    assert_eq!(entries[1]["quote"]["status"], "unavailable");

    // Without quotes the entries carry none, and paging works.
    let page = call(
        &url,
        "tok",
        "degenview_get_watchlist",
        json!({"watchlist_id": id, "offset": 1, "limit": 1}),
    );
    let page = structured(&page);
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["entries"][0]["quote"], Value::Null);
    assert_eq!(page["total_entries"], 2);
    assert_eq!(page["truncated"], false);

    let removed = call(
        &url,
        "tok",
        "degenview_update_watchlist",
        json!({"watchlist_id": id, "remove": [{"source": "binance", "symbol": "BTCUSDT"}]}),
    );
    assert_eq!(structured(&removed)["removed"], json!(["BINANCE:BTCUSDT"]));

    let deleted = call(
        &url,
        "tok",
        "degenview_delete_watchlist",
        json!({"watchlist_id": id}),
    );
    assert_eq!(structured(&deleted)["status"], "deleted");
    let gone = call(
        &url,
        "tok",
        "degenview_get_watchlist",
        json!({"watchlist_id": id}),
    );
    assert_eq!(error_code(&gone), "NOT_FOUND");

    // Favorites stays.
    let favorites = lists[0]["watchlist_id"].as_str().unwrap();
    let refused = call(
        &url,
        "tok",
        "degenview_delete_watchlist",
        json!({"watchlist_id": favorites}),
    );
    assert_eq!(error_code(&refused), "CONFLICT");
}

#[test]
fn a_refused_watchlist_edit_changes_nothing() {
    let harness = Harness::start(&[(
        "c",
        "tok",
        &[Permission::WatchlistsRead, Permission::WatchlistsWrite],
    )]);
    let url = harness.url();
    let created = call(
        &url,
        "tok",
        "degenview_create_watchlist",
        json!({"name": "Keep", "markets": [{"source": "binance", "symbol": "BTCUSDT"}]}),
    );
    let id = structured(&created)["watchlist"]["watchlist_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The name is refused after the markets were fine: neither the removal nor the add happens.
    let refused = call(
        &url,
        "tok",
        "degenview_update_watchlist",
        json!({
            "watchlist_id": id,
            "rename": "bad\u{7}name",
            "add": [{"source": "binance", "symbol": "ETHUSDT"}],
            "remove": [{"source": "binance", "symbol": "BTCUSDT"}],
        }),
    );
    assert_eq!(error_code(&refused), "INVALID_ARGUMENT");
    let read = call(
        &url,
        "tok",
        "degenview_get_watchlist",
        json!({"watchlist_id": id}),
    );
    let read = structured(&read);
    assert_eq!(read["name"], "Keep");
    assert_eq!(read["market_count"], 1);
    assert_eq!(read["entries"][0]["instrument"], "BINANCE:BTCUSDT");
}

#[test]
fn bad_watchlist_arguments_never_reach_the_app() {
    let harness = Harness::start(&[(
        "c",
        "tok",
        &[
            Permission::WatchlistsRead,
            Permission::WatchlistsWrite,
            Permission::WatchlistsDelete,
        ],
    )]);
    let url = harness.url();
    let update = "degenview_update_watchlist";
    let btc = json!([{"source": "binance", "symbol": "BTCUSDT"}]);
    for (tool, arguments) in [
        (update, json!({"watchlist_id": "1"})),
        (update, json!({"watchlist_id": "x", "rename": "a"})),
        (
            update,
            json!({"watchlist_id": "1", "add": btc, "remove": btc}),
        ),
        (
            update,
            json!({"watchlist_id": "1", "add": [{"source": "binance", "symbol": "BTC-USD"}]}),
        ),
        (update, json!({"watchlist_id": "1", "rename": ""})),
        ("degenview_create_watchlist", json!({"name": ""})),
        (
            "degenview_create_watchlist",
            json!({"name": "a".repeat(201)}),
        ),
        (
            "degenview_create_watchlist",
            json!({"name": "ok", "markets": [{"source": "kalshi", "symbol": "X"}]}),
        ),
        (
            "degenview_get_watchlist",
            json!({"watchlist_id": "1", "limit": 0}),
        ),
        (
            "degenview_get_watchlist",
            json!({"watchlist_id": "1", "limit": 501}),
        ),
        ("degenview_get_watchlist", json!({"watchlist_id": "-1"})),
        ("degenview_delete_watchlist", json!({"watchlist_id": ""})),
    ] {
        assert!(
            rejected(&url, "tok", tool, arguments.clone()),
            "{tool} {arguments}"
        );
    }
    // Nothing was created by any of them.
    let listed = call(&url, "tok", "degenview_list_watchlists", json!({}));
    assert_eq!(
        structured(&listed)["watchlists"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn each_watchlist_and_market_permission_gates_only_its_own_tools() {
    let harness = Harness::start(&[
        ("read", "read-tok", LISTS),
        ("write", "write-tok", &[Permission::WatchlistsWrite]),
        ("delete", "delete-tok", &[Permission::WatchlistsDelete]),
        ("markets", "markets-tok", SEARCH),
    ]);
    harness.shared.attach_quotes(feed());
    harness
        .shared
        .set_market_search(Arc::new(|_, _| Ok(Vec::new())));
    let url = harness.url();
    let id = "1";
    let tools = [
        (
            "degenview_search_symbols",
            json!({"query": "btc", "sources": ["binance"]}),
            "markets-tok",
        ),
        (
            "degenview_get_quotes",
            json!({"markets": [{"source": "binance", "symbol": "BTCUSDT"}], "wait_seconds": 0}),
            "markets-tok",
        ),
        ("degenview_list_watchlists", json!({}), "read-tok"),
        (
            "degenview_get_watchlist",
            json!({"watchlist_id": id}),
            "read-tok",
        ),
        (
            "degenview_create_watchlist",
            json!({"name": "Mine"}),
            "write-tok",
        ),
        (
            "degenview_update_watchlist",
            json!({"watchlist_id": id, "rename": "Renamed"}),
            "write-tok",
        ),
        (
            "degenview_delete_watchlist",
            json!({"watchlist_id": id}),
            "delete-tok",
        ),
    ];
    for (tool, arguments, owner) in &tools {
        for token in ["read-tok", "write-tok", "delete-tok", "markets-tok"] {
            if token == *owner {
                continue;
            }
            let result = call(&url, token, tool, arguments.clone());
            assert_eq!(
                error_code(&result),
                "PERMISSION_DENIED",
                "{tool} by {token}"
            );
        }
    }
    // Quotes inside a watchlist need both: reading lists alone is refused.
    let result = call(
        &url,
        "read-tok",
        "degenview_get_watchlist",
        json!({"watchlist_id": id, "include_quotes": true}),
    );
    assert_eq!(error_code(&result), "PERMISSION_DENIED");
}

fn bar(day: i64, close: f64) -> crate::model::Kline {
    let open_time = 1_760_918_400_000 + day * 86_400_000;
    crate::model::Kline {
        open_time,
        close_time: open_time + 86_399_999,
        open: close - 1.0,
        high: close + 2.0,
        low: close - 3.0,
        close,
        volume: 10.0 + day as f64,
    }
}

#[test]
fn candles_of_any_market_come_from_the_provider_with_no_chart() {
    let harness = Harness::start(&[("c", "tok", SEARCH)]);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    {
        let seen = seen.clone();
        harness
            .shared
            .set_candle_fetch(Arc::new(move |source, symbol, interval, limit| {
                seen.lock()
                    .unwrap()
                    .push(format!("{source:?} {symbol} {interval} {limit}"));
                Ok((0..5).map(|day| bar(day, 100.0 + day as f64)).collect())
            }));
    }
    let result = call(
        &harness.url(),
        "tok",
        "degenview_get_market_candles",
        json!({"source": "binance", "symbol": "btcusdt", "timeframe": "1D", "limit": 3}),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    let out = &result["structuredContent"];
    assert_eq!(*seen.lock().unwrap(), ["Binance BTCUSDT 1d 3"]);
    assert_eq!(out["instrument"], "BINANCE:BTCUSDT");
    assert_eq!(out["timeframe"], "1D");
    assert_eq!(
        out["columns"],
        json!(["time", "open", "high", "low", "close", "volume"])
    );
    // The provider gave five; the newest three are returned, oldest first.
    assert_eq!(out["count"], 3);
    assert_eq!(out["bars"][0][4], 102.0);
    assert_eq!(out["bars"][2][4], 104.0);
    assert_eq!(out["summary"]["last_close"], 104.0);
    assert_eq!(out["summary"]["high"], 106.0);
    assert_eq!(out["summary"]["low"], 99.0);
    assert_eq!(out["range"]["to"], "2025-10-24T00:00:00Z");
}

#[test]
fn a_provider_failure_and_an_empty_answer_are_reported_not_hidden() {
    let harness = Harness::start(&[("c", "tok", SEARCH)]);
    let url = harness.url();
    harness
        .shared
        .set_candle_fetch(Arc::new(|_, _, _, _| Err("Binance is down".into())));
    let args = json!({"source": "binance", "symbol": "BTCUSDT", "timeframe": "1H"});
    let failed = call(&url, "tok", "degenview_get_market_candles", args.clone());
    assert_eq!(error_code(&failed), "APP_UNAVAILABLE");
    assert!(
        failed["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains("Binance is down")
    );
    harness
        .shared
        .set_candle_fetch(Arc::new(|_, _, _, _| Ok(Vec::new())));
    let empty = call(&url, "tok", "degenview_get_market_candles", args);
    assert_eq!(structured(&empty)["count"], 0);
    assert!(
        structured(&empty)["notes"]
            .to_string()
            .contains("no candles")
    );
}

#[test]
fn bad_candle_requests_never_reach_the_provider() {
    let harness = Harness::start(&[
        ("c", "tok", SEARCH),
        ("n", "n-tok", &[Permission::ChartsRead]),
    ]);
    harness.shared.set_candle_fetch(Arc::new(|_, _, _, _| {
        panic!("a refused request must not fetch")
    }));
    let url = harness.url();
    let tool = "degenview_get_market_candles";
    for arguments in [
        json!({"source": "binance", "symbol": "BTCUSDT", "timeframe": "4h"}),
        json!({"source": "binance", "symbol": "BTCUSDT"}),
        json!({"source": "binance", "symbol": "BTC-USD", "timeframe": "1D"}),
        json!({"source": "binance", "symbol": "", "timeframe": "1D"}),
        json!({"source": "polymarket", "symbol": "0xabc", "timeframe": "1D"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "timeframe": "1D", "limit": 0}),
        json!({"source": "binance", "symbol": "BTCUSDT", "timeframe": "1D", "limit": 1001}),
        json!({"source": "binance", "symbol": "BTCUSDT", "timeframe": "1D", "before": "2025-01-01T00:00:00Z"}),
    ] {
        assert!(
            rejected(&url, "tok", tool, arguments.clone()),
            "{arguments}"
        );
    }
    let denied = call(
        &url,
        "n-tok",
        tool,
        json!({"source": "binance", "symbol": "BTCUSDT", "timeframe": "1D"}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
}
