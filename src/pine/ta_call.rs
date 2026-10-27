//! Call-site wiring for the `ta.*` functions in [`ta_extra`](super::ta_extra): binds the Pine
//! parameters, finds the call site's state and hands both to the pure implementation.

use super::array_builtins::{Bound, new_array};
use super::ast::Argument;
use super::runtime_types::RuntimeContext;
use super::runtime_value::RuntimeValue;
use super::session::Session;
use super::ta_extra;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};

fn number(bound: &Bound, name: &str, default: f64) -> f64 {
    bound
        .get(name)
        .and_then(RuntimeValue::number)
        .unwrap_or(default)
}

fn length(bound: &Bound, name: &str) -> i64 {
    bound
        .get(name)
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0)
}

/// The parameters of the `ta.*` functions in [`ta_extra`], in Pine's order; None for any other name.
pub fn parameters(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "ta.alma" => &["series", "length", "offset", "sigma", "floor"],
        "ta.cog" | "ta.mode" => &["source", "length"],
        "ta.percentile_nearest_rank" | "ta.percentile_linear_interpolation" => {
            &["source", "length", "percentage"]
        }
        "ta.valuewhen" => &["condition", "source", "occurrence"],
        "ta.mfi" => &["series", "length"],
        "ta.vwma" => &["source", "length"],
        "ta.stoch" => &["source", "high", "low", "length"],
        "ta.wpr" => &["length"],
        "ta.kc" | "ta.kcw" => &["series", "length", "mult", "useTrueRange"],
        "ta.bbw" => &["series", "length", "mult"],
        "ta.tsi" => &["source", "short_length", "long_length"],
        "ta.rci" => &["source", "length"],
        "ta.pivot_point_levels" => &["type", "anchor", "developing"],
        _ => return None,
    })
}

impl Session {
    /// `ta.alma`, `ta.mfi`, `ta.valuewhen`, … None when `name` is not one of them.
    pub(crate) fn extra_ta_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some(parameters) = parameters(name) else {
            return Ok(None);
        };
        let bound = self.bind_named(arguments, parameters, context)?;
        let na = RuntimeValue::Na;
        let source = |key: &str| bound.get(key).unwrap_or(&na).clone();
        let key = self.site_key(site, context);
        let bar = context.bar.clone();
        let state = self.working.ta_states.entry(key).or_default();
        let value = match name {
            "ta.alma" => ta_extra::alma(
                &source("series"),
                length(&bound, "length"),
                number(&bound, "offset", 0.85),
                number(&bound, "sigma", 6.0),
                bound
                    .get("floor")
                    .and_then(RuntimeValue::bool)
                    .unwrap_or(false),
                state,
            ),
            "ta.cog" => ta_extra::cog(&source("source"), length(&bound, "length"), state),
            "ta.mode" => ta_extra::mode(&source("source"), length(&bound, "length"), state),
            "ta.percentile_nearest_rank" | "ta.percentile_linear_interpolation" => {
                ta_extra::percentile(
                    &source("source"),
                    length(&bound, "length"),
                    number(&bound, "percentage", f64::NAN),
                    name == "ta.percentile_linear_interpolation",
                    state,
                )
            }
            "ta.valuewhen" => ta_extra::valuewhen(
                &source("condition"),
                &source("source"),
                length(&bound, "occurrence"),
                state,
            ),
            "ta.mfi" => ta_extra::mfi(&source("series"), length(&bound, "length"), &bar, state),
            "ta.vwma" => ta_extra::vwma(&source("source"), length(&bound, "length"), &bar, state),
            "ta.stoch" => ta_extra::stoch(
                &source("source"),
                &source("high"),
                &source("low"),
                length(&bound, "length"),
                state,
            ),
            "ta.wpr" => ta_extra::wpr(length(&bound, "length"), &bar, state),
            "ta.kc" => {
                let (basis, upper, lower) = ta_extra::keltner(
                    &source("series"),
                    length(&bound, "length"),
                    number(&bound, "mult", f64::NAN),
                    bound
                        .get("useTrueRange")
                        .and_then(RuntimeValue::bool)
                        .unwrap_or(true),
                    &bar,
                    state,
                );
                let float =
                    |value: Option<f64>| value.map_or(RuntimeValue::Na, RuntimeValue::Float);
                RuntimeValue::Tuple(vec![float(basis), float(upper), float(lower)])
            }
            "ta.kcw" => ta_extra::keltner_width(
                &source("series"),
                length(&bound, "length"),
                number(&bound, "mult", f64::NAN),
                bound
                    .get("useTrueRange")
                    .and_then(RuntimeValue::bool)
                    .unwrap_or(true),
                &bar,
                state,
            ),
            "ta.bbw" => ta_extra::bollinger_width(
                &source("series"),
                length(&bound, "length"),
                number(&bound, "mult", 2.0),
                state,
            ),
            "ta.rci" => ta_extra::rci(&source("source"), length(&bound, "length"), state),
            "ta.pivot_point_levels" => {
                let kind = bound
                    .get("type")
                    .and_then(RuntimeValue::text_value)
                    .unwrap_or("");
                let flag = |key: &str| bound.get(key).and_then(RuntimeValue::bool).unwrap_or(false);
                let levels = ta_extra::pivot_point_levels(
                    kind,
                    flag("anchor"),
                    flag("developing"),
                    &bar,
                    state,
                );
                let Some(levels) = levels else {
                    return Err(Diagnostic::error(
                        "PINE4035",
                        DiagnosticCategory::Runtime,
                        format!(
                            "ta.pivot_point_levels: the type must be one of {}, not '{kind}'.",
                            ta_extra::PIVOT_TYPES.join(", ")
                        ),
                        range,
                    ));
                };
                let values = levels
                    .iter()
                    .map(|level| level.map_or(RuntimeValue::Na, RuntimeValue::Float))
                    .collect();
                new_array(&mut self.working, values)
            }
            _ => ta_extra::tsi(
                &source("source"),
                length(&bound, "short_length"),
                length(&bound, "long_length"),
                state,
            ),
        };
        Ok(Some(value))
    }

    /// `ta.obv`, `ta.accdist`, … read as variables. None when `name` is not one of them.
    pub(crate) fn cumulative_variable(
        &mut self,
        name: &str,
        context: &RuntimeContext,
    ) -> Option<RuntimeValue> {
        let slot = ta_extra::VARIABLES
            .iter()
            .position(|candidate| *candidate == name)?;
        let key = usize::MAX - slot;
        // The state leaves the map while it reads the committed history out of `working`.
        let mut state = self.working.ta_states.remove(&key).unwrap_or_default();
        let value = ta_extra::cumulative(name, &context.bar, &self.working, &mut state);
        self.working.ta_states.insert(key, state);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_function_ta_extra_lists_has_parameters_and_the_reverse() {
        for name in ta_extra::FUNCTIONS {
            assert!(parameters(name).is_some(), "{name}");
        }
        let probes = [
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
            "ta.pivot_point_levels",
        ];
        for name in probes {
            assert!(ta_extra::FUNCTIONS.contains(&name), "{name}");
        }
        assert_eq!(probes.len(), ta_extra::FUNCTIONS.len());
    }
}
