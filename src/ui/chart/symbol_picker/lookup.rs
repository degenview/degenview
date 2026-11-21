//! What the search box asks for: the text, or a `VENUE:SYMBOL` read as a venue and a symbol.
//!
//! `BINANCE:BTCUSDT` and `NASDAQ:AAPL` (the watchlist import's own format,
//! [`crate::model::market::venue`]) search the symbol on that venue alone, whatever the pull-down
//! says. A prefix no provider serves (`KUCOIN:BTCUSDT`) is never dropped silently: the symbol is
//! searched on every venue of the tab, and the list says the prefix was not understood.

use super::search::Scope;
use crate::model::DataSource;
use crate::model::market::profile::Class;
use crate::model::market::venue::{self, VenueQuery};

/// A prefix this long or longer is a phrase with a colon in it, not a venue.
const MAX_PREFIX_CHARS: usize = 12;

/// The venue and the text one search asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lookup {
    /// What is searched for (or filtered by): the box's text, or the symbol after a prefix.
    pub query: String,
    /// The one venue searched: the prefix's, else the pull-down's; `None` is every venue of the tab.
    pub venue: Option<DataSource>,
    /// The prefix the text starts with when no provider here serves it.
    pub unsupported: Option<String>,
}

impl Lookup {
    /// Reads `text` as typed in `scope` with `chosen` in the pull-down. Only the tabs that match
    /// markets read a prefix; Dashboards filters titles as typed.
    pub fn of(text: &str, scope: Scope, chosen: Option<DataSource>) -> Self {
        let plain = Self {
            query: text.to_owned(),
            venue: chosen,
            unsupported: None,
        };
        if scope == Scope::Dashboards {
            return plain;
        }
        let Some(VenueQuery {
            prefix,
            source,
            symbol,
        }) = venue::parse(text)
        else {
            return plain;
        };
        match source {
            Some(source) if is_listed(source) => Self {
                query: symbol,
                venue: Some(source),
                unsupported: None,
            },
            // Prediction titles have colons ("Fed: cut"), and a watchlist is filtered, not searched.
            _ if !scope.searches() || scope == Scope::Predictions => plain,
            _ if !is_symbol(&symbol) || !looks_like_venue(&prefix) => plain,
            // The prefix overrides the pull-down here as it does for a served one: a venue filter
            // left over from before would hide the symbol's match on the venue that does list it.
            _ => Self {
                query: symbol,
                venue: None,
                unsupported: Some(prefix),
            },
        }
    }

    /// Whether the text has nothing to search for yet.
    pub fn is_empty(&self) -> bool {
        self.query.trim().is_empty()
    }

    /// The venue the prefix named when `scope` has no markets of it (Binance on the Stocks tab).
    pub fn outside(&self, scope: Scope) -> Option<DataSource> {
        self.venue.filter(|venue| !scope.includes(*venue))
    }

    /// The venues one search of `scope` asks, as the no-results card lists them.
    pub fn searched(&self, scope: Scope) -> Vec<DataSource> {
        scope
            .venues()
            .into_iter()
            .filter(|venue| self.venue.is_none_or(|only| only == *venue))
            .collect()
    }
}

/// Whether the picker lists markets of `source` (CoinMarketCap's indices are a dashboard).
fn is_listed(source: DataSource) -> bool {
    source.profile().class != Class::Index
}

/// A symbol is one word; a phrase after a colon ("Fed: rate cut") is a search for text.
fn is_symbol(symbol: &str) -> bool {
    !symbol.is_empty() && !symbol.contains(char::is_whitespace)
}

/// A venue's name is one short word: letters and digits, like `KUCOIN` or `HKEX`.
fn looks_like_venue(prefix: &str) -> bool {
    (2..=MAX_PREFIX_CHARS).contains(&prefix.chars().count())
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// How a venue is named inside a sentence: "Alpaca (IEX)" is Alpaca there.
pub fn short_name(source: DataSource) -> &'static str {
    source.display_name().split(" (").next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(text: &str, scope: Scope) -> Lookup {
        Lookup::of(text, scope, None)
    }

    #[test]
    fn plain_text_is_searched_as_typed() {
        let lookup = Lookup::of("btc", Scope::All, Some(DataSource::Coinbase));
        assert_eq!(lookup.query, "btc");
        assert_eq!(lookup.venue, Some(DataSource::Coinbase));
        assert_eq!(lookup.unsupported, None);
    }

    #[test]
    fn a_served_prefix_searches_the_symbol_on_that_venue_over_the_pull_down() {
        let lookup = Lookup::of(" binance : BTCUSDT", Scope::All, Some(DataSource::Coinbase));
        assert_eq!(lookup.query, "BTCUSDT");
        assert_eq!(lookup.venue, Some(DataSource::Binance));
        assert_eq!(lookup.unsupported, None);
        let stock = of("NASDAQ:AAPL", Scope::Stocks);
        assert_eq!(
            (stock.query.as_str(), stock.venue),
            ("AAPL", Some(DataSource::Alpaca))
        );
    }

    #[test]
    fn a_dex_market_keeps_its_own_colon_after_the_venue_prefix() {
        for text in ["HYPERLIQUID:xyz:SP500", "hyperliquid : xyz:SP500"] {
            let lookup = of(text, Scope::All);
            assert_eq!(lookup.query, "xyz:SP500", "{text}");
            assert_eq!(lookup.venue, Some(DataSource::Hyperliquid), "{text}");
            assert_eq!(lookup.unsupported, None, "{text}");
        }
    }

    #[test]
    fn a_prefix_alone_narrows_the_venue_and_waits_for_a_symbol() {
        let lookup = of("COINBASE:", Scope::All);
        assert!(lookup.is_empty());
        assert_eq!(lookup.venue, Some(DataSource::Coinbase));
    }

    #[test]
    fn a_venue_nothing_serves_is_named_and_searched_everywhere() {
        for prefix in ["kucoin", "BITGET", "HKEX", "SSE", "CME"] {
            let lookup = of(&format!("{prefix}:BTCUSDT"), Scope::All);
            assert_eq!(lookup.unsupported, Some(prefix.to_uppercase()));
            assert_eq!(lookup.query, "BTCUSDT");
            assert_eq!(lookup.venue, None);
        }
        // A venue picked in the pull-down does not narrow it: Binance's BTCUSDT must still show.
        let chosen = Lookup::of("KUCOIN:BTCUSDT", Scope::All, Some(DataSource::Coinbase));
        assert_eq!(chosen.unsupported, Some("KUCOIN".to_owned()));
        assert_eq!(chosen.venue, None);
        assert_eq!(chosen.searched(Scope::All).len(), Scope::All.venues().len());
        // CoinMarketCap has indices, not markets to search.
        assert_eq!(
            of("CMC:BTC.D", Scope::All).unsupported,
            Some("CMC".to_owned())
        );
    }

    #[test]
    fn a_colon_in_a_phrase_is_not_a_venue() {
        for text in [
            "KUCOIN:",
            "Fed: rate cut",
            "a:b",
            "really long name:x",
            "bad-venue:x",
        ] {
            let lookup = of(text, Scope::All);
            assert_eq!(lookup.unsupported, None, "{text}");
            assert_eq!(lookup.query, text, "{text}");
        }
    }

    #[test]
    fn predictions_and_watchlists_read_only_a_prefix_they_know() {
        let fed = of("Fed: cut", Scope::Predictions);
        assert_eq!((fed.query.as_str(), fed.unsupported), ("Fed: cut", None));
        let okx = of("KUCOIN:BTC", Scope::Watchlists);
        assert_eq!((okx.query.as_str(), okx.unsupported), ("KUCOIN:BTC", None));
        let known = of("kalshi:fed", Scope::Predictions);
        assert_eq!(
            (known.query.as_str(), known.venue),
            ("fed", Some(DataSource::Kalshi))
        );
        let listed = of("binance:btc", Scope::Watchlists);
        assert_eq!(
            (listed.query.as_str(), listed.venue),
            ("btc", Some(DataSource::Binance))
        );
    }

    #[test]
    fn dashboards_filter_titles_as_typed() {
        let lookup = of("binance:btc", Scope::Dashboards);
        assert_eq!(lookup.query, "binance:btc");
        assert_eq!(lookup.venue, None);
    }

    #[test]
    fn a_venue_the_tab_does_not_list_is_reported() {
        assert_eq!(
            of("BINANCE:BTC", Scope::Stocks).outside(Scope::Stocks),
            Some(DataSource::Binance)
        );
        assert_eq!(
            of("BINANCE:BTC", Scope::Crypto).outside(Scope::Crypto),
            None
        );
        assert_eq!(
            of("POLYMARKET:x", Scope::All).outside(Scope::All),
            Some(DataSource::Polymarket)
        );
    }

    #[test]
    fn the_searched_venues_are_the_tabs_or_the_one_chosen() {
        let names = |lookup: &Lookup, scope| -> Vec<&'static str> {
            lookup.searched(scope).into_iter().map(short_name).collect()
        };
        assert_eq!(
            names(&of("btc", Scope::All), Scope::All),
            [
                "Binance",
                "Coinbase",
                "OKX",
                "Kraken",
                "Bybit",
                "Gate.io",
                "MEXC",
                "Hyperliquid",
                "Aster",
                "Lighter",
                "Deribit",
                "HTX",
                "Bitfinex",
                "Bitstamp",
                "BingX",
                "DEXScreener",
                "CoinGecko",
                "Alpaca"
            ]
        );
        assert_eq!(
            names(&of("coinbase:btc", Scope::All), Scope::All),
            ["Coinbase"]
        );
        assert_eq!(names(&of("btc", Scope::Stocks), Scope::Stocks), ["Alpaca"]);
    }
}
