//! The families of builtins that evaluate a call, as methods the interpreter reaches two ways: the
//! long chain in `Session::call` asks each family in turn, and a call that `call_binding` gave a
//! `Route` goes straight to its family. Both run the same code, so a routed call can only be faster,
//! never different.

use super::ast::{Argument, Route};
use super::runtime_types::RuntimeContext;
use super::runtime_value::RuntimeValue;
use super::session::Session;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};
use super::{alert_builtins, drawings, math, strategy_builtins, strings, ta, visuals};

impl Session {
    /// Evaluates a call that was routed to `route`. `None` means the family does not take it after
    /// all (only the families that can say so, which are the ones that answer `Option` themselves),
    /// and the caller falls back to the chain.
    pub(crate) fn call_routed(
        &mut self,
        route: Route,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        debug_assert_eq!(
            super::call_binding::route_of(name),
            Some(route),
            "{name} is routed to a family that does not own it"
        );
        match route {
            Route::Scalar => self.scalar_call(name, arguments, range, context),
            Route::Collection => self.collection_call(name, arguments, range, context),
            Route::Math => self.math_call(name, arguments, context).map(Some),
            Route::Strings => self.strings_call(name, arguments, range, context).map(Some),
            Route::TaTable => self.ta_table_call(name, arguments, site, context).map(Some),
            Route::TaExtra => self.extra_ta_call(name, arguments, site, range, context),
            Route::TaInline => self
                .inline_call(name, arguments, site, range, context)
                .map(Some),
            Route::StrategyTrade => self
                .strategy_trade_call(name, arguments, range, context)
                .map(Some),
            Route::Input => self.input_call(name, site, range, context).map(Some),
            Route::Drawings => self
                .drawings_call(name, arguments, range, context)
                .map(Some),
            Route::Visuals => self
                .visuals_call(name, arguments, site, range, context)
                .map(Some),
            Route::Strategy => self
                .strategy_call(name, arguments, range, context)
                .map(Some),
            Route::Alert => self
                .alert_call(name, arguments, site, range, context)
                .map(Some),
        }
    }

    pub(crate) fn math_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let values = self.eval_all(arguments, context)?;
        Ok(math::call(name, &values, self.mintick))
    }

    pub(crate) fn strings_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let values = self.eval_all(arguments, context)?;
        strings::call(name, &values, range, self.mintick)
    }

    /// The `ta.*` functions `ta::evaluate` computes from one source and a length.
    pub(crate) fn ta_table_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        site: usize,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let default_source = match name {
            "ta.highest" | "ta.highestbars" => Some("high"),
            "ta.lowest" | "ta.lowestbars" => Some("low"),
            _ => None,
        };
        let uses_default_source = default_source.is_some()
            && arguments.len() == 1
            && !arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("source"));
        let (source, second) = if uses_default_source {
            (
                self.market(default_source.unwrap_or_default(), context)
                    .unwrap_or(RuntimeValue::Na),
                self.arg_value(arguments, 0, Some("length"), context)?,
            )
        } else {
            (
                self.arg_value(arguments, 0, Some("source"), context)?,
                self.arg_value(arguments, 1, Some("length"), context)?,
            )
        };
        let length = second.int_value().unwrap_or(0);
        let key = self.site_key(site, context);
        let state = self.working.ta_states.entry(key).or_default();
        Ok(ta::evaluate(name, &source, &second, length, state))
    }

    pub(crate) fn drawings_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let args = self.eval_args(arguments, context)?;
        drawings::call(
            name,
            &args,
            &mut self.working,
            &context.bar,
            self.bar_seconds,
            &self.drawing_limits,
            range,
        )
    }

    pub(crate) fn visuals_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let args = self.eval_args(arguments, context)?;
        let key = self.site_key(site, context);
        visuals::call(name, &args, key, &mut self.working, range)
    }

    pub(crate) fn strategy_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let args = self.eval_args(arguments, context)?;
        let is_strategy =
            self.program.declaration.script_type == super::program::ScriptType::Strategy;
        strategy_builtins::call(name, &args, &mut self.working, is_strategy, range)
    }

    pub(crate) fn alert_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let args = self.eval_args(arguments, context)?;
        let key = self.site_key(site, context);
        if let Some(event) = alert_builtins::call(
            name,
            &args,
            key,
            &mut self.working,
            &context.flags,
            &mut self.once_per_bar_ledger,
            &self.symbol,
            self.mintick,
            self.bar_seconds,
            &context.bar,
            range,
        )? {
            self.emit(event);
        }
        Ok(RuntimeValue::Void)
    }

    /// The declaration and input builtins are no-ops here (their values are read at compile time).
    pub(crate) fn input_call(
        &mut self,
        name: &str,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        match name {
            "indicator" | "strategy" | "library" => Ok(RuntimeValue::Na),
            _ => self.input_value(site, range, context),
        }
    }

    pub(crate) fn strategy_trade_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let number = self.arg_value(arguments, 0, Some("trade_num"), context)?;
        strategy_builtins::trade_property(
            name,
            number.int_value(),
            &self.working.broker,
            context.bar.close,
        )
        .ok_or_else(|| {
            Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                format!("Unknown or unsupported function '{name}'."),
                range,
            )
        })
    }

    /// The functions `call_binding::INLINE` lists: computed here rather than in a table of their own.
    pub(crate) fn inline_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if matches!(name, "ta.sar" | "ta.supertrend") {
            let values = self.eval_all(arguments, context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_price(name, &values, &context.bar, state));
        }
        if name == "fixnan" {
            let source = self.arg_value(arguments, 0, Some("source"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_fixnan(&source, state));
        }
        if name == "ta.tr" {
            // `ta.tr(handle_na)` reads like the `ta.tr` variable.
            return Ok(self.market("ta.tr", context).unwrap_or(RuntimeValue::Na));
        }
        if name == "runtime.error" {
            let message = self.arg_value(arguments, 0, Some("message"), context)?;
            let text = match message {
                RuntimeValue::String(text) => text,
                _ => "runtime.error()".to_owned(),
            };
            return Err(Diagnostic::error(
                "PINE4030",
                DiagnosticCategory::Runtime,
                text,
                range,
            ));
        }
        if name == "ta.barssince" {
            let condition = self.arg_value(arguments, 0, Some("condition"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_barssince(&condition, state));
        }
        if name == "ta.vwap" {
            let bound = self.bind_named(arguments, &["source", "anchor", "stdev_mult"], context)?;
            let price = bound
                .get("source")
                .and_then(RuntimeValue::number)
                .unwrap_or(context.bar.close);
            let anchor = bound.get("anchor").and_then(RuntimeValue::bool);
            let multiplier = bound.get("stdev_mult").and_then(RuntimeValue::number);
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_vwap(
                price,
                &context.bar,
                anchor,
                multiplier,
                state,
            ));
        }
        if name == "ta.dmi" {
            let di = self.arg_value(arguments, 0, Some("diLength"), context)?;
            let smoothing = self.arg_value(arguments, 1, Some("adxSmoothing"), context)?;
            let key = self.site_key(site, context);
            return Ok(ta::evaluate_dmi(
                &mut self.working.ta_states,
                key,
                &context.bar,
                di.int_value().unwrap_or(0),
                smoothing.int_value().unwrap_or(0),
            ));
        }
        if name == "ta.cum" {
            let source = self.arg_value(arguments, 0, Some("source"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_cum(&source, state));
        }
        if name == "ta.correlation" {
            let first = self.arg_value(arguments, 0, Some("source1"), context)?;
            let second = self.arg_value(arguments, 1, Some("source2"), context)?;
            let length = self.arg_value(arguments, 2, Some("length"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_correlation(
                &first,
                &second,
                length.int_value().unwrap_or(0),
                state,
            ));
        }
        if name == "ta.atr" {
            let length = self.arg_value(arguments, 0, Some("length"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_atr(
                state,
                &context.bar,
                length.int_value().unwrap_or(14),
            ));
        }
        if name == "ta.bb" {
            let source = self.arg_value(arguments, 0, Some("series"), context)?;
            let length = self.arg_value(arguments, 1, Some("length"), context)?;
            let multiplier = self.arg_value(arguments, 2, Some("mult"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_bb(
                &source,
                length.int_value().unwrap_or(0),
                multiplier.number().unwrap_or(2.0),
                state,
            ));
        }
        if matches!(name, "ta.pivothigh" | "ta.pivotlow") {
            let high = name == "ta.pivothigh";
            // `(source, leftbars, rightbars)`, or `(leftbars, rightbars)` reading the bar's own
            // high or low.
            let positional = arguments.iter().filter(|a| a.name.is_none()).count();
            let (source, left, right) = if positional >= 3
                || arguments
                    .iter()
                    .any(|a| a.name.as_deref() == Some("source"))
            {
                (
                    self.arg_value(arguments, 0, Some("source"), context)?,
                    self.arg_value(arguments, 1, Some("leftbars"), context)?,
                    self.arg_value(arguments, 2, Some("rightbars"), context)?,
                )
            } else {
                let own = if high {
                    context.bar.high
                } else {
                    context.bar.low
                };
                (
                    RuntimeValue::Float(own),
                    self.arg_value(arguments, 0, Some("leftbars"), context)?,
                    self.arg_value(arguments, 1, Some("rightbars"), context)?,
                )
            };
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_pivot(
                &source,
                left.int_value().unwrap_or(0),
                right.int_value().unwrap_or(0),
                high,
                state,
            ));
        }
        if name == "ta.linreg" {
            let bound = self.bind_named(arguments, &["source", "length", "offset"], context)?;
            let source = bound.get("source").cloned().unwrap_or(RuntimeValue::Na);
            let length = bound
                .get("length")
                .and_then(RuntimeValue::int_value)
                .unwrap_or(0);
            let offset = bound.get("offset").cloned().unwrap_or(RuntimeValue::Na);
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate(name, &source, &offset, length, state));
        }
        Err(Diagnostic::error(
            "PINE4007",
            DiagnosticCategory::Runtime,
            format!("Unknown or unsupported function '{name}'."),
            range,
        ))
    }
}
