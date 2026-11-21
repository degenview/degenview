//! The REST call behind a watchlist price of this venue, and the parsers of its answers.
//!
//! `metaAndAssetCtxs` answers every perpetual of one dex as `[meta, contexts]`, the contexts in the
//! order of the meta's `universe`; `spotMetaAndAssetCtxs` answers every spot pair, each context
//! naming its `coin`. A context is the same object the sockets push as `activeAssetCtx`.

use super::ids::{dex_of, is_spot};
use super::rest::INFO_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::Outcome;
use crate::service::watchlist_quotes::provider::{
    QuoteFailure, QuoteRequest, json, number, post_json,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// Which answer a market is in.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Book {
    /// The perpetuals of the main dex (`None`) or of a HIP-3 dex.
    Perps(Option<String>),
    Spot,
}

impl Book {
    fn of(id: &str) -> Self {
        if is_spot(id) {
            Self::Spot
        } else {
            Self::Perps(dex_of(id).map(str::to_owned))
        }
    }

    fn query(&self) -> Value {
        match self {
            Self::Perps(None) => json!({"type": "metaAndAssetCtxs"}),
            Self::Perps(Some(dex)) => json!({"type": "metaAndAssetCtxs", "dex": dex}),
            Self::Spot => json!({"type": "spotMetaAndAssetCtxs"}),
        }
    }

    fn parse(&self, body: &str) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
        match self {
            Self::Perps(_) => parse_perps(body),
            Self::Spot => parse_spot(body),
        }
    }
}

/// One question per dex (and one for the spot pairs) that the markets asked for are in; a question
/// that fails fails only its markets.
pub(super) fn fetch_quotes(requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
    let mut books: BTreeMap<Book, Vec<&QuoteRequest>> = BTreeMap::new();
    for request in requests {
        books
            .entry(Book::of(&request.symbol))
            .or_default()
            .push(request);
    }
    let mut outcome = Outcome::default();
    for (book, asked) in books {
        let answer = post_json(DataSource::Hyperliquid, INFO_URL, &book.query().to_string())
            .and_then(|body| book.parse(&body));
        match answer {
            Ok(quotes) => outcome.quotes.extend(asked.iter().filter_map(|request| {
                Some((request.symbol.clone(), *quotes.get(&request.symbol)?))
            })),
            Err(failure) => outcome.failed.push((
                asked.iter().map(|request| request.symbol.clone()).collect(),
                failure,
            )),
        }
    }
    Ok(outcome)
}

/// A market's price and its 24 hours: the mark price, the change against the price a day ago
/// (`prevDayPx`, a rolling reference) and the notional traded in the day (`dayNtlVlm`, in dollars).
/// `None` for a market with no price.
pub(super) fn context_quote(context: &Value) -> Option<SourceQuote> {
    let price = number(context.get("markPx")).filter(|price| *price > 0.0)?;
    let volume = number(context.get("dayNtlVlm"));
    Some(SourceQuote {
        price,
        previous_day_price: number(context.get("prevDayPx")).filter(|price| *price > 0.0),
        volume_24h: volume,
        volume_kind: volume.map(|_| VolumeKind::QuoteCurrency),
        timestamp_ms: None,
    })
}

fn answer_of(body: &str) -> Result<Vec<Value>, QuoteFailure> {
    match json(body)? {
        Value::Array(parts) if parts.len() == 2 => Ok(parts),
        _ => Err(QuoteFailure::Failed(
            "Unexpected Hyperliquid response".to_owned(),
        )),
    }
}

/// `[{"universe":[{"name":"BTC",…}]},[{"markPx":"…",…}]]`: the contexts pair with the universe by
/// position.
fn parse_perps(body: &str) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let parts = answer_of(body)?;
    let (Some(universe), Some(contexts)) = (
        parts[0].get("universe").and_then(Value::as_array),
        parts[1].as_array(),
    ) else {
        return Err(QuoteFailure::Failed(
            "Unexpected Hyperliquid response".to_owned(),
        ));
    };
    Ok(universe
        .iter()
        .zip(contexts)
        .filter_map(|(market, context)| {
            Some((
                market.get("name")?.as_str()?.to_owned(),
                context_quote(context)?,
            ))
        })
        .collect())
}

/// `[{"universe":[…],"tokens":[…]},[{"coin":"@107","markPx":"…",…}]]`.
fn parse_spot(body: &str) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let parts = answer_of(body)?;
    let contexts = parts[1]
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Hyperliquid response".to_owned()))?;
    Ok(contexts
        .iter()
        .filter_map(|context| {
            Some((
                context.get("coin")?.as_str()?.to_owned(),
                context_quote(context)?,
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_market_is_asked_where_its_dex_or_the_spot_pairs_are() {
        assert_eq!(Book::of("BTC"), Book::Perps(None));
        assert_eq!(Book::of("kPEPE"), Book::Perps(None));
        assert_eq!(Book::of("xyz:SP500"), Book::Perps(Some("xyz".to_owned())));
        assert_eq!(Book::of("@107"), Book::Spot);
        assert_eq!(Book::of("PURR/USDC"), Book::Spot);
        assert_eq!(
            Book::of("xyz:SP500").query(),
            json!({"type": "metaAndAssetCtxs", "dex": "xyz"})
        );
        assert_eq!(Book::of("BTC").query(), json!({"type": "metaAndAssetCtxs"}));
        assert_eq!(
            Book::of("@107").query(),
            json!({"type": "spotMetaAndAssetCtxs"})
        );
    }

    const PERPS: &str = r#"[{"universe":[{"name":"BTC","szDecimals":5},{"name":"kPEPE","szDecimals":0},{"name":"ETH","szDecimals":4}],"marginTables":[]},
        [{"funding":"0.0000023748","openInterest":"37261.6","prevDayPx":"62264.0","dayNtlVlm":"5620137934.1","premium":"-0.0001","oraclePx":"58444.0","markPx":"58414.0","midPx":"58431.5","impactPxs":["58431.0","58432.9"],"dayBaseVlm":"93610.18"},
         {"funding":"0.0","openInterest":"0.0","prevDayPx":"0.37621","dayNtlVlm":"0.0","premium":null,"oraclePx":"0.3754","markPx":"0.37621","midPx":null,"impactPxs":null,"dayBaseVlm":"0.0"},
         {"funding":"0.0","openInterest":"0.0","prevDayPx":"1600.0","dayNtlVlm":"1.0","premium":null,"oraclePx":"1.0","markPx":"0","midPx":null,"impactPxs":null,"dayBaseVlm":"0.0"}]]"#;

    #[test]
    fn perpetual_contexts_pair_with_their_names_by_position() {
        let quotes = parse_perps(PERPS).unwrap();
        assert_eq!(quotes.len(), 2, "a market with no price is left out");
        let btc = quotes["BTC"];
        assert_eq!(btc.price, 58_414.0);
        assert_eq!(btc.previous_day_price, Some(62_264.0));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(5_620_137_934.1), Some(VolumeKind::QuoteCurrency))
        );
        // A market nobody traded still has the mark price and a reference.
        assert_eq!(quotes["kPEPE"].price, 0.37621);
        assert!(!quotes.contains_key("ETH"));
    }

    #[test]
    fn spot_contexts_name_their_own_pair() {
        let body = r#"[{"universe":[],"tokens":[]},[
            {"prevDayPx":"0.078016","dayNtlVlm":"1295299.5","markPx":"0.068999","midPx":"0.069054","circulatingSupply":"1","coin":"PURR/USDC","totalSupply":"1","dayBaseVlm":"17785452.0"},
            {"prevDayPx":"65.973","dayNtlVlm":"143487826.8","markPx":"59.051","midPx":"59.053","coin":"@107","dayBaseVlm":"2307734.6"},
            {"prevDayPx":"1.0","dayNtlVlm":"0.0","markPx":"1.0","dayBaseVlm":"0.0"}]]"#;
        let quotes = parse_spot(body).unwrap();
        assert_eq!(quotes.len(), 2, "a context with no coin is left out");
        assert_eq!(quotes["@107"].price, 59.051);
        assert_eq!(quotes["@107"].previous_day_price, Some(65.973));
        assert_eq!(quotes["PURR/USDC"].price, 0.068999);
    }

    #[test]
    fn an_answer_of_another_shape_is_a_failure_not_an_empty_book() {
        for body in [
            "null",
            "[]",
            r#"{"universe":[]}"#,
            r#"[{"universe":[]},{}]"#,
            "not json",
        ] {
            assert!(
                matches!(parse_perps(body), Err(QuoteFailure::Failed(_))),
                "{body}"
            );
        }
        assert!(matches!(parse_spot("[1,2]"), Err(QuoteFailure::Failed(_))));
    }
}
