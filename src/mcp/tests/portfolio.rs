//! The read-only portfolio tools over a real loopback socket, against the real portfolio host
//! (in-memory database) and a recorded quote provider.

use super::market::feed;
use super::{Harness, call, error_code, rpc, tool_names};
use crate::model::DataSource;
use crate::model::mcp::Permission;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioTransaction, PortfolioTransactionType,
};
use crate::model::watchlist::new_id;
use serde_json::{Value, json};
use std::collections::HashMap;

const PORTFOLIO: &[Permission] = &[Permission::PortfolioRead];

fn asset(key: &str, quote: PortfolioCurrency) -> PortfolioAsset {
    PortfolioAsset {
        key: key.into(),
        symbol: "BTC/USDT".into(),
        name: "Bitcoin".into(),
        source: DataSource::Binance,
        quote_currency: quote,
        metadata: HashMap::new(),
    }
}

fn transaction(
    portfolio_id: u64,
    key: &str,
    kind: PortfolioTransactionType,
    quantity: f64,
    price: f64,
    currency: PortfolioCurrency,
    timestamp: i64,
) -> PortfolioTransaction {
    PortfolioTransaction {
        id: new_id(),
        portfolio_id,
        asset: asset(key, PortfolioCurrency::Usd),
        kind,
        quantity,
        price: Some(price),
        price_currency: currency,
        fee: 0.0,
        fee_currency: currency,
        timestamp,
        notes: "note".into(),
        source: Default::default(),
        external_transaction_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Two bitcoin buys, one sale, an asset bought and sold out, and one bought in euros.
fn seeded(harness: &Harness) -> u64 {
    use PortfolioTransactionType::{Buy, Sell};
    let mut host = harness.portfolio();
    let id = host
        .create_portfolio("Main", PortfolioCurrency::Usd)
        .unwrap();
    let usd = PortfolioCurrency::Usd;
    for tx in [
        transaction(id, "BINANCE:BTCUSDT", Buy, 2.0, 50_000.0, usd, 1_000),
        transaction(id, "BINANCE:BTCUSDT", Buy, 2.0, 60_000.0, usd, 2_000),
        transaction(id, "BINANCE:BTCUSDT", Sell, 1.0, 70_000.0, usd, 3_000),
        transaction(id, "BINANCE:OLDUSDT", Buy, 1.0, 10.0, usd, 1_500),
        transaction(id, "BINANCE:OLDUSDT", Sell, 1.0, 15.0, usd, 2_500),
        transaction(
            id,
            "BINANCE:ETHUSDT",
            Buy,
            4.0,
            100.0,
            PortfolioCurrency::Eur,
            4_000,
        ),
    ] {
        host.add_transaction(tx).unwrap();
    }
    id
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
fn the_portfolio_tools_need_portfolio_read_and_nothing_else_unlocks_them() {
    let harness = Harness::start(&[
        ("p", "tok", PORTFOLIO),
        ("o", "o-tok", &[Permission::PaperRead]),
    ]);
    let url = harness.url();
    assert_eq!(
        tool_names(&url, "tok"),
        [
            "degenview_get_portfolio_holdings",
            "degenview_list_portfolio_transactions",
            "degenview_list_portfolios",
        ]
    );
    for tool in [
        "degenview_list_portfolios",
        "degenview_get_portfolio_holdings",
    ] {
        let denied = call(&url, "o-tok", tool, json!({}));
        assert_eq!(error_code(&denied), "PERMISSION_DENIED", "{tool}");
    }
}

#[test]
fn portfolios_are_listed_with_their_transaction_counts() {
    let harness = Harness::start(&[("p", "tok", PORTFOLIO)]);
    let id = seeded(&harness);
    let listed = call(
        &harness.url(),
        "tok",
        "degenview_list_portfolios",
        json!({}),
    );
    assert_eq!(listed["isError"], json!(false), "{listed}");
    let out = &listed["structuredContent"];
    let portfolios = out["portfolios"].as_array().unwrap();
    assert_eq!(portfolios.len(), 1);
    assert_eq!(portfolios[0]["portfolio_id"], id.to_string());
    assert_eq!(portfolios[0]["name"], "Main");
    assert_eq!(portfolios[0]["base_currency"], "usd");
    assert_eq!(portfolios[0]["transaction_count"], 6);
    assert_eq!(out["privacy_mode"], false);
}

#[test]
fn holdings_are_valued_per_currency_and_a_foreign_cost_gets_no_profit() {
    let harness = Harness::start(&[("p", "tok", PORTFOLIO)]);
    harness.shared.attach_quotes(feed());
    let id = seeded(&harness);
    let result = call(
        &harness.url(),
        "tok",
        "degenview_get_portfolio_holdings",
        json!({"portfolio_id": id.to_string(), "wait_seconds": 1}),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    let out = &result["structuredContent"];
    let holdings = out["holdings"].as_array().unwrap();
    let by = |asset: &str| holdings.iter().find(|h| h["asset"] == asset).unwrap();
    // Three coins held at an average cost of 55,000 after selling one at that average.
    let btc = by("BINANCE:BTCUSDT");
    assert_eq!(btc["quantity"], 3.0);
    assert_eq!(btc["cost_basis"], 165_000.0);
    assert_eq!(btc["average_cost"], 55_000.0);
    assert_eq!(btc["realized_pnl"], 15_000.0);
    assert_eq!(btc["price"], 64_250.5);
    assert_eq!(btc["value"], 192_751.5);
    assert_eq!(btc["unrealized_pnl"], 27_751.5);
    assert_eq!(btc["currency_mismatch"], false);
    assert_eq!(btc["price_status"], "live");
    // Bought in euros but quoted in dollars: priced (the provider does not know it) is not
    // enough to make a profit up, and it is kept out of the totals.
    let eth = by("BINANCE:ETHUSDT");
    assert_eq!(eth["currency_mismatch"], true);
    assert_eq!(eth["unrealized_pnl"], Value::Null);
    let totals = out["totals"].as_array().unwrap();
    assert_eq!(totals.len(), 1, "{totals:?}");
    assert_eq!(totals[0]["currency"], "usd");
    assert_eq!(totals[0]["holdings"], 1);
    assert_eq!(totals[0]["value"], 192_751.5);
    // The bought-and-sold-out asset has no row but its profit still counts.
    assert!(holdings.iter().all(|h| h["asset"] != "BINANCE:OLDUSDT"));
    assert_eq!(out["closed_positions_realized_pnl"], 5.0);
    assert_eq!(
        out["unpriced"], 1,
        "ethereum has no quote from the recorded provider"
    );
    assert_eq!(out["portfolio"]["name"], "Main");
}

#[test]
fn prices_can_be_skipped_and_need_no_quote_feed() {
    let harness = Harness::start(&[("p", "tok", PORTFOLIO)]);
    seeded(&harness);
    let result = call(
        &harness.url(),
        "tok",
        "degenview_get_portfolio_holdings",
        json!({"include_prices": false}),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    let out = &result["structuredContent"];
    assert_eq!(
        out["portfolio"],
        Value::Null,
        "all unarchived portfolios together"
    );
    assert_eq!(out["holdings"][0]["value"], Value::Null);
    assert!(out["notes"].to_string().contains("not asked for"));
    // With prices wanted and no feed the call says so rather than returning blanks.
    let unfed = call(
        &harness.url(),
        "tok",
        "degenview_get_portfolio_holdings",
        json!({"wait_seconds": 0}),
    );
    assert_eq!(error_code(&unfed), "APP_UNAVAILABLE");
}

#[test]
fn transactions_page_newest_first_and_filter_by_asset() {
    let harness = Harness::start(&[("p", "tok", PORTFOLIO)]);
    let id = seeded(&harness).to_string();
    let url = harness.url();
    let page = call(
        &url,
        "tok",
        "degenview_list_portfolio_transactions",
        json!({"portfolio_id": id, "limit": 2}),
    );
    let out = &page["structuredContent"];
    assert_eq!(out["total"], 6);
    assert_eq!(out["truncated"], true);
    assert_eq!(out["transactions"][0]["asset"], "BINANCE:ETHUSDT");
    assert_eq!(out["transactions"][0]["price_currency"], "eur");
    assert_eq!(out["transactions"][1]["kind"], "sell");
    let btc = call(
        &url,
        "tok",
        "degenview_list_portfolio_transactions",
        json!({"asset": "binance:btcusdt"}),
    );
    assert_eq!(btc["structuredContent"]["total"], 3);
}

#[test]
fn unknown_portfolios_and_bad_arguments_are_refused() {
    let harness = Harness::start(&[("p", "tok", PORTFOLIO)]);
    seeded(&harness);
    let url = harness.url();
    for tool in [
        "degenview_get_portfolio_holdings",
        "degenview_list_portfolio_transactions",
    ] {
        let missing = call(&url, "tok", tool, json!({"portfolio_id": "424242"}));
        assert_eq!(error_code(&missing), "NOT_FOUND", "{tool}");
        assert!(
            rejected(&url, "tok", tool, json!({"portfolio_id": "abc"})),
            "{tool}"
        );
        assert!(
            rejected(&url, "tok", tool, json!({"portfolio_id": ""})),
            "{tool}"
        );
    }
    for extra in [
        json!({"limit": 0}),
        json!({"limit": 201}),
        json!({"asset": ""}),
    ] {
        assert!(
            rejected(
                &url,
                "tok",
                "degenview_list_portfolio_transactions",
                extra.clone()
            ),
            "{extra}"
        );
    }
    assert!(rejected(
        &url,
        "tok",
        "degenview_get_portfolio_holdings",
        json!({"wait_seconds": 16})
    ));
}
