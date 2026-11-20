//! Reading candles over the browser's `fetch`.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use degenview_market::Kline;
use degenview_market::Timeframe;
use degenview_market::bucket::FoldPlan;

use crate::error::ChartError;
use crate::exchange::Exchange;

/// The newest candles of a market at a timeframe, and what they were made from.
pub struct Fetched {
    pub plan: FoldPlan,
    /// How many candles of the plan's timeframe one request can fill.
    pub limit: usize,
    /// The timeframe's candles, oldest first: the exchange's own when it serves that length, else
    /// a finer length folded the way the app folds it.
    pub candles: Vec<Kline>,
    /// The candles the exchange answered with, of the plan's base length.
    pub base: Vec<Kline>,
}

/// The newest candles of `symbol` on `exchange` at `timeframe`.
pub async fn fetch_timeframe(
    exchange: Exchange,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Fetched, ChartError> {
    let plan = exchange.plan(timeframe)?;
    let limit = exchange.shown(plan);
    let requested = plan.base_count(limit);
    let url = exchange.candles_url(symbol, plan.base(), requested);
    let body = get_text(exchange, &url).await?;
    let base = exchange.parse_candles(&body, plan.base())?;
    Ok(Fetched {
        plan,
        limit,
        candles: plan.shape(base.clone(), requested, limit),
        base,
    })
}

async fn get_text(exchange: Exchange, url: &str) -> Result<String, ChartError> {
    let window = web_sys::window().ok_or(ChartError::NoWindow)?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(describe)?
        .unchecked_into();
    if !response.ok() {
        return Err(ChartError::Http {
            exchange: exchange.label(),
            status: response.status(),
        });
    }
    let body = JsFuture::from(response.text().map_err(describe)?)
        .await
        .map_err(describe)?;
    Ok(body.as_string().unwrap_or_default())
}

fn describe(value: JsValue) -> ChartError {
    ChartError::Network(value.as_string().unwrap_or_else(|| format!("{value:?}")))
}
