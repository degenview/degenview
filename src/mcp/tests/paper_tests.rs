//! The read-only paper-trading tools over a real loopback socket, against the real paper host
//! (in-memory database) that the stub app serves.

use super::{Harness, call, error_code, rpc, tool_names};
use crate::model::DataSource;
use crate::model::mcp::Permission;
use crate::model::paper::{
    PaperAccountSettings, PaperCurrency, PaperInstrument, PaperOrderRequest, PaperOrderSide,
    PaperOrderType, PaperQuote, PaperTimeInForce,
};
use crate::model::watchlist::now_ms;
use serde_json::{Value, json};

const PAPER: &[Permission] = &[Permission::PaperRead];

fn instrument() -> PaperInstrument {
    PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance)
}

fn request(account_id: u64, kind: PaperOrderType, limit: Option<f64>) -> PaperOrderRequest {
    PaperOrderRequest {
        account_id,
        instrument: instrument(),
        side: PaperOrderSide::Buy,
        kind,
        quantity: 1.0,
        limit_price: limit,
        stop_price: None,
        time_in_force: PaperTimeInForce::GoodTilCanceled,
        take_profit: None,
        stop_loss: None,
    }
}

fn quote(bid: f64, ask: f64, timestamp: i64) -> PaperQuote {
    PaperQuote {
        instrument_key: instrument().key,
        bid: Some(bid),
        ask: Some(ask),
        last: Some((bid + ask) / 2.0),
        timestamp,
        is_market_open: true,
    }
}

/// An account with an open long, one working limit order, and one closed trade.
fn seeded(harness: &Harness) -> u64 {
    let mut host = harness.paper();
    let account = host
        .create_account(
            "Main",
            PaperCurrency::Usd,
            100_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    let now = now_ms();
    host.process_quote(quote(99.0, 100.0, now)).unwrap();
    host.submit(request(account, PaperOrderType::Market, None))
        .unwrap();
    host.close_position(account, &instrument().key).unwrap();
    host.process_quote(quote(109.0, 110.0, now + 1)).unwrap();
    host.submit(request(account, PaperOrderType::Market, None))
        .unwrap();
    host.submit(request(account, PaperOrderType::Limit, Some(50.0)))
        .unwrap();
    host.process_quote(quote(119.0, 120.0, now + 2)).unwrap();
    account
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
fn the_paper_tools_are_read_only_and_listed_only_for_paper_read() {
    let harness = Harness::start(&[
        ("paper", "tok", PAPER),
        ("charts", "c-tok", &[Permission::ChartsRead]),
    ]);
    let names = tool_names(&harness.url(), "tok");
    assert_eq!(
        names,
        [
            "degenview_get_paper_account",
            "degenview_list_paper_accounts",
            "degenview_list_paper_fills",
            "degenview_list_paper_trades",
        ]
    );
    assert!(Permission::PaperRead.id() == "paper.read" && !Permission::PaperRead.is_mutating());
    let denied = call(
        &harness.url(),
        "c-tok",
        "degenview_list_paper_accounts",
        json!({}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
}

#[test]
fn an_account_reports_its_equity_positions_and_working_orders() {
    let harness = Harness::start(&[("paper", "tok", PAPER)]);
    let account = seeded(&harness);
    let url = harness.url();

    let listed = call(&url, "tok", "degenview_list_paper_accounts", json!({}));
    assert_eq!(listed["isError"], json!(false), "{listed}");
    let accounts = listed["structuredContent"]["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["name"], "Main");
    assert_eq!(accounts[0]["currency"], "usd");
    assert_eq!(accounts[0]["selected"], true);
    assert_eq!(accounts[0]["run_by_another_mac"], Value::Null);
    // One round trip at 100 in and 99 out lost a point; the long open since 110 is marked at 119.
    assert_eq!(accounts[0]["realized_pnl"], -1.0);
    assert_eq!(accounts[0]["unrealized_pnl"], 9.0);

    let detail = call(
        &url,
        "tok",
        "degenview_get_paper_account",
        json!({"account_id": account.to_string()}),
    );
    let detail = &detail["structuredContent"];
    let positions = detail["positions"].as_array().unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["side"], "long");
    assert_eq!(positions[0]["quantity"], 1.0);
    assert_eq!(positions[0]["average_entry_price"], 110.0);
    assert_eq!(
        positions[0]["mark_price"], 119.0,
        "a long is marked at the bid"
    );
    assert_eq!(positions[0]["unrealized_pnl"], 9.0);
    let orders = detail["open_orders"].as_array().unwrap();
    assert_eq!(orders.len(), 1, "{orders:?}");
    assert_eq!(orders[0]["kind"], "limit");
    assert_eq!(orders[0]["status"], "working");
    assert_eq!(orders[0]["limit_price"], 50.0);
    assert_eq!(orders[0]["remaining_quantity"], 1.0);
}

#[test]
fn fills_and_closed_trades_page_newest_first() {
    let harness = Harness::start(&[("paper", "tok", PAPER)]);
    let account = seeded(&harness).to_string();
    let url = harness.url();

    let fills = call(
        &url,
        "tok",
        "degenview_list_paper_fills",
        json!({"account_id": account, "limit": 2}),
    );
    let fills = &fills["structuredContent"];
    assert_eq!(fills["total"], 3, "buy, sell and the second buy");
    assert_eq!(fills["truncated"], true);
    let rows = fills["fills"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["price"], 110.0, "newest first");
    assert_eq!(rows[0]["side"], "buy");
    assert_eq!(rows[0]["price_source"], "bid_ask");
    let older = call(
        &url,
        "tok",
        "degenview_list_paper_fills",
        json!({"account_id": account, "limit": 2, "offset": 2}),
    );
    assert_eq!(
        older["structuredContent"]["fills"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(older["structuredContent"]["truncated"], false);

    let trades = call(
        &url,
        "tok",
        "degenview_list_paper_trades",
        json!({"account_id": account}),
    );
    let trades = &trades["structuredContent"];
    assert_eq!(trades["total"], 1);
    assert_eq!(trades["trades"][0]["side"], "long");
    assert_eq!(trades["trades"][0]["entry_price"], 100.0);
    assert_eq!(trades["trades"][0]["exit_price"], 99.0);
    assert_eq!(trades["losers"], 1);
    assert_eq!(trades["winners"], 0);
}

#[test]
fn unknown_accounts_and_bad_arguments_are_refused() {
    let harness = Harness::start(&[("paper", "tok", PAPER)]);
    let account = seeded(&harness).to_string();
    let url = harness.url();
    for tool in [
        "degenview_get_paper_account",
        "degenview_list_paper_fills",
        "degenview_list_paper_trades",
    ] {
        let missing = call(&url, "tok", tool, json!({"account_id": "424242"}));
        assert_eq!(error_code(&missing), "NOT_FOUND", "{tool}");
        assert!(
            rejected(&url, "tok", tool, json!({"account_id": "abc"})),
            "{tool}"
        );
        assert!(
            rejected(&url, "tok", tool, json!({"account_id": ""})),
            "{tool}"
        );
        assert!(rejected(&url, "tok", tool, json!({})), "{tool}");
    }
    for extra in [
        json!({"limit": 0}),
        json!({"limit": 201}),
        json!({"offset": -1}),
    ] {
        let mut args = json!({"account_id": account});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(
            rejected(&url, "tok", "degenview_list_paper_fills", args.clone()),
            "{args}"
        );
    }
}
