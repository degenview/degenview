//! Finds the DegenView market for a TradingView symbol: the same market on its own exchange, or
//! the same pair on another. Only an exact name counts; nothing that merely looks alike is taken.

use crate::model::DataSource;
use crate::model::tradingview::TvSymbol;
use crate::model::tradingview::plan::{Market, Match, Unmatched};
use crate::model::tradingview::symbol;
use crate::service::{ProviderError, SearchResult};
use std::collections::VecDeque;

pub fn market_of(result: &SearchResult) -> Market {
    Market {
        source: result.source,
        symbol: result.full_symbol.clone(),
        name: result.symbol.clone(),
        image_url: result
            .event
            .as_ref()
            .and_then(|event| event.image_url.clone()),
    }
}

/// A provider's ticker search.
pub type Search =
    dyn Fn(DataSource, &str) -> Result<Vec<SearchResult>, ProviderError> + Send + Sync;

/// A provider to ask for a name.
struct Request {
    source: DataSource,
    name: String,
    /// The symbol's own exchange, which nothing stands in for while it does not answer.
    own: bool,
}

/// What a [`Resolution`] needs next.
#[derive(Debug, PartialEq)]
pub enum Step<'a> {
    /// A ticker search of `source` for `name`, answered with [`Resolution::answer`].
    Ask(DataSource, &'a str),
    Done(&'a Match),
}

/// The search for one TradingView symbol's market, a request at a time, so that each can be made
/// on its provider's own workers.
pub struct Resolution {
    requests: VecDeque<Request>,
    asked: usize,
    failed: usize,
    /// TradingView's exchange is one DegenView has no provider for.
    venue_unserved: bool,
    found: Option<Match>,
}

impl Resolution {
    pub fn new(tv: &TvSymbol) -> Self {
        let mut resolution = Self {
            requests: VecDeque::new(),
            asked: 0,
            failed: 0,
            venue_unserved: tv.source().is_none(),
            found: None,
        };
        if tv.is_expression() {
            resolution.found = Some(Match::Unmatched(Unmatched::Expression));
            return resolution;
        }
        let own = symbol::native_name(tv);
        if tv.is_derivative() && own.is_none() {
            resolution.found = Some(Match::Unmatched(Unmatched::Derivative));
            return resolution;
        }
        let own_source = own.as_ref().map(|(source, _)| *source);
        resolution
            .requests
            .extend(own.map(|(source, name)| Request {
                source,
                name,
                own: true,
            }));
        // A stock's ticker is not a pair, whatever it ends in, and no other exchange's spot pair
        // stands in for a derivative.
        let pair = tv
            .pair()
            .filter(|_| tv.source() != Some(DataSource::Alpaca) && !tv.is_derivative());
        if let Some((base, quote)) = pair {
            for source in symbol::substitutes(quote) {
                if own_source == Some(source) {
                    continue;
                }
                if let Some(name) = symbol::pair_on(source, base, quote) {
                    resolution.requests.push_back(Request {
                        source,
                        name,
                        own: false,
                    });
                }
            }
        }
        resolution.settle_if_exhausted();
        resolution
    }

    pub fn step(&self) -> Step<'_> {
        match (&self.found, self.requests.front()) {
            (Some(found), _) => Step::Done(found),
            (None, Some(request)) => Step::Ask(request.source, &request.name),
            // `settle_if_exhausted` leaves no resolution without either.
            (None, None) => Step::Done(&Match::Unmatched(Unmatched::NotFound)),
        }
    }

    /// Takes in what the request [`Self::step`] asked for came back with.
    pub fn answer(&mut self, results: Result<Vec<SearchResult>, ProviderError>) {
        if self.found.is_some() {
            return;
        }
        let Some(request) = self.requests.pop_front() else {
            return;
        };
        self.asked += 1;
        match results {
            Ok(results) => {
                let market = results
                    .iter()
                    .find(|result| result.full_symbol.eq_ignore_ascii_case(&request.name))
                    .map(market_of);
                self.found = market.map(|market| {
                    if request.own {
                        Match::Exact(market)
                    } else {
                        Match::Substitute(market)
                    }
                });
            }
            Err(ProviderError::MissingKey(DataSource::Alpaca)) if request.own => {
                self.found = Some(Match::Unmatched(Unmatched::NeedsKey));
            }
            // Not knowing is not a reason to take another exchange's market: search again.
            Err(_) if request.own => self.found = Some(Match::Unmatched(Unmatched::Unreachable)),
            Err(_) => self.failed += 1,
        }
        self.settle_if_exhausted();
    }

    /// With nothing left to ask and nothing found, says why.
    fn settle_if_exhausted(&mut self) {
        if self.found.is_none() && self.requests.is_empty() {
            self.found = Some(Match::Unmatched(
                if self.asked > 0 && self.failed == self.asked {
                    Unmatched::Unreachable
                } else if self.venue_unserved {
                    Unmatched::Venue
                } else {
                    Unmatched::NotFound
                },
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole resolution, its requests answered one after another.
    fn resolve(tv: &TvSymbol, search: &Search) -> Match {
        let mut resolution = Resolution::new(tv);
        loop {
            let results = match resolution.step() {
                Step::Done(found) => return found.clone(),
                Step::Ask(source, name) => search(source, name),
            };
            resolution.answer(results);
        }
    }

    fn hit(source: DataSource, name: &str) -> SearchResult {
        SearchResult::new(source, name, name)
    }

    /// A catalog of what each provider lists.
    fn catalog(source: DataSource, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let listed: &[&str] = match source {
            DataSource::Binance => &["BTCUSDT", "BTCUSDTX", "ETHUSDT.P"],
            DataSource::Coinbase => &["ETH-USD"],
            _ => &[],
        };
        Ok(listed
            .iter()
            .filter(|name| name.starts_with(query))
            .map(|name| hit(source, name))
            .collect())
    }

    fn resolve_text(text: &str) -> Match {
        resolve(&TvSymbol::read(text).unwrap(), &catalog)
    }

    #[test]
    fn a_binance_perpetual_matches_itself_and_nothing_stands_in_for_one() {
        let Match::Exact(market) = resolve_text("BINANCE:ETHUSDT.P") else {
            panic!("not matched");
        };
        assert_eq!(market.symbol, "ETHUSDT.P");
        // One Binance does not list is not found, not replaced by a spot pair.
        assert_eq!(
            resolve_text("BINANCE:BTCUSDT.P"),
            Match::Unmatched(Unmatched::NotFound)
        );
    }

    #[test]
    fn the_symbols_own_exchange_wins() {
        let Match::Exact(market) = resolve_text("BINANCE:BTCUSDT") else {
            panic!();
        };
        assert_eq!(market.symbol, "BTCUSDT");
    }

    #[test]
    fn an_unserved_exchange_gets_the_same_pair_elsewhere() {
        let Match::Substitute(market) = resolve_text("BITSTAMP:ETHUSD") else {
            panic!();
        };
        assert_eq!(
            (market.source, market.symbol.as_str()),
            (DataSource::Coinbase, "ETH-USD")
        );
    }

    #[test]
    fn a_different_quote_is_never_substituted() {
        assert_eq!(
            resolve_text("BITSTAMP:ETHEUR"),
            Match::Unmatched(Unmatched::Venue)
        );
        assert_eq!(
            resolve_text("BINANCE:BTCUSDC"),
            Match::Unmatched(Unmatched::NotFound)
        );
        assert_eq!(
            resolve_text("BYBIT:BTCUSDT.P"),
            Match::Unmatched(Unmatched::Derivative)
        );
    }

    #[test]
    fn an_exchange_that_does_not_answer_is_not_replaced() {
        let binance_down = |source: DataSource, query: &str| match source {
            DataSource::Binance => Err(ProviderError::Other("timeout".into())),
            _ => Ok(vec![hit(source, query)]),
        };
        assert_eq!(
            resolve(&TvSymbol::read("BINANCE:BTCUSDT").unwrap(), &binance_down),
            Match::Unmatched(Unmatched::Unreachable)
        );
    }

    #[test]
    fn a_missing_key_and_a_dead_network_are_said() {
        let no_key = |source: DataSource, _: &str| Err(ProviderError::MissingKey(source));
        assert_eq!(
            resolve(&TvSymbol::read("NASDAQ:AAPL").unwrap(), &no_key),
            Match::Unmatched(Unmatched::NeedsKey)
        );
        let down = |_: DataSource, _: &str| Err(ProviderError::Other("timeout".into()));
        assert_eq!(
            resolve(&TvSymbol::read("BINANCE:BTCUSDT").unwrap(), &down),
            Match::Unmatched(Unmatched::Unreachable)
        );
    }
}
