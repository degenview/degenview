//! Indicator readings for the indicator tool: the built-in indicators computed over a chart's
//! own candles with the same functions the chart draws, aligned with the bars returned. Pure
//! values; the app-thread caller supplies the candles.

use crate::mcp::chart_data::{MAX_CANDLES, iso};
use crate::model::Kline;
use crate::model::{
    BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, RSI_PERIOD, SUPERTREND_MULTIPLIER, SUPERTREND_PERIOD,
    bollinger, ema, rsi, supertrend_flips,
};
use schemars::JsonSchema;
use serde::Serialize;

/// EMA periods a reading may ask for. The chart's own menu offers five; a reading is free.
pub const MIN_EMA_PERIOD: usize = 2;
pub const MAX_EMA_PERIOD: usize = 400;

/// The indicators a reading can include. Volume is a column of the candles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Rsi,
    Ema,
    Bollinger,
    TrendFlips,
}

impl Reading {
    pub const ALL: [Self; 4] = [Self::Rsi, Self::Ema, Self::Bollinger, Self::TrendFlips];
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadingRequest {
    pub readings: Vec<Reading>,
    pub ema_period: usize,
    pub limit: usize,
    pub before: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorSeriesOut {
    /// `rsi_14`, `ema_20`, `bb_upper`, `bb_middle`, `bb_lower`, `trend_flip`.
    pub name: String,
    /// What it is and how it is computed.
    pub description: String,
    /// One value per entry of `times`; null while the indicator is still warming up (or, for
    /// `trend_flip`, on a bar with no flip).
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorValuesResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    pub timeframe: String,
    /// The chart's type (`candles`, `heikin_ashi`, `renko`, …). For a type that builds its own
    /// bars (heikin_ashi, renko, kagi, line_break, point_and_figure, range) the readings, times
    /// and closes are those of the built bars the chart draws, not of the market's candles.
    pub chart_type: String,
    /// Bar open times, oldest first, ISO-8601 UTC. Every series has one value per time.
    pub times: Vec<String>,
    pub series: Vec<IndicatorSeriesOut>,
    /// The close of each bar, for reading the indicators against price.
    pub closes: Vec<f64>,
    /// Bars the chart holds in all (up to the replay position); indicators are warm over these.
    pub loaded_bars: usize,
    /// Whether the chart holds bars older than the first returned. Page back with `before`.
    pub has_older_loaded: bool,
    /// Whether the newest returned bar is still forming, so its values can still change.
    pub last_bar_forming: bool,
    pub notes: Vec<String>,
}

/// What the readings belong to.
pub struct ReadingMeta {
    pub workspace_id: u64,
    pub chart_id: u64,
    pub instrument: String,
    pub timeframe: String,
    /// The chart's type slug.
    pub chart_type: String,
    /// Whether `candles` handed to [`indicator_values`] are bars built for that type.
    pub constructed: bool,
    pub replay_ms: Option<i64>,
    pub now_ms: i64,
}

fn aligned<T: Copy>(values: &[Option<T>], start: usize, end: usize) -> Vec<Option<T>> {
    values[start..end].to_vec()
}

pub fn indicator_values(
    meta: &ReadingMeta,
    candles: &[Kline],
    request: &ReadingRequest,
) -> IndicatorValuesResult {
    let limit = request.limit.clamp(1, MAX_CANDLES);
    let end = request.before.map_or(candles.len(), |before| {
        candles.partition_point(|bar| bar.open_time < before)
    });
    let start = end.saturating_sub(limit);
    let mut series = Vec::new();
    for reading in &request.readings {
        match reading {
            Reading::Rsi => series.push(IndicatorSeriesOut {
                name: format!("rsi_{RSI_PERIOD}"),
                description: format!(
                    "Relative strength index of closes, {RSI_PERIOD} bars, Wilder smoothing (0 to 100)"
                ),
                values: aligned(&rsi(candles, RSI_PERIOD), start, end),
            }),
            Reading::Ema => series.push(IndicatorSeriesOut {
                name: format!("ema_{}", request.ema_period),
                description: format!(
                    "Exponential moving average of closes, {} bars, seeded with the simple average of the first {}",
                    request.ema_period, request.ema_period
                ),
                values: aligned(&ema(candles, request.ema_period), start, end),
            }),
            Reading::Bollinger => {
                let (upper, middle, lower) =
                    bollinger(candles, BOLLINGER_PERIOD, BOLLINGER_MULTIPLIER);
                for (name, part, values) in [
                    ("bb_upper", "upper band", upper),
                    ("bb_middle", "middle band (simple average)", middle),
                    ("bb_lower", "lower band", lower),
                ] {
                    series.push(IndicatorSeriesOut {
                        name: name.to_owned(),
                        description: format!(
                            "Bollinger {part}: {BOLLINGER_PERIOD}-bar average of closes, \
                             {BOLLINGER_MULTIPLIER} population standard deviations"
                        ),
                        values: aligned(&values, start, end),
                    });
                }
            }
            Reading::TrendFlips => {
                let flips = supertrend_flips(candles, SUPERTREND_PERIOD, SUPERTREND_MULTIPLIER);
                series.push(IndicatorSeriesOut {
                    name: "trend_flip".to_owned(),
                    description: format!(
                        "Supertrend ({SUPERTREND_PERIOD} bars, multiplier {SUPERTREND_MULTIPLIER}) regime change: \
                         1 a bullish flip, -1 a bearish flip, null no flip. The newest bar's flip is never reported."
                    ),
                    values: flips[start..end]
                        .iter()
                        .map(|flip| flip.map(|bullish| if bullish { 1.0 } else { -1.0 }))
                        .collect(),
                });
            }
        }
    }
    let page = &candles[start..end];
    let mut notes = vec![if meta.constructed {
        format!(
            "Computed with the chart's built-in indicator code over the {} bars the chart \
                 draws (built from the market's candles), as the chart's own indicators are. \
                 They are not the market's candles: times and closes are those of the built bars. \
                 Values are warm at the first bar returned unless the chart holds fewer bars \
                 than the indicator needs.",
            meta.chart_type
        )
    } else {
        "Computed from the market's own candles with the chart's built-in indicator code, over \
             every bar the chart holds, so the values are warm at the first bar returned unless the \
             chart holds fewer bars than the indicator needs."
                .to_owned()
    }];
    if let Some(replay) = meta.replay_ms {
        notes.push(format!(
            "Replay is on: bars after {} are hidden, as on the chart.",
            iso(replay)
        ));
    }
    IndicatorValuesResult {
        workspace_id: meta.workspace_id.to_string(),
        chart_id: meta.chart_id.to_string(),
        instrument: meta.instrument.clone(),
        timeframe: meta.timeframe.clone(),
        chart_type: meta.chart_type.clone(),
        times: page.iter().map(|bar| iso(bar.open_time)).collect(),
        series,
        closes: page.iter().map(|bar| bar.close).collect(),
        loaded_bars: candles.len(),
        has_older_loaded: start > 0,
        last_bar_forming: end == candles.len()
            && page.last().is_some_and(|bar| bar.close_time > meta.now_ms),
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(count: usize) -> Vec<Kline> {
        (0..count)
            .map(|n| {
                let close = 100.0 + (n as f64 * 0.7).sin() * 5.0 + n as f64 * 0.2;
                Kline {
                    open_time: 1_760_918_400_000 + n as i64 * 86_400_000,
                    close_time: 1_760_918_400_000 + (n as i64 + 1) * 86_400_000 - 1,
                    open: close - 0.5,
                    high: close + 1.0,
                    low: close - 1.0,
                    close,
                    volume: 10.0,
                }
            })
            .collect()
    }

    fn meta() -> ReadingMeta {
        ReadingMeta {
            workspace_id: 1,
            chart_id: 2,
            instrument: "BINANCE:BTCUSDT".into(),
            timeframe: "1D".into(),
            chart_type: "candles".into(),
            constructed: false,
            replay_ms: None,
            now_ms: 0,
        }
    }

    fn request(limit: usize, before: Option<i64>) -> ReadingRequest {
        ReadingRequest {
            readings: Reading::ALL.to_vec(),
            ema_period: 20,
            limit,
            before,
        }
    }

    #[test]
    fn every_series_has_one_value_per_returned_bar_and_matches_the_chart_code() {
        let candles = bars(60);
        let result = indicator_values(&meta(), &candles, &request(10, None));
        assert_eq!(result.times.len(), 10);
        assert_eq!(result.closes.len(), 10);
        let names: Vec<_> = result.series.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "rsi_14",
                "ema_20",
                "bb_upper",
                "bb_middle",
                "bb_lower",
                "trend_flip"
            ]
        );
        assert!(result.series.iter().all(|s| s.values.len() == 10));
        // The last values are the chart's own functions over the whole buffer, not a recompute
        // over the page (which would restart the warm-up).
        assert_eq!(
            result.series[0].values.last().copied().flatten(),
            rsi(&candles, RSI_PERIOD).last().copied().flatten()
        );
        assert!(result.series[0].values.iter().all(Option::is_some));
        assert_eq!(result.loaded_bars, 60);
        assert!(result.has_older_loaded);
    }

    #[test]
    fn a_short_history_leaves_the_warm_up_null_and_paging_goes_back() {
        let candles = bars(25);
        let all = indicator_values(&meta(), &candles, &request(25, None));
        let rsi_values = &all.series[0].values;
        assert!(rsi_values[..14].iter().all(Option::is_none));
        assert!(rsi_values[14].is_some());
        assert!(!all.has_older_loaded);

        let before = candles[20].open_time;
        let page = indicator_values(&meta(), &candles, &request(5, Some(before)));
        assert_eq!(
            page.times.first().map(String::as_str),
            Some(iso(candles[15].open_time).as_str())
        );
        assert_eq!(page.times.len(), 5);
        assert!(
            !page.last_bar_forming,
            "a page before the newest bar is closed"
        );
    }

    #[test]
    fn only_the_asked_readings_are_computed_and_replay_is_noted() {
        let candles = bars(30);
        let mut asked = request(5, None);
        asked.readings = vec![Reading::Ema];
        asked.ema_period = 9;
        let mut meta = meta();
        meta.replay_ms = Some(candles[29].open_time);
        let result = indicator_values(&meta, &candles, &asked);
        assert_eq!(result.series.len(), 1);
        assert_eq!(result.series[0].name, "ema_9");
        assert!(
            result
                .notes
                .iter()
                .any(|note| note.contains("Replay is on"))
        );
        let empty = indicator_values(&meta, &[], &asked);
        assert!(empty.times.is_empty());
        assert!(empty.series[0].values.is_empty());
    }
}
