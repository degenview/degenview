//! What is known about a market data source without asking it: its names, how its market ids are
//! spelled, which candle lengths it serves, how its watchlist prices are fetched and how often.
//!
//! One [`Profile`] per [`DataSource`], all in [`table`]. Everything that used to be a `match` on the
//! source across the app reads the profile instead, so a new venue is one table entry (plus its
//! `service::market::<venue>` directory) and no other file names it. What needs I/O (fetching,
//! sockets) is not here: it is the venue's behaviour in `service::market::venue`.

mod table;

use super::source::DataSource;
use crate::model::Timeframe;
use crate::model::market::MarketKind;

pub use table::{ALL, profile_of};

/// The facts about one source.
#[derive(Debug)]
pub struct Profile {
    /// The venue's stable name; portfolio and paper keys are built from it.
    pub display_name: &'static str,
    /// Upper-case provider prefix in the import/export text format (`BINANCE:BTCUSDT`).
    pub slug: &'static str,
    /// The provider's name as every MCP tool spells it.
    pub mcp_id: &'static str,
    pub class: Class,
    /// Whether an id can say nothing to a reader (a DEX pair's address, a spot pair Hyperliquid
    /// numbers `@107`), so a chart added from search is titled by the hit's own label.
    pub opaque_ids: bool,
    /// How its markets are traded, the first being what an id means unless the venue says
    /// otherwise (`service::market::venue::Venue::kind_of`). A venue with perpetuals and spot lists both.
    pub kinds: &'static [MarketKind],
    pub series: Series,
    /// Whether the source reports per-candle turnover for the volume bars to draw.
    pub volume: bool,
    /// How the source's own spelling of a market is shown.
    pub display: Casing,
    /// How an id that is not a pair compares: the spelling the source treats as one market.
    pub id: Casing,
    pub pair: PairStyle,
    /// The quote a bare asset is priced in (`BTC` is `BTCUSDT` on Binance, `BTC-USD` on Coinbase),
    /// and the quote the search ranks first.
    pub home_quote: Option<&'static str>,
    /// The candle lengths the source serves itself, shortest first. Empty for a source that has no
    /// candles to ask for.
    pub natives: &'static [Timeframe],
    pub older_history: OlderHistory,
    /// Whether a chart of the source asks for its newest candles every 15 seconds instead of every
    /// 5: its REST allowance is small, or a socket keeps the chart current between the refreshes.
    pub slow_refresh: bool,
    pub streams: Streams,
    pub change: ChangeBasis,
    pub quotes: QuoteTiming,
    /// How candles are cached; `None` where they are not held.
    pub cache: Option<CachePolicy>,
}

impl Profile {
    /// What a market of this source is when the venue has no more to say: the first of its kinds.
    pub fn default_kind(&self) -> MarketKind {
        self.kinds.first().copied().unwrap_or_default()
    }
}

/// What kind of market a source lists. Decides where the symbol picker offers it, whose artwork a
/// row draws and how an import line is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Class {
    /// A spot exchange with its own pairs.
    Exchange,
    /// A coin aggregator: coins by id, not pairs.
    Aggregator,
    /// A DEX pair, identified by its address.
    Dex,
    Stock,
    Prediction,
    /// Index charts that are not markets (CoinMarketCap's gauges).
    Index,
}

impl Class {
    /// Whether a market of this class is found by symbol and charted with candles: everything but
    /// the prediction markets and the index gauges.
    pub const fn is_symbol_market(self) -> bool {
        !matches!(self, Self::Prediction | Self::Index)
    }

    /// Whether the picker puts the class under Crypto.
    pub const fn is_crypto(self) -> bool {
        matches!(self, Self::Exchange | Self::Aggregator | Self::Dex)
    }

    /// Where the symbol picker lists the class among the markets found by symbol.
    pub const fn picker_rank(self) -> u8 {
        match self {
            Self::Exchange => 0,
            Self::Dex => 1,
            Self::Aggregator => 2,
            Self::Stock => 3,
            Self::Prediction => 4,
            Self::Index => 5,
        }
    }

    /// Whether the market's artwork is its coin's, keyed by base asset.
    pub const fn has_coin_artwork(self) -> bool {
        matches!(self, Self::Exchange | Self::Aggregator)
    }
}

/// What the source's data is, and so how a timeframe is served.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Series {
    /// Candles at the lengths in [`Profile::natives`], folded to the others.
    Candles,
    /// Price samples (CoinGecko), made into candles of any length they are fine enough for.
    Samples,
    /// A line that follows any timeframe: probabilities and index values.
    Line,
}

/// How a spelling is cased.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Casing {
    Upper,
    Lower,
    AsGiven,
}

impl Casing {
    /// `text` in this casing.
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Upper => text.to_uppercase(),
            Self::Lower => text.to_lowercase(),
            Self::AsGiven => text.to_owned(),
        }
    }
}

/// How the source's market ids join two assets.
#[derive(Debug)]
pub enum PairStyle {
    /// Not a pair: a stock ticker, a coin id, a DEX address, a prediction-market id.
    None,
    /// The assets run together (`BTCUSDT`) and the quote is found by its suffix.
    Concat(Concat),
    /// A separator between the assets (`BTC-USD`, `BTC_USDT`, `BTC/USD`).
    Separated(Separated),
    /// The venue's own names, kept exactly as it lists them, case and all (`BTC`, `kPEPE`,
    /// `xyz:SP500`, `PURR/USDC`, `@107`): no quote is guessed or added to a bare asset.
    Listed(Listed),
}

/// A pair written without a separator.
#[derive(Debug)]
pub struct Concat {
    /// Pairs quoted in an asset `quotes` does not hold, as `(base, quote)` (Aster's `BTCU` is BTC
    /// quoted in `U`). Such an id is read whole, so it is never taken for a bare asset that gets
    /// the home quote (`BTCUUSDT`), and `quotes` stays free of a quote that any asset ending in
    /// its letters would match.
    pub exceptions: &'static [(&'static str, &'static str)],
    /// Quote assets a pair ends in, longest first so `BTCFDUSD` is BTC/FDUSD, not BTCFD/USD.
    pub quotes: &'static [&'static str],
    /// How many of the leading quotes take a one-letter base (`SUSDT`); a one-letter base before
    /// any other quote is a wrapped asset (`WBTC` is not W/BTC).
    pub one_letter_base_quotes: usize,
    /// Whether the asset of a pair never is a one-letter base, even before the quotes above.
    pub wrapped_asset_guard: bool,
    /// Characters dropped from a typed id before it is read (`BTC/USDT`).
    pub dropped: &'static [char],
}

impl Concat {
    /// Splits an upper-case id like `SOLBNB` into `("SOL", "BNB")`, or `None` when no quote ends it
    /// (a bare asset such as `SOL`).
    pub fn split<'a>(&self, upper: &'a str) -> Option<(&'a str, &'static str)> {
        self.quotes.iter().enumerate().find_map(|(index, quote)| {
            upper
                .strip_suffix(quote)
                .filter(|base| base.len() > usize::from(index >= self.one_letter_base_quotes))
                .map(|base| (base, *quote))
        })
    }

    /// [`Self::split`] as the asset of a pair reads it: a wrapped asset is not split.
    fn split_asset<'a>(&self, upper: &'a str) -> Option<(&'a str, &'static str)> {
        self.split_known(upper)
            .filter(|(base, _)| !self.wrapped_asset_guard || base.len() > 1)
    }

    /// [`Self::split`], knowing `exceptions` first: `BTCU` is BTC over `U`.
    fn split_known<'a>(&self, upper: &'a str) -> Option<(&'a str, &'static str)> {
        self.exceptions
            .iter()
            .find_map(|(base, quote)| {
                let held = upper.strip_suffix(quote)?;
                (held == *base).then_some((held, *quote))
            })
            .or_else(|| self.split(upper))
    }
}

/// A pair written with a separator.
#[derive(Debug)]
pub struct Separated {
    pub separator: char,
    /// What the source calls its markets, for the message that says how to write one.
    pub noun: &'static str,
    /// Other characters a typed id may use for the separator.
    pub also_typed_as: &'static [char],
    /// The source's own names for assets that its APIs refuse under the common ones, as
    /// `(source's, common)`; either way is read, and the common one is written.
    pub aliases: &'static [(&'static str, &'static str)],
}

impl Separated {
    fn alias<'a>(&self, asset: &'a str) -> &'a str {
        self.aliases
            .iter()
            .find(|(own, _)| own.eq_ignore_ascii_case(asset))
            .map_or(asset, |(_, common)| common)
    }
}

/// Names a venue lists as they are.
#[derive(Debug)]
pub struct Listed {
    /// Joins the namespace of a market to its name (`xyz:SP500` is `SP500` on the dex `xyz`).
    pub namespace: char,
    /// Characters besides letters and digits that a name may hold, for the shape check of an
    /// imported line.
    pub also_in_names: &'static [char],
    /// What the venue calls its markets, for the message that says how to write one.
    pub noun: &'static str,
    /// A few names as the venue writes them, for the same message.
    pub examples: &'static str,
}

impl Listed {
    /// The asset of an upper-case id: the name without its namespace.
    fn asset<'a>(&self, head: &'a str) -> &'a str {
        head.rsplit_once(self.namespace)
            .map_or(head, |(_, asset)| asset)
    }

    /// Whether `name` is shaped like one of the venue's names: letters, digits and the characters
    /// it allows, under a length no listed name comes near.
    pub fn is_name(&self, name: &str) -> bool {
        (1..=MAX_LISTED_NAME).contains(&name.chars().count())
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || self.also_in_names.contains(&ch))
    }
}

/// The longest name [`Listed::is_name`] accepts.
const MAX_LISTED_NAME: usize = 40;

impl PairStyle {
    /// The asset of a market, from the first part of its upper-case id.
    pub(super) fn base_of<'a>(&self, head: &'a str) -> &'a str {
        match self {
            Self::Concat(concat) => concat.split_asset(head).map_or(head, |(base, _)| base),
            Self::Listed(listed) => listed.asset(head),
            Self::Separated(_) | Self::None => head,
        }
    }

    /// The quote of an upper-case id the exchange itself spells out; never guessed.
    pub(super) fn quote_of(&self, upper: &str) -> Option<String> {
        match self {
            Self::Concat(concat) => concat.split_known(upper).map(|(_, quote)| quote.to_owned()),
            Self::Separated(Separated { separator, .. }) => {
                let (base, quote) = upper.split_once(*separator)?;
                (!base.is_empty()
                    && !quote.is_empty()
                    && !quote.contains(['-', '/', ' ', *separator]))
                .then(|| quote.to_owned())
            }
            Self::Listed(_) | Self::None => None,
        }
    }

    /// The id the source's API takes for `symbol`, a bare asset getting `home_quote`. A name the
    /// venue lists takes the casing of `id` (the profile's, which compares ids); everything else is
    /// returned as given.
    pub(super) fn api_symbol(&self, symbol: &str, home_quote: Option<&str>, id: Casing) -> String {
        let home = home_quote.unwrap_or_default();
        match self {
            Self::Concat(concat) => {
                let mut upper = symbol.to_uppercase();
                upper.retain(|ch| !concat.dropped.contains(&ch));
                if concat.split_asset(&upper).is_some() {
                    upper
                } else {
                    format!("{upper}{home}")
                }
            }
            Self::Separated(style) => {
                let separator = style.separator;
                let id: String = symbol
                    .to_uppercase()
                    .chars()
                    .map(|ch| {
                        if style.also_typed_as.contains(&ch) {
                            separator
                        } else {
                            ch
                        }
                    })
                    .collect();
                match id.split_once(separator) {
                    Some((base, quote)) => {
                        format!("{}{separator}{}", style.alias(base), style.alias(quote))
                    }
                    None => format!("{}{separator}{home}", style.alias(&id)),
                }
            }
            Self::Listed(_) => id.apply(symbol),
            Self::None => symbol.to_owned(),
        }
    }
}

/// How a quote's change is measured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeBasis {
    Rolling24h,
    PreviousClose,
    Window,
}

impl ChangeBasis {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rolling24h => "Change over the last 24 hours",
            Self::PreviousClose => "Change since the previous close",
            Self::Window => "Change over the market's comparison window",
        }
    }
}

/// Whether a source's older candles can be paged in, which scrolling back past what a refresh
/// brings needs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OlderHistory {
    Never,
    Always,
    /// Only a short way back (GeckoTerminal reaches `days`): a page of `page` base candles that
    /// already spans that has no older one.
    Within {
        page: i64,
        days: i64,
    },
}

/// Which of the source's data arrives over a socket instead of waiting for a refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Streams {
    /// The forming candle of a chart.
    pub candles: bool,
    /// Watchlist prices, from a shared ticker socket with REST filling in.
    pub quotes: bool,
}

/// How watchlist prices of a source are polled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuoteTiming {
    /// Seconds between refreshes while a sidebar is visible.
    pub refresh_secs: u64,
    /// How long after its last refresh a price is still shown as current: a few poll intervals, so
    /// one missed round does not dim a row but a real outage does.
    pub max_age_secs: i64,
    /// The most symbols one request carries.
    pub batch: usize,
}

/// How a source's candles are held.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachePolicy {
    /// Candles younger than this many seconds are served without a request.
    pub fresh_secs: u64,
    /// Which series are held.
    pub of: CachedSeries,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CachedSeries {
    Every,
    /// Only the series folded from finer candles, which take several requests.
    Folded,
}

impl DataSource {
    /// Every source, in the order the picker asks them. The one list: a new variant fails to compile
    /// in [`table::profile_of`] until it has a profile, and the tests until it is listed here.
    pub const ALL: [DataSource; 16] = ALL;

    pub fn profile(self) -> &'static Profile {
        profile_of(self)
    }

    /// The sources whose forming candle a chart takes from a socket instead of waiting for the
    /// next refresh.
    pub fn candle_streams() -> impl Iterator<Item = DataSource> {
        Self::ALL
            .into_iter()
            .filter(|source| source.profile().streams.candles)
    }

    /// The sources whose watchlist prices come from a shared ticker socket, REST filling in.
    pub fn quote_streams() -> impl Iterator<Item = DataSource> {
        Self::ALL
            .into_iter()
            .filter(|source| source.profile().streams.quotes)
    }

    /// Whether watchlist prices of this source come from a ticker socket.
    pub fn streams_quotes(self) -> bool {
        self.profile().streams.quotes
    }

    /// The sources found by symbol and charted with candles, in the order the symbol picker asks
    /// them: exchanges first, then DEX pairs, the coin aggregator and stocks (so an exchange's
    /// market ranks before a DEX's of the same name).
    pub fn symbol_markets() -> impl Iterator<Item = DataSource> {
        let mut sources: Vec<DataSource> = Self::ALL
            .into_iter()
            .filter(|source| source.profile().class.is_symbol_market())
            .collect();
        sources.sort_by_key(|source| source.profile().class.picker_rank());
        sources.into_iter()
    }

    /// The crypto sources (exchanges, the coin aggregator and DEX pairs), in the order the picker
    /// asks them.
    pub fn crypto_markets() -> impl Iterator<Item = DataSource> {
        Self::ALL
            .into_iter()
            .filter(|source| source.profile().class.is_crypto())
    }

    /// The spot exchanges, in the order the picker asks them.
    pub fn exchanges() -> impl Iterator<Item = DataSource> {
        Self::ALL
            .into_iter()
            .filter(|source| source.profile().class == Class::Exchange)
    }

    /// The stock sources.
    pub fn stock_markets() -> impl Iterator<Item = DataSource> {
        Self::ALL
            .into_iter()
            .filter(|source| source.profile().class == Class::Stock)
    }

    /// The prediction markets, in the order their answers are listed.
    pub fn prediction_markets() -> impl Iterator<Item = DataSource> {
        Self::ALL
            .into_iter()
            .filter(|source| source.profile().class == Class::Prediction)
    }
}

#[cfg(test)]
mod tests;
