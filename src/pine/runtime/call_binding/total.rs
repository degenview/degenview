//! Which builtins are *total*: running one cannot fail and changes nothing the script can see, so
//! the session may run it on a bar its branch skips (`hoist`).
//!
//! Whether a call can fail is a fact about its evaluator, not about the family it is dispatched
//! with, so it is decided here and nowhere else:
//!
//! - `math` and the `ta` table (`Route::Math`, `Route::TaTable`) return a bare `RuntimeValue`
//!   (`call_routes.rs`): they have no error to raise. `math.random` is the exception, and not
//!   because it fails: it draws on the session's generator, so running it early would change every
//!   later draw.
//! - The inline and extra `ta` functions and the scalars answer with a `Result`, and some of them
//!   do raise: `runtime.error` always, `ta.pivot_point_levels` for an unknown type (and it fills an
//!   array), `time` and `time_close` for a timeframe they cannot read. These are opt-in, by name:
//!   [`NAMES`] is what has been read and found total, [`REFUSED`] the rest of those families. A
//!   name in neither is not total, and a test makes every name those families answer be in one of
//!   the two, so a builtin added to them has to be decided.
//! - Everything else (collections, strings, drawings, requests, strategy, user methods, anything
//!   unknown) is not total.

use super::{Route, route_of};

/// The inline and extra `ta` functions and the scalars whose evaluators cannot fail.
pub(crate) const NAMES: &[&str] = &[
    // `INLINE`, but `runtime.error`.
    "ta.sar",
    "ta.supertrend",
    "fixnan",
    "ta.tr",
    "ta.barssince",
    "ta.vwap",
    "ta.dmi",
    "ta.cum",
    "ta.correlation",
    "ta.atr",
    "ta.bb",
    "ta.pivothigh",
    "ta.pivotlow",
    "ta.linreg",
    "ta.macd",
    // `extra::FUNCTIONS`, but `ta.pivot_point_levels`.
    "ta.alma",
    "ta.cog",
    "ta.mode",
    "ta.percentile_nearest_rank",
    "ta.percentile_linear_interpolation",
    "ta.valuewhen",
    "ta.mfi",
    "ta.vwma",
    "ta.stoch",
    "ta.wpr",
    "ta.kc",
    "ta.kcw",
    "ta.bbw",
    "ta.tsi",
    "ta.rci",
    // The scalars that only look at their arguments.
    "na",
    "nz",
    "int",
    "float",
    "bool",
];

/// The names of those families that are not total: they raise (`runtime.error`,
/// `ta.pivot_point_levels`, `time`, `time_close`), or have not been read for a failure yet. Only the
/// test that makes every name be decided reads it: a name not in [`NAMES`] is refused either way.
#[cfg(test)]
pub(crate) const REFUSED: &[&str] = &[
    "runtime.error",
    "ta.pivot_point_levels",
    "time",
    "time_close",
    "timestamp",
    "str.tostring",
    "str.format_time",
    "string",
    "color",
    "color.new",
    "color.r",
    "color.g",
    "color.b",
    "color.t",
    "color.rgb",
    "color.from_gradient",
    "timeframe.in_seconds",
    "timeframe.change",
    "timeframe.from_seconds",
    "year",
    "month",
    "dayofmonth",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "weekofyear",
];

/// Whether running the builtin `name` on a bar its branch skips is harmless.
pub fn is_total(name: &str) -> bool {
    match route_of(name) {
        Some(Route::TaTable) => true,
        Some(Route::Math) => name != "math.random",
        Some(Route::TaInline | Route::TaExtra | Route::Scalar) => NAMES.contains(&name),
        _ => false,
    }
}
