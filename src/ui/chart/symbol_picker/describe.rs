//! What the picker says about a market in words: the Description column, the pair under the
//! symbol and the line under the preview's ticker. Also loads the names those need.

use crate::model::DataSource;
use crate::model::market::asset_name::AssetNames;
use crate::service::SearchResult;
use crate::service::market::coin_index::CoinIndex;
use crate::service::market::names::{Directory, Listed};
use crate::service::work_queue::{self, Pending};
use eframe::egui;
use std::sync::Arc;

use super::search::Backend;

/// The names directory and the two loads that fill it. Both are only read once per picker; until
/// they answer, markets whose provider lists no names have none.
#[derive(Default)]
pub struct Names {
    pub directory: Directory,
    coins: Option<Pending<Arc<CoinIndex>>>,
    listed: Option<Pending<Arc<Listed>>>,
    asked: bool,
}

impl Names {
    /// The same names for a freshly opened picker, which asks again so a failed load is retried.
    pub fn reopened(self) -> Self {
        Self {
            directory: self.directory,
            ..Self::default()
        }
    }

    /// Starts the loads on the first call and takes in what has answered.
    pub fn poll(&mut self, ctx: &egui::Context, backend: Backend) {
        if !self.asked {
            self.asked = true;
            let coins = backend.coin_names;
            self.coins = Some(work_queue::start_for(
                DataSource::CoinGecko,
                Some(ctx),
                coins,
            ));
            let listed = backend.listed_names;
            self.listed = Some(work_queue::start(Some(ctx), listed));
        }
        if let Some(coins) = self.coins.as_mut().and_then(Pending::poll)
            && !coins.is_empty()
        {
            self.directory.coins = coins;
        }
        if let Some(listed) = self.listed.as_mut().and_then(Pending::poll)
            && !listed.is_empty()
        {
            self.directory.listed = listed;
        }
    }

    pub fn of(&self, result: &SearchResult) -> AssetNames {
        self.directory.names(result)
    }
}

/// The Description column: what the market trades. A pool or a forex pair is named on both
/// sides, since neither side is a given; a prediction market says which event it belongs to.
pub fn column(result: &SearchResult, names: &AssetNames) -> String {
    if result.source.is_prediction_market() {
        return event_title(result).to_owned();
    }
    match result.source {
        DataSource::DexScreener | DataSource::Oanda => both(names),
        _ => names.base.clone().unwrap_or_default(),
    }
}

/// The line under the symbol: the pair (`BTC / USDT`) of a market that is one.
pub fn pair_line(result: &SearchResult) -> String {
    if result.source.is_prediction_market() {
        return event_title(result).to_owned();
    }
    if matches!(
        result.source,
        DataSource::Alpaca | DataSource::CoinGecko | DataSource::CoinMarketCap
    ) {
        return String::new();
    }
    let quote = result
        .quote
        .clone()
        .or_else(|| result.source.quote_asset(&result.symbol));
    quote.map_or_else(String::new, |quote| {
        format!("{} / {quote}", result.base_asset())
    })
}

/// The line under the preview's ticker: `Bitcoin / Tether`, or the company or coin alone; the
/// pair while no name is known.
pub fn headline(result: &SearchResult, names: &AssetNames) -> String {
    let named = match result.source {
        DataSource::Polymarket | DataSource::Kalshi => String::new(),
        _ => both(names),
    };
    if named.is_empty() {
        pair_line(result)
    } else {
        named
    }
}

/// `Bitcoin / Tether`, or the base's name alone when only it is known.
fn both(names: &AssetNames) -> String {
    match (&names.base, &names.quote) {
        (Some(base), Some(quote)) => format!("{base} / {quote}"),
        (Some(base), None) => base.clone(),
        _ => String::new(),
    }
}

/// The event a prediction choice belongs to, when the choice is not the event itself.
fn event_title(result: &SearchResult) -> &str {
    result
        .event
        .as_ref()
        .map(|event| event.title.as_str())
        .filter(|title| !title.is_empty() && *title != result.symbol)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(base: Option<&str>, quote: Option<&str>) -> AssetNames {
        AssetNames::new(base, quote)
    }

    #[test]
    fn a_row_says_what_it_trades_and_in_which_pair() {
        let pair = SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT");
        let named = names(Some("Bitcoin"), Some("Tether"));
        assert_eq!(column(&pair, &named), "Bitcoin");
        assert_eq!(pair_line(&pair), "BTC / USDT");
        assert_eq!(headline(&pair, &named), "Bitcoin / Tether");
        assert_eq!(headline(&pair, &AssetNames::default()), "BTC / USDT");

        let pool =
            SearchResult::new(DataSource::DexScreener, "BTC/USDC", "0x1").pair("BTC", "USDC");
        assert_eq!(
            column(&pool, &names(Some("Bitcoin"), Some("USD Coin"))),
            "Bitcoin / USD Coin"
        );

        let stock = SearchResult::new(DataSource::Alpaca, "AAPL — Apple Inc.", "AAPL");
        assert_eq!(
            column(&stock, &names(Some("Apple Inc."), None)),
            "Apple Inc."
        );
        assert_eq!(pair_line(&stock), "");

        // A pick remembered before pairs were stored still reads its pair from the symbol.
        let old = SearchResult::new(DataSource::Coinbase, "BTC-USD", "BTC-USD");
        assert_eq!(pair_line(&old), "BTC / USD");
    }
}
