use super::*;
use std::collections::HashSet;

/// How many variants `DataSource` has. Bumped with [`position`], which cannot compile without an
/// arm for a new variant.
const VARIANTS: usize = 13;

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
        DataSource::CoinGecko => 7,
        DataSource::DexScreener => 8,
        DataSource::Alpaca => 9,
        DataSource::Polymarket => 10,
        DataSource::Kalshi => 11,
        DataSource::CoinMarketCap => 12,
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
            !matches!(profile.pair, PairStyle::None),
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
        ]
    );
}

#[test]
fn the_picker_lists_markets_by_symbol_and_predictions_apart() {
    let markets: Vec<_> = DataSource::symbol_markets().collect();
    assert_eq!(markets.len(), 10);
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
    assert_eq!(timing(DataSource::DexScreener), (10, 60, 30));
    assert_eq!(timing(DataSource::CoinGecko), (30, 120, 250));
    assert_eq!(timing(DataSource::Alpaca), (5, 45, 200));
    assert_eq!(timing(DataSource::Polymarket), (30, 120, 100));
    assert_eq!(timing(DataSource::Kalshi), (30, 120, 100));
    assert_eq!(timing(DataSource::CoinMarketCap), (0, 0, 100));
}
