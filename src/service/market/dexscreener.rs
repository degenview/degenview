use super::geckoterminal::GeckoTerminal;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use crate::model::market::asset_name::AssetNames;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::net::url::encode_component;
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

const BASE_URL: &str = "https://api.dexscreener.com/latest/dex";

pub struct DexScreener {
    http: Arc<dyn Http>,
    geckoterminal: GeckoTerminal,
}

impl DexScreener {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            geckoterminal: GeckoTerminal::new(context),
        }
    }
}

impl MarketSource for DexScreener {
    /// DEXScreener publishes no candles on its free tier, so charts come from
    /// GeckoTerminal, which indexes the same pools and does.
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        self.geckoterminal.fetch_klines(symbol, timeframe, limit)
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        self.geckoterminal
            .fetch_klines_range(symbol, timeframe, start_ms, end_ms, max)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = search_url(&q);
        let body = self
            .http
            .fetch(DataSource::DexScreener, &Request::get(&url))?;
        parse_search(&body, query)
    }
}

/// The search address for an already trimmed, lower-cased query.
fn search_url(query: &str) -> String {
    format!("{BASE_URL}/search?q={}", encode_component(query))
}

fn parse_search(body: &str, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::DexScreener, error))?;

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
            names: Box::new(AssetNames::new(
                pair.pointer("/baseToken/name").and_then(Value::as_str),
                pair.pointer("/quoteToken/name").and_then(Value::as_str),
            )),
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
    use crate::service::market::error::ErrorKind;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::market::ports::http;

    #[test]
    fn a_query_outside_ascii_is_escaped_in_the_search_address() {
        assert_eq!(
            search_url("比特币 & co"),
            format!("{BASE_URL}/search?q=%E6%AF%94%E7%89%B9%E5%B8%81%20%26%20co")
        );
    }

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

    #[test]
    fn search_is_one_request_for_the_trimmed_lower_cased_query_and_a_failure_is_its_kind() {
        let rig = Rig::new();
        rig.http.on(
            "/search?q=wif",
            r#"{"pairs":[{"baseToken":{"symbol":"WIF","name":"dogwifhat"},"quoteToken":{"symbol":"SOL"},"pairAddress":"a1","priceUsd":"2.5","liquidity":{"usd":1000.0}}]}"#,
        );
        let dexscreener = DexScreener::new(&rig.context());
        let found = dexscreener.search_tickers("  WIF ").expect("hits");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbol, "WIF/SOL");
        assert_eq!(
            rig.http.urls(),
            ["https://api.dexscreener.com/latest/dex/search?q=wif"]
        );

        rig.http.fail("/search?q=", http::Error::Status(429));
        assert_eq!(
            dexscreener.search_tickers("wif").unwrap_err().kind,
            ErrorKind::RateLimited
        );
    }
}
