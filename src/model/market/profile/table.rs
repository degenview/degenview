//! The profile of every source. One `static` per source with its candle lengths above it, and the
//! two functions that look them up. Adding a source: a `DataSource` variant, an entry here, a line
//! in [`ALL`].

use super::{
    CachePolicy, CachedSeries, Casing, ChangeBasis, Class, Concat, Listed, OlderHistory, PairStyle,
    Profile, QuoteTiming, Separated, Series, Streams,
};
use crate::model::market::MarketKind;
use crate::model::market::natives::{GECKOTERMINAL_HISTORY_DAYS, GECKOTERMINAL_PAGE};
use crate::model::market::source::DataSource;
use crate::model::{TimeUnit, Timeframe};

/// Every source, in the order lists of sources are searched and shown.
pub const ALL: [DataSource; 16] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::Okx,
    DataSource::Kraken,
    DataSource::Bybit,
    DataSource::Gate,
    DataSource::Mexc,
    DataSource::Hyperliquid,
    DataSource::Aster,
    DataSource::Lighter,
    DataSource::CoinGecko,
    DataSource::DexScreener,
    DataSource::Alpaca,
    DataSource::Polymarket,
    DataSource::Kalshi,
    DataSource::CoinMarketCap,
];

pub fn profile_of(source: DataSource) -> &'static Profile {
    match source {
        DataSource::Binance => &BINANCE,
        DataSource::Coinbase => &COINBASE,
        DataSource::Okx => &OKX,
        DataSource::Kraken => &KRAKEN,
        DataSource::Bybit => &BYBIT,
        DataSource::Gate => &GATE,
        DataSource::Mexc => &MEXC,
        DataSource::Hyperliquid => &HYPERLIQUID,
        DataSource::Aster => &ASTER,
        DataSource::Lighter => &LIGHTER,
        DataSource::CoinGecko => &COINGECKO,
        DataSource::DexScreener => &DEXSCREENER,
        DataSource::Alpaca => &ALPACA,
        DataSource::Polymarket => &POLYMARKET,
        DataSource::Kalshi => &KALSHI,
        DataSource::CoinMarketCap => &COINMARKETCAP,
    }
}

/// The kinds of market a source lists: spot, which is everything that is not a perpetual.
const SPOT: &[MarketKind] = &[MarketKind::Spot];

/// A venue that lists perpetual contracts only.
const PERP: &[MarketKind] = &[MarketKind::Perp];

/// A venue with perpetuals and spot markets, perpetuals being what a bare id means.
const PERP_AND_SPOT: &[MarketKind] = &[MarketKind::Perp, MarketKind::Spot];

const fn minutes(count: u16) -> Timeframe {
    native(count, TimeUnit::Minute)
}

const fn hours(count: u16) -> Timeframe {
    native(count, TimeUnit::Hour)
}

const fn days(count: u16) -> Timeframe {
    native(count, TimeUnit::Day)
}

/// A native length, checked when the list is built.
const fn native(count: u16, unit: TimeUnit) -> Timeframe {
    match Timeframe::new(count, unit) {
        Ok(timeframe) => timeframe,
        Err(_) => panic!("a native timeframe is out of range"),
    }
}

/// Quote assets Binance pairs end in, longest first so BTCFDUSD is BTC/FDUSD instead of
/// BTCFD/USD. Matches Swift's MarketSymbol quote list.
const BINANCE_QUOTES: [&str; 16] = [
    "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY", "BRL", "JPY",
    "USD", "BTC", "ETH", "BNB",
];

/// Quote assets MEXC's spot catalog ends its ids in, the dollar stablecoins first. A one-letter base
/// is a real asset before those (`SUSDT`), where before another quote it is a wrapped one (`WBTC` is
/// not W/BTC).
const MEXC_QUOTES: [&str; 9] = [
    "USDT", "USDC", "USD1", "USDE", "USDF", "EUR", "BRL", "ETH", "BTC",
];

/// Binance-shaped pairs, as Binance and Bybit write them. A one-letter "base" is a bare wrapped asset
/// (WBTC, WETH), not a pair.
const BINANCE_PAIRS: Concat = Concat {
    exceptions: &[],
    quotes: &BINANCE_QUOTES,
    one_letter_base_quotes: BINANCE_QUOTES.len(),
    wrapped_asset_guard: true,
    dropped: &['/', '-'],
};

const COINBASE_QUOTE_TIMING: QuoteTiming = QuoteTiming {
    refresh_secs: 5,
    // Coinbase is refreshed from REST every 30 seconds when the socket has been quiet.
    max_age_secs: 90,
    batch: 20,
};

/// Seconds a quote is current, and refreshed, for an exchange that quotes every market in one call.
const fn exchange_quotes(batch: usize) -> QuoteTiming {
    QuoteTiming {
        refresh_secs: 5,
        max_age_secs: 45,
        batch,
    }
}

/// The kline intervals of the Binance-shaped API, Binance's and Aster's. Its `3d` is left out: both
/// count those from a day of their own (not a multiple of three since 1970-01-01), so they would
/// not match every other source's. Weeks start on Monday and months on the first, as here.
const BINANCE_SHAPED_NATIVES: [Timeframe; 14] = [
    minutes(1),
    minutes(3),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(2),
    hours(4),
    hours(6),
    hours(8),
    hours(12),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

pub static BINANCE: Profile = Profile {
    display_name: "Binance",
    slug: "BINANCE",
    mcp_id: "binance",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Concat(BINANCE_PAIRS),
    home_quote: Some("USDT"),
    natives: &BINANCE_SHAPED_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: false,
    },
    change: ChangeBasis::Rolling24h,
    quotes: exchange_quotes(100),
    // Binance answers a length of its own in one cheap request and has a live feed; folded from a
    // finer one, a series can take a dozen requests, so it is held and refreshed by its tail.
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Folded,
    }),
};

/// Coinbase's granularities: 60, 300, 900, 3600, 21600 and 86400 seconds.
const COINBASE_NATIVES: [Timeframe; 6] = [
    minutes(1),
    minutes(5),
    minutes(15),
    hours(1),
    hours(6),
    Timeframe::DAY_1,
];

pub static COINBASE: Profile = Profile {
    display_name: "Coinbase",
    slug: "COINBASE",
    mcp_id: "coinbase",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Separated(Separated {
        separator: '-',
        noun: "products",
        also_typed_as: &['/'],
        aliases: &[],
    }),
    home_quote: Some("USD"),
    natives: &COINBASE_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: false,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    quotes: COINBASE_QUOTE_TIMING,
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Every,
    }),
};

/// OKX's candle bars. Its plain `6H`, `12H`, `1D`, `1W`, `1M` and `3M` are cut at UTC+8
/// midnight, so the `utc` bars are the ones used: they are cut as everything else is. Left out:
/// `8H` (OKX has no such bar), `2D` and `3D` (counted from a day that is not a multiple of the
/// length since 1970-01-01, as Binance's `3d` is: they fold from days) and the year (it folds from
/// the quarter).
const OKX_NATIVES: [Timeframe; 14] = [
    minutes(1),
    minutes(3),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(2),
    hours(4),
    hours(6),
    hours(12),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
    Timeframe::MONTH_3,
];

pub static OKX: Profile = Profile {
    display_name: "OKX",
    slug: "OKX",
    mcp_id: "okx",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Separated(Separated {
        separator: '-',
        noun: "spot markets",
        also_typed_as: &['/'],
        aliases: &[],
    }),
    home_quote: Some("USDT"),
    natives: &OKX_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    // One call answers every spot market, so a round is one request however many rows.
    quotes: exchange_quotes(2_000),
    // OKX answers 300 candles a request, so even a chart of its own length takes two.
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Every,
    }),
};

/// Kraken's OHLC intervals in minutes: 1, 5, 15, 30, 60, 240 and 1440. Left out: its week
/// (`10080`) starts on a Thursday and its `21600` is a 15-day length counted from 1970-01-01, so
/// weeks, months and years fold from days.
const KRAKEN_NATIVES: [Timeframe; 7] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(4),
    Timeframe::DAY_1,
];

pub static KRAKEN: Profile = Profile {
    display_name: "Kraken",
    slug: "KRAKEN",
    mcp_id: "kraken",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    // Kraken's own spelling of an asset is the legacy one (`XBT`, `XDG`) in some places; its APIs take
    // the common ticker (`BTC`, `DOGE`) and refuse the legacy one.
    pair: PairStyle::Separated(Separated {
        separator: '/',
        noun: "markets",
        also_typed_as: &['-', ' '],
        aliases: &[("XBT", "BTC"), ("XDG", "DOGE")],
    }),
    home_quote: Some("USD"),
    natives: &KRAKEN_NATIVES,
    // Kraken's OHLC answers the newest 720 candles whatever is asked: the chart's first 500 leave the
    // rest for the first older page, and a second page is empty.
    older_history: OlderHistory::Always,
    slow_refresh: true,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    quotes: exchange_quotes(100),
    cache: None,
};

/// Bybit's kline intervals: 1, 3, 5, 15, 30, 60, 120, 240, 360 and 720 minutes, `D`, `W` and `M`.
/// All of them are cut where this app cuts: days at UTC midnight, weeks on Monday and months on
/// the calendar, so the quarter and the year fold from months.
const BYBIT_NATIVES: [Timeframe; 13] = [
    minutes(1),
    minutes(3),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(2),
    hours(4),
    hours(6),
    hours(12),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

pub static BYBIT: Profile = Profile {
    display_name: "Bybit",
    slug: "BYBIT",
    mcp_id: "bybit",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Concat(BINANCE_PAIRS),
    home_quote: Some("USDT"),
    natives: &BYBIT_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    // One call answers every spot market, so a round is one request however many rows.
    quotes: exchange_quotes(2_000),
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Folded,
    }),
};

/// Gate.io's documented candlestick intervals (it also answers `3m`, `2h`, `6h` and `12h`, which
/// its docs do not list, so they fold): 1, 5, 15, 30 minutes, 1, 4 and 8 hours, `1d`, `7d` and
/// `30d`. Its days are cut at UTC midnight, its `7d` weeks start on Monday and its `30d` is the
/// calendar month, so all of them are cut where this app cuts.
const GATE_NATIVES: [Timeframe; 10] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(4),
    hours(8),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

pub static GATE: Profile = Profile {
    display_name: "Gate.io",
    slug: "GATEIO",
    mcp_id: "gateio",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Separated(Separated {
        separator: '_',
        noun: "markets",
        also_typed_as: &['/', '-', ' '],
        aliases: &[],
    }),
    home_quote: Some("USDT"),
    natives: &GATE_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    quotes: exchange_quotes(2_500),
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Folded,
    }),
};

/// MEXC's kline intervals: 1, 5, 15, 30 and 60 minutes, 4 and 8 hours, `1d`, `1W` and `1M`. They
/// are cut where this app cuts, except in the oldest part of its history: the days, weeks and
/// months before about 2018-12 are cut at UTC+8 midnight, and those candles are left out.
const MEXC_NATIVES: [Timeframe; 10] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(4),
    hours(8),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

pub static MEXC: Profile = Profile {
    display_name: "MEXC",
    slug: "MEXC",
    mcp_id: "mexc",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Concat(Concat {
        exceptions: &[],
        quotes: &MEXC_QUOTES,
        one_letter_base_quotes: 5,
        wrapped_asset_guard: false,
        dropped: &['/', '-', '_', ' '],
    }),
    home_quote: Some("USDT"),
    natives: &MEXC_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: false,
    },
    change: ChangeBasis::Rolling24h,
    quotes: exchange_quotes(2_500),
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Folded,
    }),
};

/// Hyperliquid's candle intervals: 1, 3, 5, 15 and 30 minutes, 1, 2, 4, 8 and 12 hours, `1d` and
/// `3d`. All are cut where this app cuts, checked against the API: minutes and hours from UTC
/// midnight, days at midnight and a `3d` on a multiple of three days since 1970-01-01. Left out:
/// its `1w` opens on a Thursday (weeks counted from 1970-01-01) and its `1M` is thirty days counted
/// from 1970-01-01, not the calendar month, so weeks, months and years fold from days.
const HYPERLIQUID_NATIVES: [Timeframe; 12] = [
    minutes(1),
    minutes(3),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(2),
    hours(4),
    hours(8),
    hours(12),
    Timeframe::DAY_1,
    days(3),
];

pub static HYPERLIQUID: Profile = Profile {
    display_name: "Hyperliquid",
    slug: "HYPERLIQUID",
    mcp_id: "hyperliquid",
    class: Class::Exchange,
    // A spot pair Hyperliquid numbers is `@107`: the chart is titled by its tokens, `HYPE/USDC`.
    opaque_ids: true,
    kinds: PERP_AND_SPOT,
    series: Series::Candles,
    // Candle volume is in the base asset: contracts of a perpetual, coins of a spot pair.
    volume: true,
    // Names are case-sensitive (`BTC`, `kPEPE`, `xyz:SP500`) and shown as Hyperliquid writes them.
    display: Casing::AsGiven,
    id: Casing::AsGiven,
    pair: PairStyle::Listed(Listed {
        namespace: ':',
        also_in_names: &[':', '/', '@', '-', '_', '.'],
        noun: "markets",
        examples: "BTC, xyz:SP500 or PURR/USDC",
    }),
    home_quote: None,
    natives: &HYPERLIQUID_NATIVES,
    // Only the newest ~5000 candles of an interval exist, whatever its length: a chart's first
    // fetch leaves the rest for its first older page, and the next page is empty.
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    // One call answers every market of a dex (or all spot pairs). The limit is 1,200 weight a
    // minute per IP and a candle request weighs about 20, so prices are asked every ten seconds.
    quotes: QuoteTiming {
        refresh_secs: 10,
        max_age_secs: 60,
        batch: 2_000,
    },
    // A request answers a window of candles, so a held series is refreshed by its last few.
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Every,
    }),
};

/// Quote assets Aster's perpetuals end in.
const ASTER_QUOTES: [&str; 3] = ["USDT", "USDC", "USD1"];

/// Its `BTCU` and `ETHU` are quoted in the stablecoin `U`. `U` is no quote of `ASTER_QUOTES`: any
/// asset ending in `U` would read as one of them, and a bare `BTCU` would be asked as `BTCUUSDT`.
const ASTER_EXCEPTIONS: [(&str, &str); 2] = [("BTC", "U"), ("ETH", "U")];

pub static ASTER: Profile = Profile {
    display_name: "Aster",
    slug: "ASTER",
    mcp_id: "aster",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: PERP,
    series: Series::Candles,
    // Candle volume is in the base asset: contracts of a perpetual, in coins.
    volume: true,
    display: Casing::Upper,
    id: Casing::AsGiven,
    pair: PairStyle::Concat(Concat {
        exceptions: &ASTER_EXCEPTIONS,
        quotes: &ASTER_QUOTES,
        one_letter_base_quotes: ASTER_QUOTES.len(),
        wrapped_asset_guard: false,
        dropped: &['/', '-', '_', ' '],
    }),
    home_quote: Some("USDT"),
    natives: &BINANCE_SHAPED_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    // Its batch ticker is the whole list in one answer (230 KB, weight 40 of 2,400 a minute), so
    // prices not on the socket (it takes 200 topics) are asked every ten seconds.
    quotes: QuoteTiming {
        refresh_secs: 10,
        max_age_secs: 60,
        batch: 1_000,
    },
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Folded,
    }),
};

/// Lighter's candle resolutions: 1, 5, 15 and 30 minutes, 1, 4 and 12 hours and `1d`, each cut where
/// this app cuts (minutes and hours from UTC midnight, days at midnight; checked against the API on
/// 2026-11-21: no bar of 500 in any of them is off the grid, none is missing). It answers none
/// of `3m`, `2h`, `6h`, `8h`, `3d`, `1w` and `1M` (`invalid param`), so those, weeks, months and
/// years fold from the bars below them.
const LIGHTER_NATIVES: [Timeframe; 8] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(4),
    hours(12),
    Timeframe::DAY_1,
];

pub static LIGHTER: Profile = Profile {
    display_name: "Lighter",
    slug: "LIGHTER",
    mcp_id: "lighter",
    class: Class::Exchange,
    opaque_ids: false,
    kinds: PERP,
    series: Series::Candles,
    // Candle volume is in the base asset: contracts of a perpetual, in coins.
    volume: true,
    // Its names are upper case already (`BTC`, `1000PEPE`, `US500`, `EURUSD`).
    display: Casing::Upper,
    id: Casing::Upper,
    // Names as Lighter lists them, no quote added: a pair-shaped name (`EURUSD`) is one market.
    pair: PairStyle::Listed(Listed {
        // No Lighter name has a namespace; the colon is the character a name never holds.
        namespace: ':',
        also_in_names: &['-', '_', '.'],
        noun: "markets",
        examples: "BTC, 1000PEPE or US500",
    }),
    home_quote: None,
    natives: &LIGHTER_NATIVES,
    older_history: OlderHistory::Always,
    // A REST allowance of about one request a second per IP, and the candle socket keeps a chart
    // current: charts ask every 15 seconds, not every 5.
    slow_refresh: true,
    streams: Streams {
        candles: true,
        quotes: true,
    },
    change: ChangeBasis::Rolling24h,
    // One call answers every market (85 KB), so prices are asked every ten seconds beside the
    // sockets.
    quotes: QuoteTiming {
        refresh_secs: 10,
        max_age_secs: 60,
        batch: 2_000,
    },
    // A request always answers the newest 500 bars, so a held series is patched by one request.
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Every,
    }),
};

// CoinGecko builds its candles from prices, so it has no native lengths.
pub static COINGECKO: Profile = Profile {
    display_name: "CoinGecko",
    slug: "COINGECKO",
    mcp_id: "coingecko",
    class: Class::Aggregator,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Samples,
    // Its OHLC endpoint has no volume column.
    volume: false,
    display: Casing::Upper,
    id: Casing::Lower,
    pair: PairStyle::None,
    home_quote: None,
    natives: &[],
    older_history: OlderHistory::Never,
    slow_refresh: true,
    streams: Streams {
        candles: false,
        quotes: false,
    },
    change: ChangeBasis::Rolling24h,
    quotes: QuoteTiming {
        refresh_secs: 30,
        max_age_secs: 120,
        batch: 250,
    },
    cache: Some(CachePolicy {
        fresh_secs: 120,
        of: CachedSeries::Every,
    }),
};

/// GeckoTerminal's OHLCV aggregates (it indexes DEXScreener's pools): minute 1, 5 and 15, hour 1,
/// 4 and 12, and the day.
const DEXSCREENER_NATIVES: [Timeframe; 7] = [
    minutes(1),
    minutes(5),
    minutes(15),
    hours(1),
    hours(4),
    hours(12),
    Timeframe::DAY_1,
];

pub static DEXSCREENER: Profile = Profile {
    display_name: "DEXScreener",
    slug: "DEXSCREENER",
    mcp_id: "dexscreener",
    class: Class::Dex,
    opaque_ids: true,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    // A DEX pair address is an id, not a name: it is left alone.
    display: Casing::AsGiven,
    id: Casing::AsGiven,
    pair: PairStyle::None,
    home_quote: None,
    natives: &DEXSCREENER_NATIVES,
    // GeckoTerminal's free API reaches back only 180 days.
    older_history: OlderHistory::Within {
        page: GECKOTERMINAL_PAGE as i64,
        days: GECKOTERMINAL_HISTORY_DAYS,
    },
    slow_refresh: true,
    streams: Streams {
        candles: false,
        quotes: false,
    },
    change: ChangeBasis::Rolling24h,
    quotes: QuoteTiming {
        refresh_secs: 10,
        max_age_secs: 60,
        batch: 30,
    },
    cache: Some(CachePolicy {
        fresh_secs: 30,
        of: CachedSeries::Every,
    }),
};

/// Alpaca serves any minute count to 59 and hour count to 23, but only the lengths that divide
/// the hour are used: it is not documented where its other bars start, and one that is not on our
/// midnight grid would cut buckets the other sources do not. Days, weeks and months are as it
/// stamps them (at New York midnight).
const ALPACA_NATIVES: [Timeframe; 8] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

pub static ALPACA: Profile = Profile {
    display_name: "Alpaca (IEX)",
    slug: "ALPACA",
    mcp_id: "alpaca",
    class: Class::Stock,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Candles,
    volume: true,
    display: Casing::Upper,
    id: Casing::Upper,
    pair: PairStyle::None,
    home_quote: None,
    natives: &ALPACA_NATIVES,
    older_history: OlderHistory::Always,
    slow_refresh: false,
    streams: Streams {
        candles: false,
        quotes: false,
    },
    change: ChangeBasis::PreviousClose,
    quotes: exchange_quotes(200),
    cache: Some(CachePolicy {
        fresh_secs: 0,
        of: CachedSeries::Every,
    }),
};

/// What a prediction market's quotes look like to the engine: probabilities, refreshed slowly.
const PREDICTION_QUOTES: QuoteTiming = QuoteTiming {
    refresh_secs: 30,
    max_age_secs: 120,
    batch: 100,
};

pub static POLYMARKET: Profile = Profile {
    display_name: "Polymarket",
    slug: "POLYMARKET",
    mcp_id: "polymarket",
    class: Class::Prediction,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Line,
    volume: false,
    display: Casing::AsGiven,
    id: Casing::AsGiven,
    pair: PairStyle::None,
    home_quote: None,
    natives: &[],
    older_history: OlderHistory::Never,
    slow_refresh: false,
    streams: Streams {
        candles: false,
        quotes: false,
    },
    change: ChangeBasis::Window,
    quotes: PREDICTION_QUOTES,
    cache: None,
};

pub static KALSHI: Profile = Profile {
    display_name: "Kalshi",
    slug: "KALSHI",
    mcp_id: "kalshi",
    class: Class::Prediction,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Line,
    volume: false,
    display: Casing::AsGiven,
    id: Casing::AsGiven,
    pair: PairStyle::None,
    home_quote: None,
    natives: &[],
    older_history: OlderHistory::Never,
    slow_refresh: false,
    streams: Streams {
        candles: false,
        quotes: false,
    },
    change: ChangeBasis::Window,
    quotes: PREDICTION_QUOTES,
    cache: None,
};

// CoinMarketCap's gauges are index lines, not markets: no candles, no quotes to poll.
pub static COINMARKETCAP: Profile = Profile {
    display_name: "CoinMarketCap",
    slug: "CMC",
    mcp_id: "coinmarketcap",
    class: Class::Index,
    opaque_ids: false,
    kinds: SPOT,
    series: Series::Line,
    volume: false,
    display: Casing::AsGiven,
    id: Casing::AsGiven,
    pair: PairStyle::None,
    home_quote: None,
    natives: &[],
    older_history: OlderHistory::Never,
    slow_refresh: false,
    streams: Streams {
        candles: false,
        quotes: false,
    },
    change: ChangeBasis::Window,
    quotes: QuoteTiming {
        refresh_secs: 0,
        max_age_secs: 0,
        batch: 100,
    },
    cache: None,
};
