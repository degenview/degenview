//! How an imported file's provider text becomes a market DegenView can chart. The journal crate
//! knows no provider; this is the application's list (`DataSource`) handed to it for imports.

use degenview_journal::csv::MarketCatalog;

use crate::model::{DataSource, InstrumentId};

/// The application's chart providers, by display name or watchlist slug.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppMarkets;

fn normalised(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl MarketCatalog for AppMarkets {
    fn provider(&self, text: &str) -> Option<String> {
        let wanted = normalised(text);
        DataSource::ALL
            .into_iter()
            .find(|source| {
                normalised(source.display_name()) == wanted
                    || normalised(source.watchlist_slug()) == wanted
                    // "Alpaca" for "Alpaca (IEX)".
                    || normalised(source.display_name()).starts_with(&wanted) && wanted.len() >= 4
            })
            .map(|source| source.display_name().to_owned())
    }

    fn market_key(&self, provider: &str, symbol: &str) -> String {
        match DataSource::from_display_name(provider) {
            Some(source) => InstrumentId::new(source, symbol).key(),
            None => format!("IMPORT:{}", symbol.to_uppercase()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_is_found_by_its_usual_names_and_keyed_like_the_charts() {
        assert_eq!(AppMarkets.provider("binance").as_deref(), Some("Binance"));
        assert_eq!(
            AppMarkets.provider(" COINBASE ").as_deref(),
            Some("Coinbase")
        );
        assert_eq!(AppMarkets.provider("nowhere"), None);
        assert_eq!(
            AppMarkets.market_key("Binance", "BTCUSDT"),
            InstrumentId::new(DataSource::Binance, "BTCUSDT").key()
        );
    }
}
