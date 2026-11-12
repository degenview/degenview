use crate::model::Kline;
use crate::service::{MarketSource, SearchResult, rank_results};
use serde_json::Value;
use std::collections::HashMap;

const BASE_URL: &str = "https://api.dexscreener.com/latest/dex";

pub struct DexScreener;

pub const DEXSCREENER: DexScreener = DexScreener;

impl MarketSource for DexScreener {
    /// DEXScreener publishes no candles on its free tier, so charts come from
    /// GeckoTerminal, which indexes the same pools and does.
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        super::geckoterminal::fetch_klines(symbol, interval, limit)
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, String> {
        super::geckoterminal::fetch_klines_range(symbol, interval, start_ms, end_ms, max)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{BASE_URL}/search?q={q}");
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )
        .map_err(|error| format!("Network error: {error}"))?;
        parse_search(&body, query)
    }
}

fn parse_search(body: &str, query: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;

    // Deduplicate by base+quote symbol, keeping the highest-liquidity pair.
    let mut by_pair: HashMap<String, (f64, SearchResult)> = HashMap::new();
    for pair in value
        .get("pairs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let (Some(base), Some(quote)) = (
            pair.pointer("/baseToken/symbol").and_then(Value::as_str),
            pair.pointer("/quoteToken/symbol").and_then(Value::as_str),
        ) else {
            continue;
        };
        let key = format!("{}/{}", base.to_uppercase(), quote.to_uppercase());
        let liquidity = pair
            .pointer("/liquidity/usd")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let price = pair
            .get("priceUsd")
            .and_then(Value::as_str)
            .and_then(|text| text.parse::<f64>().ok());
        let result = SearchResult {
            symbol: key.clone(),
            full_symbol: pair
                .get("pairAddress")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            source: crate::model::DataSource::DexScreener,
            price,
            event: None,
            base: Some(base.to_uppercase()),
            quote: Some(quote.to_uppercase()),
        };
        match by_pair.get(&key) {
            Some((existing, _)) if *existing >= liquidity => {}
            _ => {
                by_pair.insert(key, (liquidity, result));
            }
        }
    }

    let mut list: Vec<(f64, SearchResult)> = by_pair.into_values().collect();
    list.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let results: Vec<SearchResult> = list.into_iter().map(|(_, result)| result).collect();
    Ok(rank_results(results, query).into_iter().take(20).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_dedupes_by_pair_keeping_highest_liquidity() {
        let body = r#"{"pairs":[
            {"baseToken":{"symbol":"WIF"},"quoteToken":{"symbol":"SOL"},"pairAddress":"a1","priceUsd":"2.5","liquidity":{"usd":1000.0}},
            {"baseToken":{"symbol":"WIF"},"quoteToken":{"symbol":"SOL"},"pairAddress":"a2","priceUsd":"2.6","liquidity":{"usd":5000.0}}
        ]}"#;
        let results = parse_search(body, "wif").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].full_symbol, "a2");
        assert_eq!(results[0].price, Some(2.6));
    }
}
