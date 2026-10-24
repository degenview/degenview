//! `request.security` and `request.security_lower_tf`, ported from
//! `PineRuntimeSession+Security.swift` + `+ForeignSecurity.swift`. These are the one Pine builtin
//! family that is not a pure `RuntimeState` handler: the `expression` argument is kept unevaluated
//! and runs against a higher (or foreign, or finer) series by swapping `working` for the call site's
//! own `SecuritySite.state` and calling `Session::eval`. The `SecurityDataProvider` supplies the
//! other symbol's / finer-timeframe candles; the chart's own higher timeframe is folded from the
//! chart's bars.

use std::collections::HashMap;
use std::sync::Arc;

use super::array_builtins;
use super::ast::{Argument, Expression, Statement};
use super::calendar;
use super::logs::LogRequest;
use super::runtime_support::{ChartSpan, SecurityDataProvider, SecurityKey};
use super::runtime_types::{BarFlags, RuntimeContext};
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::Session;
use super::state::{RuntimeState, SecuritySite};
use super::time;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};
use crate::model::Kline;

const MILLIS_PER_DAY: i64 = 86_400_000;

/// How many `request.*` expressions may contain one another.
const SECURITY_NESTING_LIMIT: usize = 3;

/// One `request.security` call's resolved arguments. `foreign` holds another symbol's series; it is
/// None for the chart's own symbol.
struct SecurityRequest {
    /// The symbol asked for, as the script wrote it (empty for the chart's own).
    symbol: String,
    interval: f64,
    expression: Expression,
    lookahead_on: bool,
    gaps_on: bool,
    foreign: Option<Arc<Vec<Kline>>>,
}

// MARK: Entry points

impl Session {
    /// A request inside another request's expression (`dynamic_requests`) runs against the outer
    /// series; the depth is bounded so a function that requests itself cannot run away.
    fn check_nesting(&self, name: &str, range: SourceRange) -> Result<(), Diagnostic> {
        if self.security_depth >= SECURITY_NESTING_LIMIT {
            return Err(Diagnostic::error(
                "PINE4023",
                DiagnosticCategory::Runtime,
                format!("{name} is nested more than {SECURITY_NESTING_LIMIT} requests deep."),
                range,
            ));
        }
        Ok(())
    }

    /// `request.security(symbol, timeframe, expression, ...)`.
    pub(crate) fn security_call(
        &mut self,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        self.check_nesting("request.security", range)?;
        let Some(request) = self.security_request(arguments, context, range)? else {
            // An invalid symbol/timeframe the script asked to ignore: na, shaped like the expression.
            return Ok(RuntimeValue::Na);
        };
        if let Some(candles) = request.foreign.clone() {
            return self.serve_foreign(&request, &candles, site, context);
        }
        if request.interval < self.bar_seconds {
            return self.serve_finer_security(&request, site, range, context);
        }
        self.serve_security(&request, site, context)
    }

    /// `request.security` to a timeframe finer than the chart's: the expression runs on the chart
    /// bar's intrabars and the value of the last one stands for the bar, as in Pine.
    fn serve_finer_security(
        &mut self,
        request: &SecurityRequest,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let Some(candles) = self.lower_timeframe_candles(None, request.interval, 0) else {
            return Err(Diagnostic::error(
                "PINE4021",
                DiagnosticCategory::Runtime,
                "request.security to a finer timeframe needs that timeframe's candles for this symbol, and none are available.",
                range,
            ));
        };
        let intrabars = self.serve_intrabars(
            &candles,
            request.interval,
            &request.expression,
            site,
            context,
        )?;
        Ok(self.last_intrabar(intrabars))
    }

    /// How many values `expression` yields when it is a tuple: a literal `[a, b]`, or a call of a
    /// script function whose last statement is one. None for a single value or when unknown.
    fn tuple_arity(&self, expression: &Expression, context: &RuntimeContext) -> Option<usize> {
        match expression {
            Expression::Tuple(items, _) => Some(items.len()),
            Expression::Call { name, range, .. } => {
                let function = self.scoped_function(name, *range, context).ok().flatten()?;
                match function.body.last()? {
                    Statement::Expression(Expression::Tuple(items, _)) => Some(items.len()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The last element of the array (or of each array of a tuple) `serve_intrabars` returned.
    fn last_intrabar(&self, intrabars: RuntimeValue) -> RuntimeValue {
        match intrabars {
            RuntimeValue::Tuple(items) => RuntimeValue::Tuple(
                items
                    .into_iter()
                    .map(|item| self.last_intrabar(item))
                    .collect(),
            ),
            RuntimeValue::Ref(RefKind::Array, id) => self
                .working
                .arrays
                .get(&id)
                .and_then(|values| values.last())
                .cloned()
                .unwrap_or(RuntimeValue::Na),
            other => other,
        }
    }

    /// `request.security_lower_tf(symbol, timeframe, expression, ...)`.
    pub(crate) fn security_lower_tf_call(
        &mut self,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        self.check_nesting("request.security_lower_tf", range)?;
        const PARAMETERS: [&str; 7] = [
            "symbol",
            "timeframe",
            "expression",
            "ignore_invalid_symbol",
            "currency",
            "ignore_invalid_timeframe",
            "calc_bars_count",
        ];
        let (values, expression) = self.security_arguments(arguments, &PARAMETERS, context)?;
        let Some(expression) = expression else {
            return Err(Diagnostic::error(
                "PINE4024",
                DiagnosticCategory::Runtime,
                "request.security_lower_tf needs a symbol, a timeframe and an expression.",
                range,
            ));
        };
        if let Some(RuntimeValue::String(text)) = values.get("symbol") {
            let foreign = !(text.is_empty() || *text == self.symbol.ticker_id);
            if foreign
                && self.security_data.is_none()
                && values.get("ignore_invalid_symbol") != Some(&RuntimeValue::Bool(true))
            {
                return Err(self.unserved_symbol(text, range));
            }
        }
        let seconds = values
            .get("timeframe")
            .and_then(RuntimeValue::text_value)
            .and_then(time::seconds_of_timeframe);
        let recent_bars = values
            .get("calc_bars_count")
            .and_then(RuntimeValue::int_value)
            .unwrap_or(0)
            .max(0) as usize;
        let usable = seconds.is_some_and(|seconds| seconds <= self.bar_seconds);
        if !usable
            && self.bar_seconds > 0.0
            && values.get("ignore_invalid_timeframe") != Some(&RuntimeValue::Bool(true))
        {
            return Err(Diagnostic::error(
                "PINE4021",
                DiagnosticCategory::Runtime,
                "request.security_lower_tf needs a timeframe no longer than the chart's.",
                range,
            ));
        }
        // `calc_bars_count` limits the request to the newest chart bars: older ones get no intrabars.
        let outside_window = recent_bars > 0
            && self.working.bar_index < self.last_bar_index - recent_bars as i64 + 1;
        if outside_window {
            let columns = self.tuple_arity(&expression, context);
            return Ok(empty_shaped(columns, self));
        }
        if let Some(seconds) = seconds
            && seconds < self.bar_seconds
            && let Some(candles) =
                self.lower_timeframe_candles(values.get("symbol"), seconds, recent_bars)
        {
            return self.serve_intrabars(&candles, seconds, &expression, site, context);
        }
        // The chart's own timeframe: each chart bar is its own single intrabar.
        let mut intrabars: Option<RuntimeValue> = None;
        if let Some(seconds) = seconds
            && seconds == self.bar_seconds
        {
            intrabars = Some(self.serve_security(
                &SecurityRequest {
                    symbol: String::new(),
                    interval: seconds,
                    expression: expression.clone(),
                    lookahead_on: false,
                    gaps_on: false,
                    foreign: None,
                },
                site,
                context,
            )?);
        }
        let columns = match &intrabars {
            Some(RuntimeValue::Tuple(values)) => Some(values.len()),
            _ => self.tuple_arity(&expression, context),
        };
        if let Some(columns) = columns {
            let Some(RuntimeValue::Tuple(values)) = intrabars else {
                return Ok(RuntimeValue::Tuple(
                    (0..columns).map(|_| self.make_array(Vec::new())).collect(),
                ));
            };
            return Ok(RuntimeValue::Tuple(
                values
                    .into_iter()
                    .map(|value| self.make_array(vec![value]))
                    .collect(),
            ));
        }
        Ok(self.make_array(intrabars.map_or(Vec::new(), |value| vec![value])))
    }
}

impl Session {
    /// Reads the call's arguments; `expression` stays unevaluated. None when the symbol or timeframe
    /// cannot be served and the script passed `ignore_invalid_*`.
    /// Whether `symbol` names the series the chart itself runs on. On a price-based chart the
    /// chart's bars are constructed ones, so even its own symbol is requested from the provider:
    /// `request.security()` on a standard ticker gives the real market data there.
    fn is_chart_series(&self, symbol: &str) -> bool {
        self.symbol.chart.is_standard() && (symbol.is_empty() || symbol == self.symbol.ticker_id)
    }

    fn security_request(
        &mut self,
        arguments: &[Argument],
        context: &mut RuntimeContext,
        range: SourceRange,
    ) -> Result<Option<SecurityRequest>, Diagnostic> {
        const PARAMETERS: [&str; 9] = [
            "symbol",
            "timeframe",
            "expression",
            "gaps",
            "lookahead",
            "ignore_invalid_symbol",
            "currency",
            "ignore_invalid_timeframe",
            "calc_bars_count",
        ];
        let (values, expression) = self.security_arguments(arguments, &PARAMETERS, context)?;
        let Some(expression) = expression else {
            return Err(Diagnostic::error(
                "PINE4024",
                DiagnosticCategory::Runtime,
                "request.security needs a symbol, a timeframe and an expression.",
                range,
            ));
        };
        let ignore_symbol = values.get("ignore_invalid_symbol") == Some(&RuntimeValue::Bool(true));
        let symbol_name = match values.get("symbol") {
            Some(RuntimeValue::String(name)) => name.clone(),
            _ => {
                if ignore_symbol {
                    return Ok(None);
                }
                return Err(self.unserved_symbol("", range));
            }
        };
        let symbol_name = if symbol_name.is_empty() {
            self.symbol.ticker_id.clone()
        } else {
            symbol_name
        };
        let is_chart_symbol = self.is_chart_series(&symbol_name);
        if !is_chart_symbol && self.security_data.is_none() {
            if ignore_symbol {
                return Ok(None);
            }
            return Err(self.unserved_symbol(&symbol_name, range));
        }
        let interval = match self.security_interval(values.get("timeframe"), range) {
            Ok(interval) => interval,
            Err(error) => {
                if values.get("ignore_invalid_timeframe") == Some(&RuntimeValue::Bool(true)) {
                    return Ok(None);
                }
                return Err(error);
            }
        };
        let foreign = if !is_chart_symbol {
            let key = SecurityKey {
                symbol: symbol_name.clone(),
                interval,
                recent_bars: 0,
            };
            match self.series(key) {
                Some(candles) => Some(candles),
                None => {
                    if ignore_symbol {
                        return Ok(None);
                    }
                    return Err(self.unserved_symbol(&symbol_name, range));
                }
            }
        } else {
            None
        };
        Ok(Some(SecurityRequest {
            symbol: if is_chart_symbol {
                String::new()
            } else {
                symbol_name
            },
            interval,
            expression,
            lookahead_on: values.get("lookahead").and_then(RuntimeValue::text_value)
                == Some("barmerge.lookahead_on"),
            gaps_on: values.get("gaps").and_then(RuntimeValue::text_value)
                == Some("barmerge.gaps_on"),
            foreign,
        }))
    }

    /// The provider's candles for `key`, asked once per run.
    fn series(&mut self, key: SecurityKey) -> Option<Arc<Vec<Kline>>> {
        if let Some(known) = self.security_series.get(&key) {
            return known.clone();
        }
        let span = match (self.first_bar_time, self.last_bar_time) {
            (Some(start_ms), Some(last)) if self.bar_seconds > 0.0 => Some(ChartSpan {
                start_ms,
                end_ms: last + (self.bar_seconds * 1000.0).round() as i64,
                spacing_ms: (self.bar_seconds * 1000.0).round() as i64,
            }),
            _ => None,
        };
        // Waiting for the provider is not the script's running time.
        let asked = std::time::Instant::now();
        let answer = self
            .security_data
            .as_ref()
            .and_then(|provider| match span {
                Some(span) => provider.candles_over(&key, span),
                None => provider.candles(&key),
            })
            .map(Arc::new);
        if let Some(deadline) = self.deadline_at.as_mut() {
            *deadline += asked.elapsed();
        }
        self.security_series.insert(key, answer.clone());
        answer
    }

    fn unserved_symbol(&self, name: &str, range: SourceRange) -> Diagnostic {
        Diagnostic::error(
            "PINE4022",
            DiagnosticCategory::Runtime,
            if name.is_empty() || self.security_data.is_none() {
                "request.security only supports the chart's own symbol (syminfo.tickerid) here."
                    .to_owned()
            } else {
                format!("No data is available for symbol '{name}'.")
            },
            range,
        )
    }

    fn security_interval(
        &self,
        value: Option<&RuntimeValue>,
        range: SourceRange,
    ) -> Result<f64, Diagnostic> {
        let interval = match value {
            Some(RuntimeValue::String(text)) if !text.is_empty() => {
                time::seconds_of_timeframe(text).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4021",
                        DiagnosticCategory::Runtime,
                        "request.security needs a timeframe such as \"60\" or \"1D\".",
                        range,
                    )
                })?
            }
            _ => self.bar_seconds,
        };
        if interval <= 0.0 {
            return Err(Diagnostic::error(
                "PINE4021",
                DiagnosticCategory::Runtime,
                "The chart's bar length is not known yet.",
                range,
            ));
        }
        Ok(interval)
    }

    /// The call's arguments by parameter name; the `expression` stays unevaluated.
    fn security_arguments(
        &mut self,
        arguments: &[Argument],
        parameters: &[&str],
        context: &mut RuntimeContext,
    ) -> Result<(HashMap<String, RuntimeValue>, Option<Expression>), Diagnostic> {
        let mut values = HashMap::new();
        let mut expression = None;
        let mut position = 0usize;
        for argument in arguments {
            let name = argument.name.as_deref().or_else(|| {
                let name = parameters.get(position).copied();
                position += 1;
                name
            });
            let Some(name) = name else { continue };
            if name == "expression" {
                expression = Some(argument.value.clone());
            } else {
                values.insert(name.to_owned(), self.eval(&argument.value, context)?);
            }
        }
        Ok((values, expression))
    }

    /// Evaluates the candles the provider holds for the chart's own symbol that closed before the
    /// chart's first bar, so the expression starts with the history TradingView would give it.
    fn warm_up(
        &mut self,
        site: &mut SecuritySite,
        request: &SecurityRequest,
        before: i64,
    ) -> Result<(), Diagnostic> {
        site.is_warmed_up = true;
        let key = SecurityKey {
            symbol: self.symbol.ticker_id.clone(),
            interval: request.interval,
            recent_bars: 0,
        };
        let Some(candles) = self.series(key) else {
            return Ok(());
        };
        for candle in candles.iter() {
            if bucket_end(candle.open_time, request.interval) > before {
                break;
            }
            site.last_result = self.evaluate_higher_timeframe(site, candle, request, true)?;
        }
        Ok(())
    }

    /// The expression on the higher-timeframe bar the chart bar belongs to, through the call site's
    /// own state.
    fn serve_security(
        &mut self,
        request: &SecurityRequest,
        site: usize,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let key = self.site_key(site, context);
        let mut state = self.working.securities.remove(&key).unwrap_or_default();
        let bar = context.bar.clone();
        if state.processed_bar == Some(bar.open_time) {
            let result = state.bar_result.clone();
            self.working.securities.insert(key, state);
            return Ok(result);
        }
        if !state.is_warmed_up {
            self.warm_up(&mut state, request, bar.open_time)?;
        }
        let start = bucket_start(bar.open_time, request.interval);
        let mut fresh = false;
        if state.bucket != Some(start) {
            // The chart skipped the bucket's last bar, so complete it now.
            if state.has_pending_bars
                && let Some(pending) = state.accumulated.clone()
            {
                state.last_result =
                    self.evaluate_higher_timeframe(&mut state, &pending, request, true)?;
                fresh = true;
            }
            state.bucket = Some(start);
            state.accumulated = None;
            state.has_pending_bars = false;
        }
        let mut developing = Vec::new();
        if let Some(accumulated) = &state.accumulated {
            developing.push(accumulated.clone());
        }
        developing.push(bar.clone());
        let developing = fold_one(&developing, request.interval);
        let end = bucket_end(start, request.interval);
        let closes_bucket = bar.open_time + (self.bar_seconds * 1000.0).round() as i64 >= end;
        let confirmed = context.flags.is_confirmed;

        let result;
        if closes_bucket && confirmed {
            result = self.evaluate_higher_timeframe(&mut state, &developing, request, true)?;
            state.last_result = result.clone();
            state.accumulated = None;
            state.has_pending_bars = false;
            fresh = true;
        } else {
            if confirmed {
                state.accumulated = Some(developing.clone());
                state.has_pending_bars = true;
            }
            if request.lookahead_on || context.flags.is_realtime {
                result = self.evaluate_higher_timeframe(&mut state, &developing, request, false)?;
            } else {
                result = state.last_result.clone();
            }
        }
        let result = if request.gaps_on && !fresh {
            RuntimeValue::Na
        } else {
            result
        };
        state.processed_bar = Some(bar.open_time);
        state.bar_result = result.clone();
        self.working.securities.insert(key, state);
        Ok(result)
    }

    /// `request.security` for a symbol the chart does not carry, from the provider's snapshot.
    fn serve_foreign(
        &mut self,
        request: &SecurityRequest,
        candles: &[Kline],
        site: usize,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let key = self.site_key(site, context);
        let mut state = self.working.securities.remove(&key).unwrap_or_default();
        let bar = context.bar.clone();
        if state.processed_bar == Some(bar.open_time) {
            let result = state.bar_result.clone();
            self.working.securities.insert(key, state);
            return Ok(result);
        }
        let mut fresh = false;
        if context.flags.is_confirmed {
            let chart_close = bar.open_time + (self.bar_seconds * 1000.0).round() as i64;
            while state.completed_candles < candles.len() {
                let candle = &candles[state.completed_candles];
                if bucket_end(candle.open_time, request.interval) > chart_close {
                    break;
                }
                state.last_result =
                    self.evaluate_higher_timeframe(&mut state, candle, request, true)?;
                state.completed_candles += 1;
                fresh = true;
            }
        }
        let mut result = state.last_result.clone();
        if (request.lookahead_on || context.flags.is_realtime)
            && let Some(index) = last_index(bar.open_time, candles)
            && index >= state.completed_candles
        {
            result = self.evaluate_higher_timeframe(&mut state, &candles[index], request, false)?;
        }
        if request.gaps_on && !fresh {
            result = RuntimeValue::Na;
        }
        state.processed_bar = Some(bar.open_time);
        state.bar_result = result.clone();
        self.working.securities.insert(key, state);
        Ok(result)
    }

    /// The provider's intrabar candles of the chart's own symbol at `seconds`.
    fn lower_timeframe_candles(
        &mut self,
        symbol_value: Option<&RuntimeValue>,
        seconds: f64,
        recent_bars: usize,
    ) -> Option<Arc<Vec<Kline>>> {
        let symbol = match symbol_value {
            Some(RuntimeValue::String(text)) if !text.is_empty() => text.clone(),
            _ => self.symbol.ticker_id.clone(),
        };
        let key = SecurityKey {
            symbol,
            interval: seconds,
            recent_bars,
        };
        // Asked of the provider once per run, not once per chart bar.
        self.series(key).filter(|candles| !candles.is_empty())
    }

    /// The expression on every intrabar that opens inside the chart bar.
    fn serve_intrabars(
        &mut self,
        candles: &[Kline],
        interval: f64,
        expression: &Expression,
        site: usize,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let key = self.site_key(site, context);
        let mut state = self.working.securities.remove(&key).unwrap_or_default();
        let bar = context.bar.clone();
        if state.processed_bar == Some(bar.open_time) {
            let result = state.bar_result.clone();
            self.working.securities.insert(key, state);
            return Ok(result);
        }
        let end = bar.open_time + (self.bar_seconds * 1000.0).round() as i64;
        let mut low = 0usize;
        let mut high = candles.len();
        while low < high {
            let middle = (low + high) / 2;
            if candles[middle].open_time < bar.open_time {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let request = SecurityRequest {
            symbol: String::new(),
            interval,
            expression: expression.clone(),
            lookahead_on: false,
            gaps_on: false,
            foreign: None,
        };
        let mut results = Vec::new();
        let mut index = low;
        while index < candles.len() && candles[index].open_time < end {
            results.push(self.evaluate_higher_timeframe(
                &mut state,
                &candles[index],
                &request,
                true,
            )?);
            index += 1;
        }
        // One array per column when the expression yields a tuple: a literal `[a, b]`, a function that
        // returns one, or whatever the intrabars turned out to produce.
        let columns = self
            .tuple_arity(expression, context)
            .or_else(|| match results.first() {
                Some(RuntimeValue::Tuple(parts)) => Some(parts.len()),
                _ => None,
            });
        let value = if let Some(columns) = columns {
            RuntimeValue::Tuple(
                (0..columns)
                    .map(|column| {
                        self.make_array(
                            results
                                .iter()
                                .map(|result| match result {
                                    RuntimeValue::Tuple(parts) if column < parts.len() => {
                                        parts[column].clone()
                                    }
                                    _ => RuntimeValue::Na,
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )
        } else {
            self.make_array(results)
        };
        state.processed_bar = Some(bar.open_time);
        state.bar_result = value.clone();
        self.working.securities.insert(key, state);
        Ok(value)
    }

    /// Runs the expression on `candle` in the site's own state. A commit keeps that state and
    /// appends the candle to its histories; otherwise the state is thrown away, like a realtime tick.
    fn evaluate_higher_timeframe(
        &mut self,
        site: &mut SecuritySite,
        candle: &Kline,
        request: &SecurityRequest,
        commit: bool,
    ) -> Result<RuntimeValue, Diagnostic> {
        let chart_bar_seconds = self.bar_seconds;
        self.security_depth += 1;
        // The chart's state steps aside and the site's takes its place. A commit moves the site's
        // state in and back out; copying it for every call would make a run quadratic in its length.
        let site_state = if commit {
            std::mem::take(&mut *site.state)
        } else {
            (*site.state).clone()
        };
        let mut chart = std::mem::replace(&mut self.working, site_state);
        // The chart's variables stay readable from the expression, without a copy of the table.
        let outer_globals = self
            .security_globals
            .replace(std::mem::take(&mut chart.variables));
        self.working.instructions = 0;
        self.working.bar_index += 1;
        self.bar_seconds = request.interval;

        let flags = BarFlags {
            is_first: self.working.bar_index == 0,
            is_last: false,
            is_history: commit,
            is_realtime: !commit,
            is_new: true,
            is_confirmed: commit,
            is_last_confirmed_history: false,
        };
        let mut security_context = RuntimeContext::new(candle.clone(), flags);
        let requested = LogRequest {
            symbol: if request.symbol.is_empty() {
                self.symbol.ticker_id.clone()
            } else {
                request.symbol.clone()
            },
            interval_seconds: request.interval,
        };
        let debugging = self.debug_is_attached();
        if debugging {
            self.debug_enter_security(&requested.symbol, requested.interval_seconds);
        }
        self.logs.enter_request(requested, candle.open_time, commit);
        let evaluated = self.eval(&request.expression, &mut security_context);
        self.logs.leave_request();
        if debugging {
            self.debug_leave();
        }
        // A returned collection or object lives in the site's state; the chart gets a copy of its own.
        let value = evaluated
            .map(|evaluated| ReferenceExport::new(&self.working).export(evaluated, &mut chart));
        if commit {
            if value.is_ok() {
                self.commit_histories(candle.clone(), false);
            }
            *site.state = std::mem::take(&mut self.working);
        }
        chart.variables =
            std::mem::replace(&mut self.security_globals, outer_globals).unwrap_or_default();
        self.working = chart;
        self.bar_seconds = chart_bar_seconds;
        self.security_depth -= 1;
        value
    }

    /// A fresh array holding `items`.
    fn make_array(&mut self, items: Vec<RuntimeValue>) -> RuntimeValue {
        array_builtins::new_array(&mut self.working, items)
    }
}

/// A tuple of `columns` empty arrays (or one empty array) for the expression shape when a timeframe
/// cannot be served.
fn empty_shaped(columns: Option<usize>, session: &mut Session) -> RuntimeValue {
    match columns {
        Some(columns) => RuntimeValue::Tuple(
            (0..columns)
                .map(|_| array_builtins::new_array(&mut session.working, Vec::new()))
                .collect(),
        ),
        None => array_builtins::new_array(&mut session.working, Vec::new()),
    }
}

/// The start of the calendar bucket that contains `open_time`, for a bar length in seconds.
pub(super) fn bucket_start(open_time: i64, interval_seconds: f64) -> i64 {
    if interval_seconds < 604_800.0 {
        let ms = (interval_seconds * 1000.0).round() as i64;
        return open_time.div_euclid(ms) * ms;
    }
    let components = calendar::components(open_time);
    if interval_seconds < 2_419_200.0 {
        // A week: Monday 00:00 UTC (Pine's `dayofweek` is Sunday = 1 … Saturday = 7).
        let days = open_time.div_euclid(MILLIS_PER_DAY);
        let days_back = (components.weekday - 2).rem_euclid(7);
        (days - days_back) * MILLIS_PER_DAY
    } else {
        // A month (and coarser): the first of the month 00:00 UTC.
        calendar::make(components.year, components.month, 1, 0, 0, 0, 0).unwrap_or(open_time)
    }
}

/// End of the bucket that opens at `start`.
pub(super) fn bucket_end(start: i64, interval_seconds: f64) -> i64 {
    if interval_seconds >= 2_419_200.0 {
        let later = start + (interval_seconds * 1.2 * 1000.0).round() as i64;
        bucket_start(later, interval_seconds)
    } else {
        start + (interval_seconds * 1000.0).round() as i64
    }
}

/// Folds candles (which share one bucket) into a single higher-timeframe candle.
fn fold_one(candles: &[Kline], interval_seconds: f64) -> Kline {
    let start = bucket_start(candles[0].open_time, interval_seconds);
    let mut folded = Kline {
        open_time: start,
        close_time: bucket_end(start, interval_seconds),
        open: candles[0].open,
        high: candles[0].high,
        low: candles[0].low,
        close: candles[0].close,
        volume: candles[0].volume,
    };
    for candle in &candles[1..] {
        folded.high = folded.high.max(candle.high);
        folded.low = folded.low.min(candle.low);
        folded.close = candle.close;
        folded.volume += candle.volume;
    }
    folded
}

/// The last candle that opens at or before `open_time`, found by bisection; None when all open later.
fn last_index(open_time: i64, candles: &[Kline]) -> Option<usize> {
    let mut low = 0usize;
    let mut high = candles.len();
    while low < high {
        let middle = (low + high) / 2;
        if candles[middle].open_time <= open_time {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    (low > 0).then(|| low - 1)
}

// MARK: Reference export

/// Copies the collections and objects a security expression returned out of the site's own state
/// and into the chart's, so a handle that crossed over does not name an unrelated value. Arrays and
/// instances are copied following the handles they hold (a handle reached twice is copied once);
/// handles inside a map or matrix are not followed and become `na`, and so do drawing handles.
struct ReferenceExport<'a> {
    source: &'a RuntimeState,
    copies: HashMap<usize, RuntimeValue>,
}

impl<'a> ReferenceExport<'a> {
    fn new(source: &'a RuntimeState) -> Self {
        Self {
            source,
            copies: HashMap::new(),
        }
    }

    fn export(&mut self, value: RuntimeValue, target: &mut RuntimeState) -> RuntimeValue {
        match value {
            RuntimeValue::Tuple(items) => RuntimeValue::Tuple(
                items
                    .into_iter()
                    .map(|item| self.export(item, target))
                    .collect(),
            ),
            RuntimeValue::Ref(kind, id) => self.copy(kind, id, target),
            other => other,
        }
    }

    fn copy(&mut self, kind: RefKind, id: usize, target: &mut RuntimeState) -> RuntimeValue {
        if let Some(known) = self.copies.get(&id) {
            return known.clone();
        }
        match kind {
            RefKind::Array => {
                let Some(items) = self.source.arrays.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                let copied = items
                    .into_iter()
                    .map(|item| self.export(item, target))
                    .collect();
                target.arrays.insert(handle, copied);
                self.copies[&id].clone()
            }
            RefKind::Object => {
                let Some(object) = self.source.instances.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                let mut copied = object.clone();
                copied.fields = object
                    .fields
                    .into_iter()
                    .map(|(name, field)| (name, self.export(field, target)))
                    .collect();
                target.instances.insert(handle, copied);
                self.copies[&id].clone()
            }
            RefKind::Map => {
                let Some(map) = self.source.maps.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                target.maps.insert(handle, map);
                self.copies[&id].clone()
            }
            RefKind::Matrix => {
                let Some(matrix) = self.source.matrices.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                target.matrices.insert(handle, matrix);
                self.copies[&id].clone()
            }
            // Drawing handles and any other reference belong to the expression's state.
            _ => RuntimeValue::Na,
        }
    }

    fn register(&mut self, kind: RefKind, id: usize, target: &mut RuntimeState) -> usize {
        target.next_reference += 1;
        let handle = target.next_reference;
        self.copies.insert(id, RuntimeValue::Ref(kind, handle));
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 3_600_000,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn bucket_start_and_end_fold_sub_day() {
        // 1h buckets at a 1h boundary.
        assert_eq!(bucket_start(7_300_000, 3600.0), 7_200_000);
        assert_eq!(bucket_end(3_600_000, 3600.0), 7_200_000);
        // 4h buckets floor to the 4h grid.
        assert_eq!(bucket_start(7_200_000, 14_400.0), 0);
    }

    #[test]
    fn fold_one_takes_open_high_low_close_and_sums_volume() {
        let folded = fold_one(&[bar(3_600_000, 10.0), bar(7_200_000, 12.0)], 14_400.0);
        assert_eq!(folded.open, 10.0);
        assert_eq!(folded.close, 12.0);
        assert_eq!(folded.volume, 2.0);
        assert_eq!(folded.open_time, 0);
    }

    #[test]
    fn last_index_bisects() {
        let candles = [bar(0, 1.0), bar(3_600_000, 2.0), bar(7_200_000, 3.0)];
        assert_eq!(last_index(0, &candles), Some(0));
        assert_eq!(last_index(3_600_000, &candles), Some(1));
        assert_eq!(last_index(3_599_999, &candles), Some(0));
        assert_eq!(last_index(-1, &candles), None);
    }

    #[test]
    fn reference_export_copies_arrays_across_the_boundary() {
        let mut source = RuntimeState::default();
        source.next_reference += 1;
        source.arrays.insert(1, vec![RuntimeValue::Int(5)]);
        let mut exporter = ReferenceExport::new(&source);

        let mut target = RuntimeState::default();
        let value = exporter.export(RuntimeValue::Ref(RefKind::Array, 1), &mut target);
        let RuntimeValue::Ref(RefKind::Array, handle) = value else {
            panic!("{value:?}");
        };
        assert_eq!(target.arrays[&handle], vec![RuntimeValue::Int(5)]);
    }

    #[test]
    fn reference_export_follows_nested_handles_and_degrades_drawings() {
        let mut source = RuntimeState::default();
        source.next_reference += 1;
        let array_id = source.next_reference;
        source.arrays.insert(array_id, vec![RuntimeValue::Int(7)]);
        source.next_reference += 1;
        let object_id = source.next_reference;
        source.instances.insert(
            object_id,
            super::super::containers::Object {
                type_name: "T".to_owned(),
                fields: [
                    (
                        "arr".to_owned(),
                        RuntimeValue::Ref(RefKind::Array, array_id),
                    ),
                    ("line".to_owned(), RuntimeValue::Ref(RefKind::Line, 99)),
                ]
                .into_iter()
                .collect(),
            },
        );
        // A self-referential array (a cycle), to check handles reached twice copy once.
        source.next_reference += 1;
        let cycle_id = source.next_reference;
        source
            .arrays
            .insert(cycle_id, vec![RuntimeValue::Ref(RefKind::Array, cycle_id)]);

        let mut exporter = ReferenceExport::new(&source);
        let mut target = RuntimeState::default();

        let value = exporter.export(RuntimeValue::Ref(RefKind::Object, object_id), &mut target);
        let RuntimeValue::Ref(RefKind::Object, handle) = value else {
            panic!("{value:?}");
        };
        let object = &target.instances[&handle];
        let RuntimeValue::Ref(RefKind::Array, arr_handle) = &object.fields["arr"] else {
            panic!();
        };
        assert_eq!(target.arrays[arr_handle], vec![RuntimeValue::Int(7)]);
        assert_eq!(
            object.fields["line"],
            RuntimeValue::Na,
            "a drawing handle is na"
        );

        let cycle = exporter.export(RuntimeValue::Ref(RefKind::Array, cycle_id), &mut target);
        let RuntimeValue::Ref(RefKind::Array, cycle_handle) = cycle else {
            panic!("{cycle:?}");
        };
        assert_eq!(
            target.arrays[&cycle_handle],
            vec![RuntimeValue::Ref(RefKind::Array, cycle_handle)],
            "a cycle survives the copy"
        );
    }

    #[test]
    fn week_and_month_buckets_use_the_calendar() {
        // A Thursday: 2024-01-11 12:00:00 UTC.
        let thursday = calendar::make(2024, 1, 11, 12, 0, 0, 0).unwrap();
        // The week (Mon) containing it starts Monday 2024-01-08.
        let monday = calendar::make(2024, 1, 8, 0, 0, 0, 0).unwrap();
        assert_eq!(bucket_start(thursday, 604_800.0), monday);
        // A month bucket is the first of the month.
        let first = calendar::make(2024, 1, 1, 0, 0, 0, 0).unwrap();
        assert_eq!(bucket_start(thursday, 2_592_000.0), first);
        // bucket_end for a month is the next month's start.
        let february = calendar::make(2024, 2, 1, 0, 0, 0, 0).unwrap();
        assert_eq!(bucket_end(first, 2_592_000.0), february);
    }

    use super::super::compiler::{self, NoLibraries};
    use super::super::program::{BarEvent, BarPhase, SymbolInfo};
    use super::super::token::Limits;

    #[derive(Default)]
    struct TestProvider {
        candles: HashMap<SecurityKey, Vec<Kline>>,
    }

    impl SecurityDataProvider for TestProvider {
        fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
            self.candles.get(key).cloned()
        }
    }

    fn run(
        source: &str,
        provider: TestProvider,
        bars: &[Kline],
    ) -> super::super::program::VisualOutput {
        let program = compiler::compile(source, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(provider)),
            None,
        );
        for (index, bar) in bars.iter().enumerate() {
            let is_last = index + 1 == bars.len();
            let _ = session.execute(
                BarEvent {
                    candle: bar.clone(),
                    phase: BarPhase::Historical,
                },
                is_last,
                None,
            );
        }
        session.output()
    }

    #[test]
    fn a_foreign_symbol_is_served_from_the_provider() {
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: "ETHUSDT".to_owned(),
                interval: 60.0,
                recent_bars: 0,
            },
            vec![bar(0, 100.0), bar(60_000, 110.0), bar(120_000, 120.0)],
        );
        let chart = [bar(0, 1.0), bar(60_000, 2.0), bar(120_000, 3.0)];
        let output = run(
            "//@version=6\nindicator(\"T\")\nx = request.security(\"ETHUSDT\", \"1\", close)\nplot(x)",
            provider,
            &chart,
        );
        assert_eq!(output.plots.len(), 1);
        assert_eq!(output.plots[0].values.last(), Some(&Some(120.0)));
    }

    #[test]
    fn security_to_a_finer_timeframe_reads_the_last_intrabar() {
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: String::new(),
                interval: 15.0,
                recent_bars: 0,
            },
            vec![
                bar(60_000, 1.0),
                bar(75_000, 2.0),
                bar(90_000, 3.0),
                bar(105_000, 4.0),
            ],
        );
        let chart = [bar(0, 10.0), bar(60_000, 11.0)];
        let source = "//@version=6\nindicator(\"T\")\nx = request.security(syminfo.tickerid, \"15S\", close)\nplot(x)";
        let output = run(source, provider, &chart);
        // The second bar's last 15s intrabar closes at 4; the first bar has none.
        assert_eq!(output.plots[0].values.last(), Some(&Some(4.0)));
        assert_eq!(output.plots[0].values.first(), Some(&None));
        // Without finer candles the run fails (the helper swallows the error): nothing is plotted.
        let missing = run(source, TestProvider::default(), &chart);
        assert!(
            missing
                .plots
                .iter()
                .all(|plot| plot.values.iter().all(Option::is_none))
        );
    }

    /// Answers only when told how far the chart reaches, as a provider that fetches intrabars does.
    struct SpanOnlyProvider {
        seen: std::sync::Arc<std::sync::Mutex<Vec<ChartSpan>>>,
    }

    impl SecurityDataProvider for SpanOnlyProvider {
        fn candles(&self, _key: &SecurityKey) -> Option<Vec<Kline>> {
            None
        }

        fn candles_over(&self, _key: &SecurityKey, chart: ChartSpan) -> Option<Vec<Kline>> {
            self.seen.lock().unwrap().push(chart);
            Some(vec![bar(60_000, 1.0), bar(75_000, 2.0)])
        }
    }

    #[test]
    fn the_provider_is_told_the_span_of_the_chart() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let program = compiler::compile(
            "//@version=6\nindicator(\"T\")\nplot(array.sum(request.security_lower_tf(syminfo.tickerid, \"15S\", close)))",
            Limits::default(),
            &NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(SpanOnlyProvider { seen: seen.clone() })),
            None,
        );
        let chart = [bar(0, 10.0), bar(60_000, 11.0)];
        assert!(session.load_history(&chart, false, None).is_ok());
        let output = session.output();
        assert_eq!(output.plots[0].values, [Some(0.0), Some(3.0)]);
        // From the first bar's open to the end of the last (60 s spacing), asked once per run.
        assert_eq!(
            *seen.lock().unwrap(),
            [ChartSpan {
                start_ms: 0,
                end_ms: 120_000,
                spacing_ms: 60_000
            }]
        );
    }

    #[test]
    fn security_lower_tf_serves_another_symbols_intrabars() {
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: "ETHUSDT".to_owned(),
                interval: 15.0,
                recent_bars: 0,
            },
            vec![bar(60_000, 5.0), bar(75_000, 6.0), bar(90_000, 7.0)],
        );
        let chart = [bar(0, 10.0), bar(60_000, 11.0)];
        let output = run(
            "//@version=6\nindicator(\"T\")\nplot(array.sum(request.security_lower_tf(\"ETHUSDT\", \"15S\", close)))",
            provider,
            &chart,
        );
        assert_eq!(output.plots[0].values, [Some(0.0), Some(18.0)]);
    }

    #[test]
    fn a_function_returning_a_tuple_gives_one_array_per_value() {
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: String::new(),
                interval: 15.0,
                recent_bars: 0,
            },
            vec![
                bar(60_000, 1.0),
                bar(75_000, 2.0),
                bar(90_000, 3.0),
                bar(105_000, 4.0),
            ],
        );
        let chart = [bar(0, 10.0), bar(60_000, 11.0)];
        let output = run(
            "//@version=6\nindicator(\"T\")\npair() =>\n    [close, close * 2]\n\
             [first, second] = request.security_lower_tf(syminfo.tickerid, \"15S\", pair())\n\
             plot(array.sum(first))\nplot(array.sum(second))",
            provider,
            &chart,
        );
        // The first bar has no intrabars: both arrays are empty. The second sums 1 + 2 + 3 + 4 and double.
        assert_eq!(output.plots[0].values, [Some(0.0), Some(10.0)]);
        assert_eq!(output.plots[1].values, [Some(0.0), Some(20.0)]);
    }

    #[test]
    fn security_lower_tf_serves_intrabars() {
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: String::new(),
                interval: 15.0,
                recent_bars: 0,
            },
            vec![
                bar(60_000, 1.0),
                bar(75_000, 2.0),
                bar(90_000, 3.0),
                bar(105_000, 4.0),
            ],
        );
        let chart = [bar(0, 10.0), bar(60_000, 11.0)];
        let output = run(
            "//@version=6\nindicator(\"T\")\nx = array.sum(request.security_lower_tf(syminfo.tickerid, \"15S\", close))\nplot(x)",
            provider,
            &chart,
        );
        // On the second bar the four 15s intrabars sum to 1 + 2 + 3 + 4.
        assert_eq!(output.plots[0].values.last(), Some(&Some(10.0)));
    }
}
