//! The preview chart: the market and timeframe the script is tried on, the candle fetch and
//! refresh, the debounced apply and the stale-output handling. Ported from
//! `ScriptPreviewViewModel`, `ScriptPreviewPane` and `ScriptPreviewMarketPicker`.
//!
//! The preview never raises alerts: it only ever calls `ChartCard::set_pine_source`, which runs
//! the script for drawing, never the alert path.

use super::inputs::{InputValues, InputsStore};
use crate::model::pine::script::ScriptType;
use crate::model::{DataSource, Kline, Timeframe};
use crate::pine::compiler::program::{InputSchema, InputValue};
use crate::service::ProviderError;
use crate::service::work_queue::Pending;
use crate::store::SharedStore;
use crate::ui::ChartCard;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(super) mod chart;
mod fetch;
pub(super) mod market_picker;
mod source;
mod state;

use market_picker::Picker;

const MARKET_KEY: &str = "scriptPreview.market";
const RECENTS_KEY: &str = "scriptPreview.recents";
const TIMEFRAME_KEY: &str = "scriptPreview.timeRange";
const CANDLE_KEY: &str = "scriptPreview.candleCount";
const YZOOM_KEY: &str = "scriptPreview.yZoom";
const RECENTS_LIMIT: usize = 5;
/// Quiet time after the last keystroke before the chart takes the new source.
const APPLY_DELAY: Duration = Duration::from_millis(350);
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

const CANDLE_LIMIT: u32 = 500;

/// A market the preview can chart: crypto and stock sources only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreviewMarket {
    /// The symbol as the chart takes it (a search result's `full_symbol`).
    pub ticker: String,
    pub source: DataSource,
}

impl PreviewMarket {
    pub fn default_market() -> Self {
        Self {
            ticker: "BTCUSDT".to_owned(),
            source: DataSource::Binance,
        }
    }

    /// Exchanges, aggregators, DEX pairs and stocks; not prediction markets (no candles a
    /// script runs on) or the CoinMarketCap indices.
    pub fn is_supported(source: DataSource) -> bool {
        matches!(
            source,
            DataSource::Binance
                | DataSource::Coinbase
                | DataSource::CoinGecko
                | DataSource::DexScreener
                | DataSource::Alpaca
        )
    }

    fn title(&self) -> String {
        self.source.display_symbol(&self.ticker)
    }
}

/// Everything about the preview's run, besides the script's text, that a script can see: the
/// candles (every field, and whether the last one has closed), the market and timeframe, the
/// appearance, the inputs and, for a script that reads it, the visible range. Two identities are
/// equal exactly when running the script again would measure the same thing. It is `Copy` and made
/// without allocating, because the Pine Studio asks for it every frame while the profiler is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunIdentity {
    candles: u64,
    last_is_closed: bool,
    market: u64,
    light: bool,
    inputs: u64,
    visible_range: Option<(i64, i64)>,
}

#[cfg(test)]
impl RunIdentity {
    /// Some identity, for a test that needs a key and not what it describes.
    pub(crate) fn for_test() -> Self {
        Self {
            candles: 0,
            last_is_closed: true,
            market: 0,
            light: false,
            inputs: 0,
            visible_range: None,
        }
    }
}

/// The inputs as one number, whatever order the map holds them in: each entry hashes by itself and
/// the results are added.
fn inputs_fingerprint(inputs: &HashMap<String, InputValue>) -> u64 {
    use std::hash::{Hash, Hasher};
    inputs
        .iter()
        .map(|(name, value)| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            name.hash(&mut hasher);
            match value {
                InputValue::Int(value) => (0u8, *value).hash(&mut hasher),
                InputValue::Float(value) => (1u8, value.to_bits()).hash(&mut hasher),
                InputValue::Bool(value) => (2u8, *value).hash(&mut hasher),
                InputValue::String(value) => (3u8, value).hash(&mut hasher),
                InputValue::Color(value) => (4u8, *value).hash(&mut hasher),
                InputValue::Source(value) => (5u8, value).hash(&mut hasher),
            }
            hasher.finish()
        })
        .fold(0u64, u64::wrapping_add)
}

pub struct PreviewState {
    shared: Option<SharedStore>,
    market: PreviewMarket,
    recents: Vec<PreviewMarket>,
    timeframe: Timeframe,
    candles: Vec<Kline>,
    /// `candles_fingerprint(&candles)`, made when the candles are replaced and not every frame.
    candles_fingerprint: u64,
    loading: bool,
    error: Option<ProviderError>,
    inbox: Option<Pending<Result<Vec<Kline>, ProviderError>>>,
    last_fetch: Option<Instant>,
    /// What the candles on screen were fetched for.
    fetched_for: Option<(PreviewMarket, Timeframe)>,
    card: ChartCard,
    script_id: Option<String>,
    script_type: Option<ScriptType>,
    /// The editor text last seen, and when it last changed.
    latest_source: Option<String>,
    changed_at: Option<Instant>,
    applied_source: Option<String>,
    stale: bool,
    compile_failed: bool,
    drawn_this_frame: bool,
    picker: Option<Picker>,
    saved_zoom: (usize, f64),
    script_title: Option<String>,
    pub(super) input_schema: InputSchema,
    pub(super) input_values: InputValues,
    inputs_store: InputsStore,
    inputs_dirty_at: Option<Instant>,
    /// The chart is waiting for a click on a candle, to stop the debugger at its bar.
    picking_bar: bool,
    /// The candle that was clicked (its open time, ms), until the debugger takes it.
    picked_bar: Option<i64>,
    /// The chart is waiting for a drag over bars, to list their log messages.
    picking_range: bool,
    /// The bars that were dragged over (open times, ms), until the Logs tab takes them.
    picked_range: Option<(i64, i64)>,
    /// The picker asked to open Settings (Stocks need Alpaca keys).
    pending_settings: bool,
}

fn inputs_path() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::store::data_dir().map(|dir| dir.join("script_preview_inputs.json"))
}

/// What the toolbar shows of the preview chart: its market, the last price with the change over
/// what is visible, and the timeframe.
pub(super) struct Summary {
    pub title: String,
    pub price: Option<String>,
    pub timeframe: Timeframe,
}

impl PreviewState {
    /// The chart is on screen this frame (the toolbar's market and timeframe act on it).
    pub(super) fn mark_drawn(&mut self) {
        self.drawn_this_frame = true;
    }

    pub(super) fn summary(&self) -> Summary {
        Summary {
            title: self.market.title(),
            price: self
                .price_line()
                .map(|(price, change)| format!("{price}  {}", signed_percent(change))),
            timeframe: self.timeframe,
        }
    }
}

fn signed_percent(value: f64) -> String {
    let sign = if value >= 0.0 { "+" } else { "−" };
    format!("{sign}{:.2}%", value.abs())
}

#[cfg(test)]
mod tests;
