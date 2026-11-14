use super::*;
use crate::model::DataSource;
use crate::model::portfolio::PortfolioCurrency;
use crate::model::portfolio::statistics::PortfolioHistoryRange;
use crate::model::portfolio::{PortfolioAsset, PortfolioHolding};
use std::collections::HashMap;

fn holding(ticker: &str, value: Option<f64>) -> PortfolioHolding {
    PortfolioHolding {
        asset: PortfolioAsset {
            key: format!("binance:{ticker}"),
            symbol: format!("{ticker}/USDT"),
            name: ticker.to_owned(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        },
        quantity: 1.0,
        cost_basis: 1.0,
        average_cost: 1.0,
        realized_pnl: 0.0,
        current_price: value,
        previous_day_price: None,
        current_value: value,
        unrealized_pnl: None,
        allocation: 0.0,
        is_price_stale: false,
    }
}

#[test]
fn top_holdings_are_the_six_largest_with_unpriced_last() {
    let holdings: Vec<PortfolioHolding> = (1..=8)
        .map(|n| holding(&format!("A{n}"), Some(f64::from(n) * 10.0)))
        .chain([holding("NONE", None)])
        .collect();
    let top = top_holdings(&holdings);
    let tickers: Vec<String> = top.iter().map(|h| h.asset.display_ticker()).collect();
    assert_eq!(tickers, ["A8", "A7", "A6", "A5", "A4", "A3"]);
    let few = vec![holding("X", None), holding("Y", Some(1.0))];
    let ranked: Vec<String> = top_holdings(&few)
        .iter()
        .map(|h| h.asset.display_ticker())
        .collect();
    assert_eq!(ranked, ["Y", "X"]);
}

#[test]
fn asset_counts_pluralise() {
    assert_eq!(asset_count_text(0), "0 assets");
    assert_eq!(asset_count_text(1), "1 asset");
    assert_eq!(asset_count_text(12), "12 assets");
}

#[test]
fn the_default_range_is_a_month() {
    assert_eq!(
        OverviewState::default().range,
        PortfolioHistoryRange::OneMonth
    );
}
