//! `request.footprint()` and the `footprint.*()` / `volume_row.*()` functions that read what it
//! returns. The rows themselves come from `pine::footprint`; this file is the script-facing side:
//! the call's arguments, the intrabar candles asked of the provider, and the handles.
//!
//! A handle is not stored in the run's state. A footprint is the chart bar's index and a row that
//! index with the row's number; the data stays in [`FootprintCache`], a cache of what the candles
//! decide, so a run does not hold (and a realtime rollback does not copy) a footprint per bar and
//! `fp[1]` still reaches the bar before while it is within [`CACHED_BARS`].

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use super::array_builtins;
use super::ast::Argument;
use super::footprint::{self, Footprint};
use super::runtime_support::{MAX_INTRABAR_CANDLES, SecurityKey};
use super::runtime_types::RuntimeContext;
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::Session;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};
use crate::model::Kline;

/// How many of the newest bars keep their footprint. A script reaches back with `fp[n]` within its
/// `max_bars_back`, which is far less; a handle to a bar that has left reads as `na`.
const CACHED_BARS: usize = 500;

/// A volume row's handle is its bar's index shifted up by this many bits, plus the row's number
/// ([`footprint::MAX_ROWS`] fits).
pub const ROW_BITS: u32 = 20;

/// Intrabar sizes the footprint is built from, finest first, in seconds. The provider answers with
/// the coarsest candle size it holds that divides the one asked for.
const INTRABAR_SECONDS: [f64; 5] = [60.0, 300.0, 900.0, 1_800.0, 3_600.0];

const DEFAULT_VALUE_AREA_PERCENT: f64 = 70.0;
const DEFAULT_IMBALANCE_PERCENT: f64 = 300.0;

/// The arguments of the one `request.footprint()` a script may make, as the numbers' bits.
type RequestKey = (i64, u64, u64);

/// The footprints a run has built, by chart bar index, with the chart bar each was built for (the
/// forming bar is built again when it changes).
#[derive(Default)]
pub struct FootprintCache {
    request: Option<RequestKey>,
    bars: BTreeMap<i64, (Kline, Arc<Footprint>)>,
}

impl FootprintCache {
    pub fn clear(&mut self) {
        self.request = None;
        self.bars.clear();
    }

    fn insert(&mut self, bar_index: i64, bar: Kline, footprint: Arc<Footprint>) {
        self.bars.insert(bar_index, (bar, footprint));
        while self.bars.len() > CACHED_BARS {
            self.bars.pop_first();
        }
    }
}

/// Whether `member` is a function of the `namespace` type (`footprint` or `volume_row`).
pub fn handles(namespace: &str, member: &str) -> bool {
    match namespace {
        "footprint" => FOOTPRINT_FUNCTIONS.contains(&member),
        "volume_row" => ROW_FUNCTIONS.contains(&member),
        _ => false,
    }
}

pub const FOOTPRINT_FUNCTIONS: [&str; 9] = [
    "buy_volume",
    "sell_volume",
    "total_volume",
    "delta",
    "poc",
    "vah",
    "val",
    "rows",
    "get_row_by_price",
];

pub const ROW_FUNCTIONS: [&str; 8] = [
    "up_price",
    "down_price",
    "buy_volume",
    "sell_volume",
    "total_volume",
    "delta",
    "has_buy_imbalance",
    "has_sell_imbalance",
];

fn error(code: &str, message: impl Into<String>, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

fn row_handle(bar_index: i64, row: usize) -> RuntimeValue {
    RuntimeValue::Ref(RefKind::VolumeRow, ((bar_index as usize) << ROW_BITS) | row)
}

impl Session {
    /// `request.footprint(ticks_per_row, va_percent, imbalance_percent)`: the footprint of the
    /// current bar, or `na` when the intrabar candles it is built from are not available.
    pub(crate) fn footprint_request(
        &mut self,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        const PARAMETERS: [&str; 3] = ["ticks_per_row", "va_percent", "imbalance_percent"];
        if self.security_depth > 0 {
            return Err(error(
                "PINE4034",
                "request.footprint() is not supported inside another request's expression.",
                range,
            ));
        }
        let bound = self.bind_named(arguments, &PARAMETERS, context)?;
        let ticks = match bound.get("ticks_per_row").and_then(RuntimeValue::int_value) {
            Some(ticks) if ticks > 0 => ticks,
            _ => {
                return Err(error(
                    "PINE4035",
                    "request.footprint() needs `ticks_per_row` to be a positive whole number.",
                    range,
                ));
            }
        };
        let number = |name: &str, default: f64| match bound.get(name) {
            None | Some(RuntimeValue::Na) => Some(default),
            Some(value) => value.number(),
        };
        let value_area = number("va_percent", DEFAULT_VALUE_AREA_PERCENT)
            .filter(|percent| *percent > 0.0 && *percent <= 100.0);
        let Some(value_area) = value_area else {
            return Err(error(
                "PINE4035",
                "request.footprint() needs `va_percent` to be above 0 and at most 100.",
                range,
            ));
        };
        let imbalance = number("imbalance_percent", DEFAULT_IMBALANCE_PERCENT)
            .filter(|percent| percent.is_finite() && *percent > 0.0);
        let Some(imbalance) = imbalance else {
            return Err(error(
                "PINE4035",
                "request.footprint() needs `imbalance_percent` to be above 0.",
                range,
            ));
        };
        // TradingView raises a runtime error when a script depends on more than one footprint request.
        let key = (ticks, value_area.to_bits(), imbalance.to_bits());
        match self.footprints.request {
            Some(first) if first != key => {
                return Err(error(
                    "PINE4033",
                    "A script can make only one request.footprint() with the same arguments each time.",
                    range,
                ));
            }
            _ => self.footprints.request = Some(key),
        }

        let bar_index = self.working.bar_index;
        let bar = context.bar.clone();
        if let Some((built_for, _)) = self.footprints.bars.get(&bar_index)
            && *built_for == bar
        {
            return Ok(RuntimeValue::Ref(RefKind::Footprint, bar_index as usize));
        }
        let Some((candles, inside)) = self.intrabars_of(&bar) else {
            return Ok(RuntimeValue::Na);
        };
        let previous = inside.start.checked_sub(1).map(|index| &candles[index]);
        let Some(footprint) = footprint::build(
            &candles[inside],
            previous,
            self.mintick,
            ticks,
            value_area,
            imbalance,
        ) else {
            return Ok(RuntimeValue::Na);
        };
        self.footprints.insert(bar_index, bar, Arc::new(footprint));
        Ok(RuntimeValue::Ref(RefKind::Footprint, bar_index as usize))
    }

    /// The provider's finer candles, and which of them are inside the chart bar `bar`.
    fn intrabars_of(&mut self, bar: &Kline) -> Option<(Arc<Vec<Kline>>, Range<usize>)> {
        // A price-based chart's bars do not span fixed times.
        if self.fixed_bar_seconds.is_some() || self.bar_seconds <= 0.0 {
            return None;
        }
        let bar_ms = (self.bar_seconds * 1000.0).round() as i64;
        let span_ms = self.last_bar_time? - self.first_bar_time? + bar_ms;
        let seconds = INTRABAR_SECONDS.iter().copied().find(|seconds| {
            *seconds < self.bar_seconds
                && (self.bar_seconds / seconds).fract() == 0.0
                && span_ms as f64 / (seconds * 1000.0) <= MAX_INTRABAR_CANDLES as f64
        })?;
        let candles = self.series(SecurityKey {
            symbol: self.symbol.ticker_id.clone(),
            interval: seconds,
            recent_bars: 0,
        })?;
        let first = candles.partition_point(|candle| candle.open_time < bar.open_time);
        let end = candles.partition_point(|candle| candle.open_time < bar.open_time + bar_ms);
        (first < end).then_some((candles, first..end))
    }

    /// The footprint a handle names, or None when the bar has left the cache.
    fn footprint_of(&self, bar_index: usize) -> Option<Arc<Footprint>> {
        self.footprints
            .bars
            .get(&(bar_index as i64))
            .map(|(_, footprint)| footprint.clone())
    }

    /// `footprint.<member>(id, ...)` and `volume_row.<member>(id, ...)` in function syntax: the id is
    /// the first argument.
    pub(crate) fn footprint_function(
        &mut self,
        namespace: &str,
        member: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let Some((id, rest)) = arguments.split_first() else {
            return Err(error(
                "PINE4036",
                format!("{namespace}.{member}() needs the id of a {namespace}."),
                range,
            ));
        };
        let id = self.eval(&id.value, context)?;
        if id == RuntimeValue::Na {
            return Err(error(
                "PINE4036",
                format!("{namespace}.{member}() needs an id that is not na."),
                range,
            ));
        }
        self.footprint_member(namespace, member, id, rest, range, context)
    }

    /// One of the functions of a footprint or volume row on `id`, with the arguments after the id.
    /// A handle to a bar that has left the cache gives `na`.
    pub(crate) fn footprint_member(
        &mut self,
        namespace: &str,
        member: &str,
        id: RuntimeValue,
        rest: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let unknown = || {
            error(
                "PINE4007",
                format!("Unknown or unsupported function '{namespace}.{member}'."),
                range,
            )
        };
        match (namespace, &id) {
            ("footprint", RuntimeValue::Ref(RefKind::Footprint, bar)) => {
                let bar = *bar;
                if !handles(namespace, member) {
                    return Err(unknown());
                }
                let price = if member == "get_row_by_price" {
                    self.arg_value(rest, 0, Some("price"), context)?.number()
                } else {
                    None
                };
                let Some(footprint) = self.footprint_of(bar) else {
                    return Ok(RuntimeValue::Na);
                };
                let bar_index = bar as i64;
                let row = |index: usize| row_handle(bar_index, index);
                Ok(match member {
                    "buy_volume" => RuntimeValue::Float(footprint.buy_volume()),
                    "sell_volume" => RuntimeValue::Float(footprint.sell_volume()),
                    "total_volume" => RuntimeValue::Float(footprint.total_volume()),
                    "delta" => RuntimeValue::Float(footprint.delta()),
                    "poc" => row(footprint.poc),
                    "vah" => row(footprint.value_area.1),
                    "val" => row(footprint.value_area.0),
                    "rows" => {
                        let rows = (0..footprint.rows.len()).map(row).collect();
                        array_builtins::new_array(&mut self.working, rows)
                    }
                    _ => price
                        .and_then(|price| footprint.row_at_price(price))
                        .map_or(RuntimeValue::Na, row),
                })
            }
            ("volume_row", RuntimeValue::Ref(RefKind::VolumeRow, handle)) => {
                if !handles(namespace, member) {
                    return Err(unknown());
                }
                let bar = handle >> ROW_BITS;
                let index = handle & ((1 << ROW_BITS) - 1);
                let Some(footprint) = self.footprint_of(bar) else {
                    return Ok(RuntimeValue::Na);
                };
                let Some(row) = footprint.rows.get(index) else {
                    return Ok(RuntimeValue::Na);
                };
                Ok(match member {
                    "up_price" => RuntimeValue::Float(row.up),
                    "down_price" => RuntimeValue::Float(row.down),
                    "buy_volume" => RuntimeValue::Float(row.buy),
                    "sell_volume" => RuntimeValue::Float(row.sell),
                    "total_volume" => RuntimeValue::Float(row.total()),
                    "delta" => RuntimeValue::Float(row.delta()),
                    "has_buy_imbalance" => RuntimeValue::Bool(footprint.has_buy_imbalance(index)),
                    _ => RuntimeValue::Bool(footprint.has_sell_imbalance(index)),
                })
            }
            _ => Err(error(
                "PINE4036",
                format!("{namespace}.{member}() needs the id of a {namespace}."),
                range,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_functions_of_each_type_are_the_documented_ones() {
        assert!(handles("footprint", "poc"));
        assert!(handles("footprint", "get_row_by_price"));
        assert!(!handles("footprint", "up_price"));
        assert!(handles("volume_row", "up_price"));
        assert!(handles("volume_row", "has_sell_imbalance"));
        assert!(!handles("volume_row", "poc"));
        assert!(!handles("matrix", "poc"));
    }

    #[test]
    fn a_row_handle_names_its_bar_and_its_row() {
        let RuntimeValue::Ref(RefKind::VolumeRow, handle) = row_handle(7, 42) else {
            panic!("not a row handle");
        };
        assert_eq!(handle >> ROW_BITS, 7);
        assert_eq!(handle & ((1 << ROW_BITS) - 1), 42);
        const { assert!(footprint::MAX_ROWS < 1 << ROW_BITS) };
    }

    #[test]
    fn only_the_newest_bars_keep_their_footprint() {
        let mut cache = FootprintCache::default();
        let bar = Kline {
            open_time: 0,
            close_time: 0,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        };
        let footprint = Arc::new(
            footprint::build(std::slice::from_ref(&bar), None, 1.0, 1, 70.0, 300.0).unwrap(),
        );
        for index in 0..CACHED_BARS as i64 + 10 {
            cache.insert(index, bar.clone(), footprint.clone());
        }
        assert_eq!(cache.bars.len(), CACHED_BARS);
        assert!(!cache.bars.contains_key(&9));
        assert!(cache.bars.contains_key(&10));
    }
}
