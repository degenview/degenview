//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use super::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::Outcome;
use crate::service::watchlist_quotes::provider::{
    QuoteFailure, QuoteRequest, from_change_percent, get, number,
};
use serde_json::Value;
use std::collections::HashMap;

/// The network of the pair whose address is `address`, from a DEXScreener search.
pub(super) fn resolve_chain(address: &str) -> Option<String> {
    let body = get(
        DataSource::DexScreener,
        &format!("{BASE_URL}/search?q={address}"),
        &[],
    )
    .ok()?;
    parse_chain(&body, address)
}

pub(super) fn fetch_quotes(requests: &[QuoteRequest]) -> Outcome {
    // One request per chain for up to 30 pools each. Pairs without a known chain are left out.
    let mut by_chain: Vec<(&str, Vec<&QuoteRequest>)> = Vec::new();
    for request in requests {
        if let Some(chain) = request.chain.as_deref().filter(|chain| *chain != "unknown") {
            match by_chain.iter_mut().find(|(name, _)| *name == chain) {
                Some((_, pairs)) => pairs.push(request),
                None => by_chain.push((chain, vec![request])),
            }
        }
    }
    let mut outcome = Outcome::default();
    let mut limited = false;
    for (chain, pairs) in by_chain {
        for page in pairs.chunks(DataSource::DexScreener.profile().quotes.batch) {
            let names: Vec<String> = page.iter().map(|request| request.symbol.clone()).collect();
            if limited {
                outcome.failed.push((names, QuoteFailure::RateLimited));
                continue;
            }
            let addresses = names.join(",");
            match get(
                DataSource::DexScreener,
                &format!("{BASE_URL}/pairs/{chain}/{addresses}"),
                &[],
            ) {
                Ok(body) => outcome.quotes.extend(parse_dex_pairs(&body, page)),
                Err(failure) => {
                    limited = matches!(failure, QuoteFailure::RateLimited);
                    outcome.failed.push((names, failure));
                }
            }
        }
    }
    outcome
}

/// `{"pairs":[{"pairAddress","priceUsd","priceChange":{"h24"},"volume":{"h24"}}]}`.
pub(in crate::service) fn parse_dex_pairs(
    body: &str,
    requests: &[&QuoteRequest],
) -> HashMap<String, SourceQuote> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return HashMap::new();
    };
    let pairs: HashMap<String, &Value> = value
        .get("pairs")
        .and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| Some((pair.get("pairAddress")?.as_str()?.to_lowercase(), pair)))
                .collect()
        })
        .unwrap_or_default();
    let now = crate::clock::now_ms();
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(pair) = pairs.get(&request.symbol.to_lowercase()) else {
            continue;
        };
        let Some(price) = number(pair.get("priceUsd")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = from_change_percent(price, number(pair.pointer("/priceChange/h24")));
        quote.volume_24h = number(pair.pointer("/volume/h24"));
        quote.volume_kind = quote.volume_24h.map(|_| VolumeKind::QuoteCurrency);
        quote.timestamp_ms = Some(now);
        quotes.insert(request.symbol.clone(), quote);
    }
    quotes
}

/// The network of the pair whose address is `address`, from a DEXScreener search.
pub(in crate::service) fn parse_chain(body: &str, address: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("pairs")?
        .as_array()?
        .iter()
        .find(|pair| {
            pair.get("pairAddress")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(address))
        })?
        .get("chainId")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dex_pairs_match_by_address_ignoring_case() {
        let body = r#"{"pairs":[{"pairAddress":"0xAbC","priceUsd":"0.5","priceChange":{"h24":-50},"volume":{"h24":1234.5}}]}"#;
        let wanted = QuoteRequest {
            symbol: "0xabc".to_owned(),
            chain: Some("solana".to_owned()),
        };
        let other = QuoteRequest {
            symbol: "0xdef".to_owned(),
            chain: Some("solana".to_owned()),
        };
        let quotes = parse_dex_pairs(body, &[&wanted, &other]);
        assert_eq!(quotes.len(), 1);
        let quote = &quotes["0xabc"];
        assert_eq!(quote.price, 0.5);
        assert!((quote.previous_day_price.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(quote.volume_24h, Some(1234.5));
        assert!(parse_dex_pairs("garbage", &[&wanted]).is_empty());
        assert!(parse_dex_pairs(r#"{"pairs":null}"#, &[&wanted]).is_empty());
    }

    #[test]
    fn chain_comes_from_the_matching_pair() {
        let body = r#"{"pairs":[
            {"pairAddress":"0x111","chainId":"ethereum"},
            {"pairAddress":"0xAAA","chainId":"base"}]}"#;
        assert_eq!(parse_chain(body, "0xaaa").as_deref(), Some("base"));
        assert_eq!(parse_chain(body, "0x999"), None);
        assert_eq!(parse_chain("{}", "0x1"), None);
    }
}
