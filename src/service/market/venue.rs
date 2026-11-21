//! What a market data venue does that needs I/O, found through one registry. The facts that need
//! none (names, id spelling, candle lengths, polling) are the venue's `model::market::profile`.
//!
//! A venue is one `service::market::<venue>` directory with an implementation of [`Venue`] in its
//! `mod.rs` and, where it has sockets, [`CandleProtocol`] in `live.rs` and [`QuoteProtocol`] in
//! `quotes.rs`. [`LiveFeed`](super::live::LiveFeed) and the quote engine stay the only loops: a
//! protocol only says how to open a socket and read what it sends.

use super::live::CandleProtocol;
use crate::model::market::MarketKind;
use crate::model::{DataSource, InstrumentId};
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult};

pub trait Venue: Sync {
    /// Candles, search and the market list over REST.
    fn candles(&self) -> &dyn MarketSource;

    /// The socket that streams the forming candle; `None` when the venue has none.
    fn live(&self) -> Option<&dyn CandleProtocol> {
        None
    }

    /// The shared ticker socket behind watchlist prices; `None` when the venue has none.
    fn quotes(&self) -> Option<&dyn QuoteProtocol> {
        None
    }

    /// Prices for `requests` over REST, one batched call where the venue has one. A venue whose
    /// markets are not priced here answers nothing.
    fn rest_quotes(&self, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
        let _ = requests;
        Ok(Outcome::default())
    }

    /// What an HTTP status other than 429 means for a quote request. `None` is the generic failure.
    /// Only a status the venue documents as "a symbol in this request is unknown" is
    /// [`QuoteFailure::Rejected`], which makes the engine search for the bad symbol; every other
    /// refusal (credentials, region, a changed endpoint, a ban) belongs to the whole venue.
    fn classify_status(&self, code: u16) -> Option<QuoteFailure> {
        let _ = code;
        None
    }

    /// How the market `id` names is traded. A venue with perpetuals tells them from its spot markets
    /// by the id; the others are what their profile says. The kind is not stored with a chart or a
    /// watchlist row: it is asked here whenever it is shown.
    fn kind_of(&self, id: &InstrumentId) -> MarketKind {
        id.source.profile().default_kind()
    }

    /// The network of a DEX pair saved without one, from its address.
    fn resolve_chain(&self, address: &str) -> Option<String> {
        let _ = address;
        None
    }

    /// Whether the venue can say which markets are busiest right now.
    fn supports_trending(&self) -> bool {
        false
    }

    /// The venue's busiest markets, or an empty list when it cannot say.
    fn trending(&self, limit: usize) -> Result<Vec<SearchResult>, ProviderError> {
        let _ = limit;
        Ok(Vec::new())
    }
}

/// The venue behind `source`. The one place that pairs a source with its code.
pub fn of(source: DataSource) -> &'static dyn Venue {
    match source {
        DataSource::Binance => &super::binance::BINANCE,
        DataSource::Coinbase => &super::coinbase::COINBASE,
        DataSource::Okx => &super::okx::OKX,
        DataSource::Kraken => &super::kraken::KRAKEN,
        DataSource::Bybit => &super::bybit::BYBIT,
        DataSource::Gate => &super::gateio::GATE,
        DataSource::Mexc => &super::mexc::MEXC,
        DataSource::CoinGecko => &super::coingecko::COINGECKO,
        DataSource::DexScreener => &super::dexscreener::DEXSCREENER,
        DataSource::Alpaca => &super::alpaca::ALPACA,
        DataSource::Polymarket => &super::polymarket::POLYMARKET,
        DataSource::Kalshi => &super::kalshi::KALSHI,
        DataSource::CoinMarketCap => &super::cmc::COIN_MARKET_CAP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_market_is_of_a_kind_its_venue_lists() {
        for source in DataSource::ALL {
            let kind = of(source).kind_of(&InstrumentId::new(source, "BTC"));
            assert!(
                source.profile().kinds.contains(&kind),
                "{source:?} answers {kind:?}, which its profile does not list"
            );
        }
    }

    #[test]
    fn the_sockets_a_venue_opens_are_the_ones_its_profile_declares() {
        for source in DataSource::ALL {
            let profile = source.profile();
            assert_eq!(
                of(source).live().is_some(),
                profile.streams.candles,
                "{source:?} candle stream"
            );
            assert_eq!(
                of(source).quotes().is_some(),
                profile.streams.quotes,
                "{source:?} quote stream"
            );
        }
    }
}
