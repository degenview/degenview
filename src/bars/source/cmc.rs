//! CoinMarketCap's charts: the dashboard keys a chart's symbol holds and their titles.

pub const ALTCOIN_HISTORICAL: &str = "cmc.altcoin_season_historical";
pub const ALTCOIN_LATEST: &str = "cmc.altcoin_season_latest";
pub const FEAR_GREED_HISTORICAL: &str = "cmc.fear_and_greed_historical";
pub const FEAR_GREED_LATEST: &str = "cmc.fear_and_greed_latest";

/// The chart types offered by the CoinMarketCap tab.
pub const CHART_TYPES: [(&str, &str); 4] = [
    (ALTCOIN_HISTORICAL, "Altcoin Season History"),
    (ALTCOIN_LATEST, "Altcoin Season Index"),
    (FEAR_GREED_HISTORICAL, "Fear & Greed History"),
    (FEAR_GREED_LATEST, "Fear & Greed Index"),
];

/// The chart title for a CMC chart key; unknown keys read as themselves.
pub fn title(symbol: &str) -> &str {
    CHART_TYPES
        .iter()
        .find(|(key, _)| *key == symbol)
        .map_or(symbol, |(_, title)| title)
}

/// Whether `symbol` is one of the "Latest" gauge charts, which draw a reading instead of a series.
pub fn is_latest(symbol: &str) -> bool {
    matches!(symbol, ALTCOIN_LATEST | FEAR_GREED_LATEST)
}
