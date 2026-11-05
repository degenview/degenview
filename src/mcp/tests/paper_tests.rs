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

// MARK: Writing

const TRADE: &[Permission] = &[
    Permission::PaperOrdersPlace,
    Permission::PaperOrdersCancel,
    Permission::PaperPositionsClose,
];

/// An account with a live quote for BTCUSDT (bid 99, ask 100) and nothing else.
fn funded(harness: &Harness) -> u64 {
    let mut host = harness.paper();
    let account = host
        .create_account(
            "Main",
            PaperCurrency::Usd,
            100_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    host.process_quote(quote(99.0, 100.0, now_ms())).unwrap();
    account
}

fn place(url: &str, token: &str, account: u64, extra: Value) -> Value {
    let mut args = json!({
        "account_id": account.to_string(),
        "source": "binance",
        "symbol": "BTCUSDT",
        "side": "buy",
        "order_type": "market",
        "quantity": 1.0,
    });
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(url, token, "degenview_place_paper_order", args)
}

#[test]
fn each_paper_write_permission_gates_only_its_own_tools() {
    let harness = Harness::start(&[
        ("place", "p-tok", &[Permission::PaperOrdersPlace]),
        ("cancel", "c-tok", &[Permission::PaperOrdersCancel]),
        ("close", "x-tok", &[Permission::PaperPositionsClose]),
        ("read", "r-tok", PAPER),
    ]);
    let url = harness.url();
    let account = funded(&harness);
    assert_eq!(
        tool_names(&url, "p-tok"),
        [
            "degenview_modify_paper_order",
            "degenview_place_paper_order"
        ]
    );
    assert_eq!(tool_names(&url, "c-tok"), ["degenview_cancel_paper_orders"]);
    assert_eq!(
        tool_names(&url, "x-tok"),
        ["degenview_close_paper_positions"]
    );
    let a = account.to_string();
    let tools = [
        (
            "degenview_place_paper_order",
            json!({"account_id": a, "source": "binance", "symbol": "BTCUSDT", "side": "buy", "order_type": "market", "quantity": 1.0}),
            "p-tok",
        ),
        (
            "degenview_modify_paper_order",
            json!({"order_id": "1", "limit_price": 5.0}),
            "p-tok",
        ),
        (
            "degenview_cancel_paper_orders",
            json!({"account_id": a, "all": true}),
            "c-tok",
        ),
        (
            "degenview_close_paper_positions",
            json!({"account_id": a, "all": true}),
            "x-tok",
        ),
    ];
    for (tool, arguments, owner) in &tools {
        for token in ["p-tok", "c-tok", "x-tok", "r-tok"] {
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
    // Nothing ran: the account is as it was.
    let host = harness.paper();
    assert!(host.snapshot().orders.is_empty());
    assert!(host.snapshot().fills.is_empty());
}

#[test]
fn a_market_order_fills_at_the_ask_and_a_close_takes_the_position_off() {
    let harness = Harness::start(&[
        ("t", "tok", TRADE),
        ("r", "r-tok", PAPER),
        (
            "rw",
            "rw-tok",
            &[Permission::PaperOrdersPlace, Permission::PaperRead],
        ),
    ]);
    let url = harness.url();
    let account = funded(&harness);

    let placed = place(&url, "rw-tok", account, json!({}));
    assert_eq!(placed["isError"], json!(false), "{placed}");
    let out = &placed["structuredContent"];
    assert_eq!(out["status"], "placed");
    assert_eq!(out["order"]["status"], "filled");
    assert_eq!(out["order"]["kind"], "market");
    assert_eq!(out["fills"][0]["price"], 100.0, "a buy fills at the ask");
    assert_eq!(out["fills"][0]["side"], "buy");
    // Marked at the bid it is a point under water.
    assert_eq!(out["account"]["unrealized_pnl"], -1.0);
    assert!(out["account"]["available_funds"].as_f64().unwrap() < 100_000.0);

    let detail = call(
        &url,
        "r-tok",
        "degenview_get_paper_account",
        json!({"account_id": account.to_string()}),
    );
    assert_eq!(detail["structuredContent"]["positions"][0]["quantity"], 1.0);

    let closed = call(
        &url,
        "tok",
        "degenview_close_paper_positions",
        json!({"account_id": account.to_string(), "source": "binance", "symbol": "btcusdt"}),
    );
    assert_eq!(closed["structuredContent"]["account"], Value::Null);
    let out = &closed["structuredContent"];
    assert_eq!(out["status"], "closed", "{closed}");
    assert_eq!(out["fills"][0]["price"], 99.0, "selling fills at the bid");
    assert_eq!(out["failures"], json!([]));
    let detail = call(
        &url,
        "r-tok",
        "degenview_get_paper_account",
        json!({"account_id": account.to_string()}),
    );
    assert_eq!(detail["structuredContent"]["positions"], json!([]));
    let again = call(
        &url,
        "tok",
        "degenview_close_paper_positions",
        json!({"account_id": account.to_string(), "all": true}),
    );
    assert_eq!(again["structuredContent"]["status"], "nothing_to_do");
    let named = call(
        &url,
        "tok",
        "degenview_close_paper_positions",
        json!({"account_id": account.to_string(), "source": "binance", "symbol": "BTCUSDT"}),
    );
    assert_eq!(error_code(&named), "NOT_FOUND");
}

#[test]
fn a_limit_order_works_can_be_changed_and_canceled() {
    let harness = Harness::start(&[("t", "tok", TRADE)]);
    let url = harness.url();
    let account = funded(&harness);
    let placed = place(
        &url,
        "tok",
        account,
        json!({"order_type": "limit", "limit_price": 50.0}),
    );
    let order = &placed["structuredContent"]["order"];
    assert_eq!(order["status"], "working", "{placed}");
    assert_eq!(placed["structuredContent"]["fills"], json!([]));
    let id = order["order_id"].as_str().unwrap().to_owned();

    let modified = call(
        &url,
        "tok",
        "degenview_modify_paper_order",
        json!({"order_id": id, "limit_price": 60.0, "quantity": 2.0}),
    );
    let out = &modified["structuredContent"];
    assert_eq!(out["status"], "modified", "{modified}");
    assert_eq!(out["orders"][0]["limit_price"], 60.0);
    assert_eq!(out["orders"][0]["quantity"], 2.0);

    let canceled = call(
        &url,
        "tok",
        "degenview_cancel_paper_orders",
        json!({"account_id": account.to_string(), "order_ids": [id, "999999"]}),
    );
    let out = &canceled["structuredContent"];
    assert_eq!(out["status"], "canceled", "{canceled}");
    assert_eq!(out["orders"][0]["status"], "canceled");
    assert_eq!(
        out["failures"].as_array().unwrap().len(),
        1,
        "an order that is not the account's"
    );
    let gone = call(
        &url,
        "tok",
        "degenview_modify_paper_order",
        json!({"order_id": id, "limit_price": 61.0}),
    );
    assert_ne!(
        gone["isError"],
        json!(false),
        "a canceled order cannot change: {gone}"
    );

    let nothing = call(
        &url,
        "tok",
        "degenview_cancel_paper_orders",
        json!({"account_id": account.to_string(), "all": true}),
    );
    assert_eq!(nothing["structuredContent"]["status"], "nothing_to_do");
}

#[test]
fn a_dry_run_places_nothing_and_a_request_id_makes_a_retry_safe() {
    let harness = Harness::start(&[("t", "tok", TRADE)]);
    let url = harness.url();
    let account = funded(&harness);

    let dry = place(&url, "tok", account, json!({"dry_run": true}));
    assert_eq!(dry["structuredContent"]["status"], "would_place", "{dry}");
    assert_eq!(dry["structuredContent"]["order"], Value::Null);
    assert!(
        harness.paper().snapshot().orders.is_empty(),
        "a dry run records nothing, not even an order"
    );
    // A dry run of an order that would be refused says why, and still records nothing.
    let too_big = place(
        &url,
        "tok",
        account,
        json!({"dry_run": true, "quantity": 1.0e9}),
    );
    assert_ne!(too_big["isError"], json!(false), "{too_big}");
    assert!(harness.paper().snapshot().orders.is_empty());

    let first = place(&url, "tok", account, json!({"request_id": "run-1.a"}));
    let second = place(&url, "tok", account, json!({"request_id": "run-1.a"}));
    assert_eq!(first["structuredContent"]["status"], "placed");
    assert_eq!(second["structuredContent"]["status"], "already_placed");
    assert_eq!(
        first["structuredContent"]["order"]["order_id"],
        second["structuredContent"]["order"]["order_id"]
    );
    assert_eq!(
        harness.paper().snapshot().orders.len(),
        1,
        "one order, not two"
    );
    let other = place(&url, "tok", account, json!({"request_id": "run-2"}));
    assert_eq!(other["structuredContent"]["status"], "placed");
    assert_eq!(harness.paper().snapshot().orders.len(), 2);
}

#[test]
fn a_bracket_reports_its_exit_orders_and_an_unpriced_market_is_refused() {
    let harness = Harness::start(&[("t", "tok", TRADE)]);
    let url = harness.url();
    let account = funded(&harness);
    let bracket = place(
        &url,
        "tok",
        account,
        json!({"take_profit": 120.0, "stop_loss": 90.0}),
    );
    let out = &bracket["structuredContent"];
    assert_eq!(out["order"]["status"], "filled", "{bracket}");
    let roles: Vec<_> = out["bracket_orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|order| order["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles.len(), 2, "{out}");
    assert!(roles.contains(&"take_profit") && roles.contains(&"stop_loss"));

    // No chart, so no quote for this market: refused with the engine's reason.
    let unpriced = call(
        &url,
        "tok",
        "degenview_place_paper_order",
        json!({"account_id": account.to_string(), "source": "binance", "symbol": "ETHUSDT",
               "side": "buy", "order_type": "market", "quantity": 1.0}),
    );
    assert_eq!(error_code(&unpriced), "CONFLICT", "{unpriced}");
    assert!(
        unpriced["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains("price")
    );
}

#[test]
fn an_account_another_mac_runs_is_not_changed_over_mcp() {
    let harness = Harness::start(&[("t", "tok", TRADE), ("r", "r-tok", PAPER)]);
    let url = harness.url();
    let mine = funded(&harness);
    // iCloud Sync brings an account another Mac runs.
    {
        let store = harness.paper_store();
        let guard = store.lock().unwrap();
        let mut snapshot = guard.paper_trading().unwrap();
        snapshot
            .accounts
            .push(crate::model::paper::PaperAccount::new(77, "From iCloud", 1));
        guard.save_paper_trading(&snapshot).unwrap();
        guard.set_paper_owner("77", Some("other-mac")).unwrap();
    }
    harness.paper().reload();
    assert!(harness.paper().owned_elsewhere(77).is_some());

    // Reading it is fine and says who runs it.
    let listed = call(&url, "r-tok", "degenview_list_paper_accounts", json!({}));
    let accounts = listed["structuredContent"]["accounts"].as_array().unwrap();
    let theirs = accounts.iter().find(|a| a["account_id"] == "77").unwrap();
    assert!(theirs["run_by_another_mac"].as_str().is_some());

    // Changing it is refused on every path, and nothing is taken over.
    let place = place(&url, "tok", 77, json!({}));
    assert_eq!(error_code(&place), "CONFLICT", "{place}");
    assert!(
        place["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains("take it over")
    );
    for (tool, arguments) in [
        (
            "degenview_cancel_paper_orders",
            json!({"account_id": "77", "all": true}),
        ),
        (
            "degenview_close_paper_positions",
            json!({"account_id": "77", "all": true}),
        ),
    ] {
        let result = call(&url, "tok", tool, arguments);
        assert_eq!(error_code(&result), "CONFLICT", "{tool}");
    }
    assert!(
        harness.paper().owned_elsewhere(77).is_some(),
        "still not this Mac's"
    );
    // Mine still works.
    let own = super::paper_tests::place(&url, "tok", mine, json!({}));
    assert_eq!(own["structuredContent"]["status"], "placed");
    let missing = super::paper_tests::place(&url, "tok", 424_242, json!({}));
    assert_eq!(error_code(&missing), "NOT_FOUND");
}

#[test]
fn bad_paper_orders_never_reach_the_engine() {
    let harness = Harness::start(&[("t", "tok", TRADE)]);
    let url = harness.url();
    let account = funded(&harness).to_string();
    let tool = "degenview_place_paper_order";
    let base = json!({"account_id": account, "source": "binance", "symbol": "BTCUSDT",
        "side": "buy", "order_type": "market", "quantity": 1.0});
    for extra in [
        json!({"quantity": 0.0}),
        json!({"quantity": -1.0}),
        json!({"quantity": 1.0e16}),
        json!({"order_type": "limit"}),
        json!({"order_type": "limit", "limit_price": 0.0}),
        json!({"order_type": "market", "limit_price": 5.0}),
        json!({"order_type": "stop"}),
        json!({"order_type": "stop", "stop_price": 5.0, "limit_price": 5.0}),
        json!({"order_type": "stop_limit", "stop_price": 5.0}),
        json!({"order_type": "iceberg"}),
        json!({"side": "short"}),
        json!({"take_profit": -3.0}),
        json!({"request_id": ""}),
        json!({"request_id": "has space"}),
        json!({"request_id": "x".repeat(65)}),
        json!({"symbol": "BTC-USD"}),
        json!({"source": "polymarket"}),
        json!({"leverage": 10}),
    ] {
        let mut args = base.clone();
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(rejected(&url, "tok", tool, args.clone()), "{args}");
    }
    for (tool, arguments) in [
        ("degenview_modify_paper_order", json!({"order_id": "1"})),
        (
            "degenview_modify_paper_order",
            json!({"order_id": "x", "limit_price": 1.0}),
        ),
        (
            "degenview_modify_paper_order",
            json!({"order_id": "1", "quantity": 0.0}),
        ),
        (
            "degenview_cancel_paper_orders",
            json!({"account_id": account}),
        ),
        (
            "degenview_cancel_paper_orders",
            json!({"account_id": account, "all": true, "order_ids": ["1"]}),
        ),
        (
            "degenview_cancel_paper_orders",
            json!({"account_id": account, "order_ids": []}),
        ),
        (
            "degenview_cancel_paper_orders",
            json!({"account_id": account, "all": false}),
        ),
        (
            "degenview_close_paper_positions",
            json!({"account_id": account}),
        ),
        (
            "degenview_close_paper_positions",
            json!({"account_id": account, "all": true, "source": "binance", "symbol": "BTCUSDT"}),
        ),
        (
            "degenview_close_paper_positions",
            json!({"account_id": account, "source": "binance"}),
        ),
    ] {
        assert!(
            rejected(&url, "tok", tool, arguments.clone()),
            "{tool} {arguments}"
        );
    }
    let host = harness.paper();
    assert!(
        host.snapshot().orders.is_empty(),
        "no refused call placed anything"
    );
}

#[test]
fn a_write_grant_alone_never_returns_the_account() {
    let harness = Harness::start(&[
        ("p", "p-tok", &[Permission::PaperOrdersPlace]),
        ("c", "c-tok", &[Permission::PaperOrdersCancel]),
        ("x", "x-tok", &[Permission::PaperPositionsClose]),
        (
            "pr",
            "pr-tok",
            &[Permission::PaperOrdersPlace, Permission::PaperRead],
        ),
        (
            "cr",
            "cr-tok",
            &[Permission::PaperOrdersCancel, Permission::PaperRead],
        ),
    ]);
    let url = harness.url();
    let account = funded(&harness);
    let a = account.to_string();
    let none = |result: &Value, what: &str| {
        assert_eq!(result["isError"], json!(false), "{what}: {result}");
        assert_eq!(
            result["structuredContent"]["account"],
            Value::Null,
            "{what}"
        );
    };
    let shown = |result: &Value, what: &str| {
        assert_eq!(result["isError"], json!(false), "{what}: {result}");
        assert!(
            result["structuredContent"]["account"]["equity"].is_number(),
            "{what}: {result}"
        );
    };

    // Dry run, then a real order.
    none(
        &place(&url, "p-tok", account, json!({"dry_run": true})),
        "dry run",
    );
    shown(
        &place(&url, "pr-tok", account, json!({"dry_run": true})),
        "dry run with read",
    );
    none(&place(&url, "p-tok", account, json!({})), "placed");

    // A no-op cancel and a no-op close still must not show the balances.
    none(
        &call(
            &url,
            "c-tok",
            "degenview_cancel_paper_orders",
            json!({"account_id": a, "all": true}),
        ),
        "cancel nothing",
    );
    shown(
        &call(
            &url,
            "cr-tok",
            "degenview_cancel_paper_orders",
            json!({"account_id": a, "all": true}),
        ),
        "cancel nothing with read",
    );
    // A partial failure (an order that is not this account's) keeps the same rule.
    let partial = call(
        &url,
        "c-tok",
        "degenview_cancel_paper_orders",
        json!({"account_id": a, "order_ids": ["999999"]}),
    );
    none(&partial, "partial failure");
    assert_eq!(
        partial["structuredContent"]["failures"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    none(
        &call(
            &url,
            "x-tok",
            "degenview_close_paper_positions",
            json!({"account_id": a, "all": true}),
        ),
        "close",
    );
}
