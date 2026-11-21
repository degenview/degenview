use super::*;
use crate::model::market::MarketKind;
use std::collections::HashSet;

/// How many variants `DataSource` has. Bumped with [`position`], which cannot compile without an
/// arm for a new variant.
const VARIANTS: usize = 15;

/// Where `source` stands in [`DataSource::ALL`]. Exhaustive, so a new variant does not compile
/// until it is placed here, and then the tests below make it appear in the list.
fn position(source: DataSource) -> usize {
    match source {
        DataSource::Binance => 0,
        DataSource::Coinbase => 1,
        DataSource::Okx => 2,
        DataSource::Kraken => 3,
        DataSource::Bybit => 4,
        DataSource::Gate => 5,
        DataSource::Mexc => 6,
        DataSource::Hyperliquid => 7,
        DataSource::Aster => 8,
        DataSource::CoinGecko => 9,
        DataSource::DexScreener => 10,
        DataSource::Alpaca => 11,
        DataSource::Polymarket => 12,
        DataSource::Kalshi => 13,
        DataSource::CoinMarketCap => 14,
    }
}

#[test]
fn all_lists_every_source_once_and_in_the_order_it_is_placed() {
    assert_eq!(DataSource::ALL.len(), VARIANTS);
    for (index, source) in DataSource::ALL.into_iter().enumerate() {
        assert_eq!(position(source), index, "{source:?}");
    }
}

#[test]
fn every_source_has_its_own_profile_with_unique_names() {
    let mut slugs = HashSet::new();
    let mut display_names = HashSet::new();
    let mut mcp_ids = HashSet::new();
    for source in DataSource::ALL {
        let profile = source.profile();
        assert!(!profile.display_name.is_empty(), "{source:?}");
        assert!(slugs.insert(profile.slug), "{source:?} repeats a slug");
        assert!(
            display_names.insert(profile.display_name),
            "{source:?} repeats a name"
        );
        assert!(
            mcp_ids.insert(profile.mcp_id),
            "{source:?} repeats an MCP id"
        );
        assert_eq!(
            profile.slug,
            profile.slug.to_uppercase(),
            "{source:?}'s slug is upper case"
        );
    }
}

#[test]
fn a_slug_names_its_source_in_any_case() {
    for source in DataSource::ALL {
        let slug = source.profile().slug;
        assert_eq!(DataSource::from_watchlist_slug(slug), Some(source));
        assert_eq!(
            DataSource::from_watchlist_slug(&slug.to_lowercase()),
            Some(source)
        );
    }
    assert_eq!(DataSource::from_watchlist_slug("KUCOIN"), None);
}

#[test]
fn candles_are_what_the_series_and_the_natives_say() {
    for source in DataSource::ALL {
        let profile = source.profile();
        match profile.series {
            Series::Candles => assert!(!profile.natives.is_empty(), "{source:?} has no natives"),
            Series::Samples | Series::Line => {
                assert!(profile.natives.is_empty(), "{source:?} has natives")
            }
        }
    }
}

#[test]
fn a_pair_style_that_needs_a_bare_asset_quote_names_one() {
    for source in DataSource::ALL {
        let profile = source.profile();
        assert_eq!(
            profile.home_quote.is_some(),
            matches!(profile.pair, PairStyle::Concat(_) | PairStyle::Separated(_)),
            "{source:?}"
        );
    }
}

#[test]
fn the_streams_follow_the_lists_they_replaced() {
    assert_eq!(
        DataSource::candle_streams().collect::<Vec<_>>(),
        [
            DataSource::Binance,
            DataSource::Okx,
            DataSource::Kraken,
            DataSource::Bybit,
            DataSource::Gate,
            DataSource::Mexc,
            DataSource::Hyperliquid,
            DataSource::Aster,
        ]
    );
    assert_eq!(
        DataSource::quote_streams().collect::<Vec<_>>(),
        [
            DataSource::Coinbase,
            DataSource::Okx,
            DataSource::Kraken,
            DataSource::Bybit,
            DataSource::Gate,
            DataSource::Hyperliquid,
            DataSource::Aster,
        ]
    );
}

#[test]
fn only_the_sources_with_a_small_rest_allowance_refresh_their_charts_slowly() {
    let slow: Vec<_> = DataSource::ALL
        .into_iter()
        .filter(|source| source.profile().slow_refresh)
        .collect();
    assert_eq!(
        slow,
        [
            DataSource::Kraken,
            DataSource::CoinGecko,
            DataSource::DexScreener
        ]
    );
}

#[test]
fn the_picker_lists_markets_by_symbol_and_predictions_apart() {
    let markets: Vec<_> = DataSource::symbol_markets().collect();
    assert_eq!(markets.len(), 12);
    assert!(markets.iter().all(|source| !source.is_prediction_market()));
    assert!(!markets.contains(&DataSource::CoinMarketCap));
    assert_eq!(
        DataSource::prediction_markets().collect::<Vec<_>>(),
        [DataSource::Polymarket, DataSource::Kalshi]
    );
}

#[test]
fn quotes_are_polled_as_each_source_allows() {
    let timing = |source: DataSource| {
        let quotes = source.profile().quotes;
        (quotes.refresh_secs, quotes.max_age_secs, quotes.batch)
    };
    assert_eq!(timing(DataSource::Binance), (5, 45, 100));
    assert_eq!(timing(DataSource::Coinbase), (5, 90, 20));
    assert_eq!(timing(DataSource::Okx), (5, 45, 2_000));
    assert_eq!(timing(DataSource::Kraken), (5, 45, 100));
    assert_eq!(timing(DataSource::Bybit), (5, 45, 2_000));
    assert_eq!(timing(DataSource::Gate), (5, 45, 2_500));
    assert_eq!(timing(DataSource::Mexc), (5, 45, 2_500));
    assert_eq!(timing(DataSource::Hyperliquid), (10, 60, 2_000));
    assert_eq!(timing(DataSource::Aster), (10, 60, 1_000));
    assert_eq!(timing(DataSource::DexScreener), (10, 60, 30));
    assert_eq!(timing(DataSource::CoinGecko), (30, 120, 250));
    assert_eq!(timing(DataSource::Alpaca), (5, 45, 200));
    assert_eq!(timing(DataSource::Polymarket), (30, 120, 100));
    assert_eq!(timing(DataSource::Kalshi), (30, 120, 100));
    assert_eq!(timing(DataSource::CoinMarketCap), (0, 0, 100));
}

#[test]
fn only_the_sources_whose_ids_say_nothing_title_a_chart_by_its_hit() {
    let opaque: Vec<_> = DataSource::ALL
        .into_iter()
        .filter(|source| source.profile().opaque_ids)
        .collect();
    assert_eq!(opaque, [DataSource::Hyperliquid, DataSource::DexScreener]);
}

#[test]
fn hyperliquid_lists_perpetuals_and_spot_by_its_own_case_sensitive_names() {
    let profile = DataSource::Hyperliquid.profile();
    assert_eq!(profile.kinds, [MarketKind::Perp, MarketKind::Spot]);
    assert_eq!(profile.default_kind(), MarketKind::Perp);
    assert_eq!(
        (profile.slug, profile.display_name),
        ("HYPERLIQUID", "Hyperliquid")
    );
    assert_eq!(profile.home_quote, None);
    assert!(profile.streams.candles && profile.streams.quotes);
    assert_eq!(
        DataSource::from_watchlist_slug("hyperliquid"),
        Some(DataSource::Hyperliquid)
    );
    // Its names are case-sensitive: they are asked and keyed as listed, however they are spelled.
    for name in ["BTC", "kPEPE", "xyz:SP500", "PURR/USDC", "@107"] {
        let instrument = crate::model::InstrumentId::new(DataSource::Hyperliquid, name);
        assert_eq!(instrument.api_symbol(), name);
        assert_eq!(instrument.key(), format!("HYPERLIQUID:{name}"));
    }
    // The `1w` and `1M` of Hyperliquid open off this app's grid, so they are folded, not native.
    let natives = DataSource::Hyperliquid.natives();
    assert!(!natives.contains(&Timeframe::WEEK_1) && !natives.contains(&Timeframe::MONTH_1));
    assert!(DataSource::Hyperliquid.serves(Timeframe::WEEK_1));
    assert!(DataSource::Hyperliquid.serves(Timeframe::YEAR_1));
    // `2m` is not an interval of Hyperliquid either: it folds from 1m.
    assert!(DataSource::Hyperliquid.serves(Timeframe::parse("2m").unwrap()));
}

#[test]
fn aster_lists_perpetuals_only_and_splits_its_ids_by_its_own_quotes() {
    let profile = DataSource::Aster.profile();
    assert_eq!(profile.kinds, [MarketKind::Perp]);
    assert_eq!((profile.slug, profile.display_name), ("ASTER", "Aster"));
    assert!(profile.streams.candles && profile.streams.quotes);
    assert_eq!(
        DataSource::from_watchlist_slug("aster"),
        Some(DataSource::Aster)
    );
    for (id, asset) in [
        ("BTCUSDT", "BTC"),
        ("BTCUSD1", "BTC"),
        ("ETHUSDC", "ETH"),
        ("QQQUSDT", "QQQ"),
        ("SPXUSDT", "SPX"),
    ] {
        assert_eq!(DataSource::Aster.base_asset(id), asset, "{id}");
    }
    // `BTCU` and `ETHU` are quoted in `U`: listed ids, never a bare asset that gets `USDT`, while an
    // asset that merely ends in `U` still does.
    for (id, asset) in [("BTCU", "BTC"), ("ETHU", "ETH")] {
        assert_eq!(DataSource::Aster.base_asset(id), asset, "{id}");
        assert_eq!(
            DataSource::Aster.quote_asset(id).as_deref(),
            Some("U"),
            "{id}"
        );
        let instrument = crate::model::InstrumentId::new(DataSource::Aster, id);
        assert_eq!(instrument.api_symbol(), id);
        assert_eq!(instrument.key(), format!("ASTER:{id}"));
    }
    for (typed, id) in [("BTC", "BTCUSDT"), ("MENU", "MENUUSDT"), ("btcu", "BTCU")] {
        assert_eq!(
            crate::model::InstrumentId::new(DataSource::Aster, typed).api_symbol(),
            id,
            "{typed}"
        );
    }
    // The grid is Binance's: the same natives, no `3d`.
    assert_eq!(DataSource::Aster.natives(), DataSource::Binance.natives());
    assert!(
        !DataSource::Aster
            .natives()
            .contains(&Timeframe::parse("3d").unwrap())
    );
    assert!(DataSource::Aster.serves(Timeframe::parse("3d").unwrap()));
}
