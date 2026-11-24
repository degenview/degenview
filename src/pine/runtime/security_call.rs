//! `request.security` and `request.security_lower_tf`, ported from
//! `PineRuntimeSession+Security.swift` + `+ForeignSecurity.swift`. These are the one Pine builtin
//! family that is not a pure `RuntimeState` handler: the `expression` argument is kept unevaluated
//! and runs against a higher (or foreign, or finer) series by swapping `working` for the call site's
//! own `SecuritySite.state` and calling `Session::eval`. The `SecurityDataProvider` supplies the
//! other symbol's / finer-timeframe candles; the chart's own higher timeframe is folded from the
//! chart's bars.

use std::collections::HashMap;
use std::sync::Arc;

use super::fast_hash::FastMap;
use super::security_slice;
use super::session::Session;
use super::state::{LoggedBar, RuntimeState, SecuritySite};
use super::support::{ChartSpan, SecurityKey};
use super::types::{BarFlags, RuntimeContext};
use super::value::{RefKind, RuntimeValue};
use crate::model::Kline;
use crate::pine::builtins::array;
use crate::pine::logs::LogRequest;
use crate::pine::syntax::ast::{Argument, Expression, Statement};
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourceRange};
use crate::pine::time;

/// How many `request.*` expressions may contain one another.
const SECURITY_NESTING_LIMIT: usize = 3;

/// A chart bar (or a candle of the enclosing request) a call site takes in. `skipped_after` is set
/// for a bar the site was not called on, replayed later: how many logged bars came after it.
#[derive(Clone, Copy)]
struct Step<'a> {
    bar: &'a Kline,
    confirmed: bool,
    chart: ChartBar,
    skipped_after: Option<usize>,
}

/// The chart bar a request is evaluated for, which decides the request's `barstate.*`: the series
/// the expression runs on is the same run, so a candle is realtime when the chart bar is and the
/// candle does not end before the chart bar opens (an older candle caught up on a live bar is
/// history), and it is the last when it holds the chart bar's final instant.
#[derive(Clone, Copy)]
struct ChartBar {
    /// When the chart bar opens; `i64::MAX` for history before the chart's first bar.
    start: i64,
    /// When the chart bar closes (exclusive), where its bucket ends: a bar that does not divide the
    /// day is cut at midnight. `i64::MIN` for history before the chart's first bar.
    end: i64,
    /// Where the chart's last bar closes (`i64::MIN` when it is not known): the candle that holds
    /// that instant is the last of the series (`barstate.islast`), whichever chart bar reads it.
    last_end: i64,
    /// Where the history the chart was loaded with ends, when it is known: with a live bar it is the
    /// moment the script was loaded (`Session::history_end`). The candle that closed last by then is
    /// the request's last confirmed history bar (`barstate.islastconfirmedhistory`), and a candle
    /// that ends after it is realtime on a realtime chart bar.
    history_end: Option<i64>,
    /// Where the history the session was loaded with ends (`Session::history_end`), None when it
    /// was never loaded as one: a candle that closed by then is confirmed (`barstate.isconfirmed`).
    loaded_end: Option<i64>,
    realtime: bool,
    /// Whether this is the chart bar's first tick (`barstate.isnew`); true on history.
    new: bool,
    /// The candles of the series, when the provider holds them (a finer or a foreign request): its
    /// last candle, and the last that closed by the history's end, are not where the chart's own
    /// buckets would put them (a stock has no candle between the close and the next open).
    candles: Option<SeriesEdges>,
}

/// The candles of a requested series that `barstate.islast` and `barstate.islastconfirmedhistory`
/// name, by open time.
#[derive(Clone, Copy)]
struct SeriesEdges {
    last: i64,
    last_confirmed: Option<i64>,
}

impl ChartBar {
    /// History the chart does not show: the candles before its first bar.
    const BEFORE_CHART: Self = Self {
        start: i64::MAX,
        end: i64::MIN,
        last_end: i64::MIN,
        history_end: None,
        loaded_end: None,
        realtime: false,
        new: true,
        candles: None,
    };

    /// The bar `bar` runs as. `history_end` is `Session::history_end`; without it the chart bar
    /// that is the last confirmed history one ends the history. `last_bar_time` is where the chart's
    /// last bar opens, when known; without it only the bar that is the last one has the last candle.
    fn of(
        bar: &Kline,
        seconds: f64,
        flags: &BarFlags,
        history_end: Option<i64>,
        last_bar_time: Option<i64>,
    ) -> Self {
        let end = Self::end_of(bar, seconds);
        let last_end = flags
            .is_last
            .then_some(end)
            .or_else(|| {
                last_bar_time
                    .map(|open| time::bucket::end(time::bucket::start(open, seconds), seconds))
            })
            .unwrap_or(i64::MIN);
        Self {
            start: bar.open_time,
            end,
            last_end,
            history_end: history_end.or(flags.is_last_confirmed_history.then_some(end)),
            loaded_end: history_end,
            realtime: flags.is_realtime,
            new: flags.is_new,
            candles: None,
        }
    }

    /// This chart bar reading the series `candles` of `interval` seconds.
    fn reading(self, candles: &[Kline], interval: f64) -> Self {
        let edges = candles.last().map(|last| SeriesEdges {
            last: last.open_time,
            last_confirmed: self.history_end.and_then(|end| {
                let closed = candles
                    .partition_point(|candle| time::bucket::end(candle.open_time, interval) <= end);
                closed.checked_sub(1).map(|index| candles[index].open_time)
            }),
        });
        Self {
            candles: edges,
            ..self
        }
    }

    /// A confirmed chart bar before the one now running, replayed for a site the call skipped.
    fn replayed(bar: &Kline, seconds: f64, history_end: Option<i64>) -> Self {
        Self {
            start: bar.open_time,
            end: Self::end_of(bar, seconds),
            history_end,
            loaded_end: history_end,
            ..Self::BEFORE_CHART
        }
    }

    /// Where the bar closes: the end of its bucket, which is not `open_time + seconds` for a length
    /// that does not divide the day (a 5h bar opened at 20:00 closes at midnight) or a calendar month.
    fn end_of(bar: &Kline, seconds: f64) -> i64 {
        time::bucket::end(time::bucket::start(bar.open_time, seconds), seconds)
    }

    /// The bar states of `candle`, a candle of `interval` seconds, evaluated for this chart bar.
    ///
    /// The last confirmed history bar and the last bar belong to the requested series, not to the
    /// chart bar. The last confirmed history bar is the last candle that closed with the history,
    /// so the one whose successor is not history (a `"1D"` request on a 4h chart, probe P12), and
    /// the last bar is the series' final candle: the one that holds the chart's last instant (a
    /// `"1D"` request with `lookahead_on` reads it on every chart bar of the day, probes A and R4).
    /// A series the provider holds names them by its candles (`SeriesEdges`).
    ///
    /// A candle is realtime on a realtime chart bar when it ends after the history did: one that
    /// closed before the script was loaded is history even inside the live chart bar (probe S2: of
    /// the 15m candles of a live 4h bar only the one forming at load was realtime).
    ///
    /// A candle is new on history and on the first tick of the chart bar that opens it: a chart bar
    /// that starts inside a candle sees that candle as already running.
    ///
    /// A candle is confirmed once it closed, and on history also while it develops (`lookahead_on`)
    /// when it closes by the history's end: the candle is complete in the data, whichever chart bar
    /// reads it. The one still open at the history's end, and one developing in real time, is not
    /// (TradingView-measured: a completed month reads 1 from every chart bar inside it, the final
    /// incomplete month 0, probes A and B; a developing day on a 4h history reads 1, probe R1).
    fn flags_of(&self, candle: &Kline, interval: f64, commit: bool, is_first: bool) -> BarFlags {
        let candle_end = time::bucket::end(candle.open_time, interval);
        let (is_last, is_last_confirmed_history) = match self.candles {
            Some(edges) => (
                candle.open_time == edges.last,
                edges.last_confirmed == Some(candle.open_time),
            ),
            None => (
                candle.open_time < self.last_end && self.last_end <= candle_end,
                self.history_end.is_some_and(|end| {
                    candle_end <= end && time::bucket::end(candle_end, interval) > end
                }),
            ),
        };
        let is_realtime =
            self.realtime && candle_end > self.start.max(self.history_end.unwrap_or(i64::MIN));
        BarFlags {
            is_first,
            is_last,
            is_history: !is_realtime,
            is_realtime,
            is_new: !is_realtime || (self.new && candle.open_time >= self.start),
            is_confirmed: commit || self.loaded_end.is_some_and(|end| candle_end <= end),
            is_last_confirmed_history,
        }
    }
}

/// The session state that belongs to the series now running, which a request's expression swaps for
/// its own (`Session::enter_series`, `leave_series`). `bar_index` and `instructions` of `working`
/// are the request's to advance, and the nesting depth is counted by the caller: neither travels.
struct SeriesFrame {
    /// The bar-by-bar state: the chart's, or the call site's own.
    working: RuntimeState,
    /// The confirmed bars of the series (`Session::series_bars`).
    bars: Vec<LoggedBar>,
    /// The chart variables the running expression reads (`Session::security_globals`); none on the
    /// chart itself.
    globals: Option<FastMap<String, RuntimeValue>>,
    /// The `varip` values of the series.
    intrabar: FastMap<String, RuntimeValue>,
    bar_seconds: f64,
    /// The chart's own bar length, known only while a request runs.
    chart_bar_seconds: Option<f64>,
}

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
    /// bar's intrabars and the value of the last one stands for the bar, or of the first with
    /// `lookahead_on`, as in Pine (TradingView probe S1: an AAPL 1D chart reads a `"60"` request's
    /// 15:30 candle, and its 9:30 one with `lookahead_on`).
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
        Ok(self.pick_intrabar(intrabars, request.lookahead_on))
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

    /// The first or the last element of the array (or of each array of a tuple) `serve_intrabars`
    /// returned.
    fn pick_intrabar(&self, intrabars: RuntimeValue, first: bool) -> RuntimeValue {
        match intrabars {
            RuntimeValue::Tuple(items) => RuntimeValue::Tuple(
                items
                    .into_iter()
                    .map(|item| self.pick_intrabar(item, first))
                    .collect(),
            ),
            RuntimeValue::Ref(RefKind::Array, id) => self
                .working
                .arrays
                .get(&id)
                .and_then(|values| if first { values.first() } else { values.last() })
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
    /// The chart bar `bar` that a request now runs for.
    fn chart_bar(&self, bar: &Kline, flags: &BarFlags) -> ChartBar {
        // Inside a request the chart's last bar is not the running series' own: only the flags say.
        // Nor is it known when it is this bar (a run that is not a loaded history takes each bar as
        // the last so far): the flags say too.
        let last_bar_time = self
            .last_bar_time
            .filter(|last| self.security_depth == 0 && *last > bar.open_time);
        ChartBar::of(
            bar,
            self.bar_seconds,
            flags,
            self.history_end,
            last_bar_time,
        )
    }

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
    pub(in crate::pine) fn series(&mut self, key: SecurityKey) -> Option<Arc<Vec<Kline>>> {
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
                unserved_symbol_message(name)
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
            if time::bucket::end(candle.open_time, request.interval) > before {
                break;
            }
            site.last_result = self.evaluate_higher_timeframe(
                site,
                candle,
                request,
                ChartBar::BEFORE_CHART,
                true,
            )?;
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
            // History from before the chart's first bar; the chart's own bars, the ones this site
            // was not called on included, are replayed below.
            let before = self
                .series_bars
                .first()
                .map_or(bar.open_time, |first| first.bar.open_time);
            self.warm_up(&mut state, request, before)?;
        }
        self.replay_skipped_bars(&mut state, request, bar.open_time)?;
        let (result, fresh) = self.advance_site(
            &mut state,
            request,
            &Step {
                bar: &bar,
                confirmed: context.flags.is_confirmed,
                chart: self.chart_bar(&bar, &context.flags),
                skipped_after: None,
            },
        )?;
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

    /// Takes the confirmed bars of the running series (the chart's, or the candles of the request
    /// this one sits in) before `before` that this site was not called on into its
    /// higher-timeframe candle, as TradingView evaluates the expression on every bar whether or not
    /// the call runs: a call under a condition sees the same candles and builds the same history as
    /// one that always runs.
    fn replay_skipped_bars(
        &mut self,
        state: &mut SecuritySite,
        request: &SecurityRequest,
        before: i64,
    ) -> Result<(), Diagnostic> {
        let start = state.processed_bar.map_or(0, |seen| {
            self.series_bars
                .partition_point(|logged| logged.bar.open_time <= seen)
        });
        let end = self
            .series_bars
            .partition_point(|logged| logged.bar.open_time < before);
        for index in start..end {
            let LoggedBar { bar, seconds } = self.series_bars[index].clone();
            let skipped_after = self.series_bars.len() - 1 - index;
            self.advance_site(
                state,
                request,
                &Step {
                    bar: &bar,
                    confirmed: true,
                    chart: ChartBar::replayed(&bar, seconds, self.history_end),
                    skipped_after: Some(skipped_after),
                },
            )?;
            state.processed_bar = Some(bar.open_time);
        }
        Ok(())
    }

    /// The chart variables (those the expression reads through `security_globals`) as they stood on
    /// the confirmed bar `back` bars before the latest one, from their histories. A variable that did
    /// not exist yet is `na`. The values are the ones the bar ended with, so a variable assigned
    /// after the request in the script reads one bar fresher than a call that ran there would see.
    fn variables_as_of(&self, back: usize) -> FastMap<String, RuntimeValue> {
        self.working
            .variables
            .keys()
            .map(|name| {
                let value = self
                    .working
                    .histories
                    .get(name)
                    .and_then(|history| {
                        history
                            .len()
                            .checked_sub(back + 1)
                            .and_then(|at| history.get(at))
                    })
                    .cloned()
                    .unwrap_or(RuntimeValue::Na);
                (name.clone(), value)
            })
            .collect()
    }

    /// Takes `step`'s bar into the site's candle. Returns the expression's value for the bar and
    /// whether a higher-timeframe bar was completed on it. A replayed bar (`skipped_after` is set)
    /// only keeps the site's state current: nothing is evaluated for the bar itself, and a candle it
    /// completes is evaluated with the chart variables of that bar.
    fn advance_site(
        &mut self,
        state: &mut SecuritySite,
        request: &SecurityRequest,
        step: &Step<'_>,
    ) -> Result<(RuntimeValue, bool), Diagnostic> {
        let Step {
            bar,
            confirmed,
            chart,
            skipped_after,
        } = *step;
        let replay = skipped_after.is_some();
        let start = time::bucket::start(bar.open_time, request.interval);
        let mut fresh = false;
        if state.bucket != Some(start) {
            // The chart skipped the bucket's last bar, so complete it now.
            if state.has_pending_bars
                && let Some(pending) = state.accumulated.clone()
            {
                self.replay_globals = skipped_after.map(|back| self.variables_as_of(back));
                state.last_result =
                    self.evaluate_higher_timeframe(state, &pending, request, chart, true)?;
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
        let end = time::bucket::end(start, request.interval);
        // The chart bar closes the bucket when its own end reaches the bucket's: a monthly bar is
        // not `open_time + 30 days` (a 31-day month would never close its quarter or year).
        let closes_bucket = chart.end >= end;

        let result;
        if closes_bucket && confirmed {
            self.replay_globals = skipped_after.map(|back| self.variables_as_of(back));
            result = self.evaluate_higher_timeframe(state, &developing, request, chart, true)?;
            state.last_result = result.clone();
            state.accumulated = None;
            state.has_pending_bars = false;
            fresh = true;
        } else {
            if confirmed {
                state.accumulated = Some(developing.clone());
                state.has_pending_bars = true;
            }
            if !replay && (request.lookahead_on || chart.realtime) {
                result =
                    self.evaluate_higher_timeframe(state, &developing, request, chart, false)?;
            } else {
                result = state.last_result.clone();
            }
        }
        Ok((result, fresh))
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
        let chart = self
            .chart_bar(&bar, &context.flags)
            .reading(candles, request.interval);
        let mut fresh = false;
        if context.flags.is_confirmed {
            let chart_close = chart.end;
            while state.completed_candles < candles.len() {
                let candle = &candles[state.completed_candles];
                if time::bucket::end(candle.open_time, request.interval) > chart_close {
                    break;
                }
                state.last_result =
                    self.evaluate_higher_timeframe(&mut state, candle, request, chart, true)?;
                state.completed_candles += 1;
                fresh = true;
            }
        }
        let mut result = state.last_result.clone();
        if (request.lookahead_on || context.flags.is_realtime)
            && let Some(index) = last_index(bar.open_time, candles)
            && index >= state.completed_candles
        {
            result =
                self.evaluate_higher_timeframe(&mut state, &candles[index], request, chart, false)?;
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
        // A candle belongs to the chart bar its last instant falls in. The bar spans as long as its
        // bucket, from where it opens (a stock's daily bar opens with its session, inside the UTC
        // day its 4h candles are cut on): not `open_time` plus the chart's nominal length, which
        // gives a 31-day month 30 days, nor the candles that merely open inside it (TradingView
        // probes R2 and R5).
        let chart = self.chart_bar(&bar, &context.flags);
        let span = chart.end - time::bucket::start(bar.open_time, self.bar_seconds);
        let window_end = bar.open_time + span;
        let ends = |candle: &Kline| time::bucket::end(candle.open_time, interval);
        let low = candles.partition_point(|candle| ends(candle) <= bar.open_time);
        let request = SecurityRequest {
            symbol: String::new(),
            interval,
            expression: expression.clone(),
            lookahead_on: false,
            gaps_on: false,
            foreign: None,
        };
        let chart = chart.reading(candles, interval);
        let mut results = Vec::new();
        let mut index = low;
        while index < candles.len() && ends(&candles[index]) <= window_end {
            results.push(self.evaluate_higher_timeframe(
                &mut state,
                &candles[index],
                &request,
                chart,
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

    /// Runs the expression on `candle` in the site's own state, for `chart_bar`. A commit keeps
    /// that state and appends the candle to its histories; otherwise the state is thrown away, like
    /// a realtime tick.
    fn evaluate_higher_timeframe(
        &mut self,
        site: &mut SecuritySite,
        candle: &Kline,
        request: &SecurityRequest,
        chart_bar: ChartBar,
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
        // The chart's variables stay readable from the expression, without a copy of the table. A
        // replayed candle reads their values as of its bar instead, which are a copy.
        let replayed = self.replay_globals.is_some();
        let globals = self
            .replay_globals
            .take()
            .unwrap_or_else(|| std::mem::take(&mut self.working.variables));
        let mut chart = self.enter_series(SeriesFrame {
            working: site_state,
            // The series the expression runs on is this site's own candles, which a request inside
            // it replays.
            bars: std::mem::take(&mut site.bars),
            globals: Some(globals),
            // `varip` values are the chart's: the request keeps its own (empty: a `varip` there
            // acts like `var`, which the declaration reads from the site's variables).
            intrabar: FastMap::default(),
            bar_seconds: request.interval,
            // The outermost request remembers the chart's own bar length for
            // `timeframe.main_period`.
            chart_bar_seconds: Some(self.chart_bar_seconds.unwrap_or(chart_bar_seconds)),
        });
        self.working.instructions = 0;
        self.working.bar_index += 1;

        let flags = chart_bar.flags_of(
            candle,
            request.interval,
            commit,
            self.working.bar_index == 0,
        );
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
        // The script variables the expression reads are computed on this series, not read off the chart.
        let slice = self
            .security_slices
            .get(&security_slice::key(&request.expression))
            .cloned();
        let evaluated = match slice {
            Some(slice) => self.run(&slice, &mut security_context).map(|_| ()),
            None => Ok(()),
        }
        .and_then(|()| self.eval(&request.expression, &mut security_context));
        self.logs.leave_request();
        if debugging {
            self.debug_leave();
        }
        // A returned collection or object lives in the site's state; the chart gets a copy of its own.
        let value = evaluated.map(|evaluated| {
            ReferenceExport::new(&self.working).export(evaluated, &mut chart.working)
        });
        if commit && value.is_ok() {
            self.commit_histories(candle.clone(), false, Vec::new());
        }
        let request_frame = self.leave_series(chart);
        if commit {
            *site.state = request_frame.working;
        }
        site.bars = request_frame.bars;
        if commit && value.is_ok() {
            site.bars.push(LoggedBar {
                bar: candle.clone(),
                seconds: request.interval,
            });
        }
        if !replayed {
            self.working.variables = request_frame.globals.unwrap_or_default();
        }
        self.security_depth -= 1;
        value
    }

    /// Puts `inner`, a request's series, in place of the one now running and returns what it
    /// displaced, which `leave_series` puts back.
    fn enter_series(&mut self, inner: SeriesFrame) -> SeriesFrame {
        self.swap_series(inner)
    }

    /// Puts the `outer` series back and returns the request's own, for its state and candles to be
    /// kept.
    fn leave_series(&mut self, outer: SeriesFrame) -> SeriesFrame {
        self.swap_series(outer)
    }

    /// Exchanges the session's per-series state with `frame`. Entering and leaving are the same
    /// exchange, so every field travels in both directions and none is left behind on an exit.
    fn swap_series(&mut self, frame: SeriesFrame) -> SeriesFrame {
        SeriesFrame {
            working: std::mem::replace(&mut self.working, frame.working),
            bars: std::mem::replace(&mut self.series_bars, frame.bars),
            globals: std::mem::replace(&mut self.security_globals, frame.globals),
            intrabar: std::mem::replace(&mut self.intrabar, frame.intrabar),
            bar_seconds: std::mem::replace(&mut self.bar_seconds, frame.bar_seconds),
            chart_bar_seconds: std::mem::replace(
                &mut self.chart_bar_seconds,
                frame.chart_bar_seconds,
            ),
        }
    }

    /// A fresh array holding `items`.
    fn make_array(&mut self, items: Vec<RuntimeValue>) -> RuntimeValue {
        array::new_array(&mut self.working, items)
    }
}

/// A tuple of `columns` empty arrays (or one empty array) for the expression shape when a timeframe
/// cannot be served.
fn empty_shaped(columns: Option<usize>, session: &mut Session) -> RuntimeValue {
    match columns {
        Some(columns) => RuntimeValue::Tuple(
            (0..columns)
                .map(|_| array::new_array(&mut session.working, Vec::new()))
                .collect(),
        ),
        None => array::new_array(&mut session.working, Vec::new()),
    }
}

/// Why a symbol the provider could not serve got no data, with the way to fix the usual cause: a
/// bare `ETH` is read from Binance as written, where only the full pair (`ETHUSDT`) exists.
fn unserved_symbol_message(name: &str) -> String {
    if name.contains(':') {
        format!(
            "No data is available for symbol '{name}'. Check that the source before the colon is \
             one DegenView reads (such as BINANCE, COINBASE or ALPACA) and that the pair is \
             spelled the way that source names it."
        )
    } else {
        format!(
            "No data is available for symbol '{name}'. A symbol without a source is read from \
             Binance exactly as written, so give the full pair, such as \"BINANCE:ETHUSDT\" or \
             \"COINBASE:ETH-USD\"."
        )
    }
}

/// Folds candles (which share one bucket) into a single higher-timeframe candle, closing where the
/// bucket ends.
fn fold_one(candles: &[Kline], interval_seconds: f64) -> Kline {
    let start = time::bucket::start(candles[0].open_time, interval_seconds);
    let mut folded = Kline {
        open_time: start,
        close_time: time::bucket::end(start, interval_seconds),
        ..candles[0].clone()
    };
    for candle in &candles[1..] {
        folded.absorb(candle);
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
    use crate::pine::runtime::support::SecurityDataProvider;

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
    fn an_unserved_symbol_says_how_to_name_it() {
        let bare = unserved_symbol_message("ETH");
        assert!(bare.starts_with("No data is available for symbol 'ETH'."));
        assert!(bare.contains("Binance"));
        assert!(bare.contains("\"BINANCE:ETHUSDT\""));
        let prefixed = unserved_symbol_message("KRAKEN:ETHUSD");
        assert!(prefixed.contains("source before the colon"));
        assert!(!prefixed.contains("without a source"));
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
    fn fold_one_of_a_quarter_spans_the_calendar_quarter() {
        let february = time::calendar::make(2024, 2, 10, 0, 0, 0, 0).unwrap();
        let folded = fold_one(&[bar(february, 10.0)], 7_776_000.0);
        assert_eq!(
            folded.open_time,
            time::calendar::make(2024, 1, 1, 0, 0, 0, 0).unwrap()
        );
        assert_eq!(
            folded.close_time,
            time::calendar::make(2024, 4, 1, 0, 0, 0, 0).unwrap()
        );
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
            crate::pine::runtime::containers::Object {
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

    use crate::pine::compiler::program::{BarEvent, BarPhase, SymbolInfo};
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

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
    ) -> crate::pine::compiler::program::VisualOutput {
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

    /// ETHUSDT closes 100, 110, 120 on the same bars as a chart that closes 1, 2, 3.
    fn eth_provider() -> TestProvider {
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: "ETHUSDT".to_owned(),
                interval: 60.0,
                recent_bars: 0,
            },
            vec![bar(0, 100.0), bar(60_000, 110.0), bar(120_000, 120.0)],
        );
        provider
    }

    fn last_plotted(source: &str) -> Option<f64> {
        let chart = [bar(0, 1.0), bar(60_000, 2.0), bar(120_000, 3.0)];
        let body = format!("//@version=6\nindicator(\"T\")\n{source}\nplot(x)");
        run(&body, eth_provider(), &chart).plots[0]
            .values
            .last()
            .copied()
            .flatten()
    }

    /// `count` hourly chart bars from `first_hour` (hours since the epoch, so hour 12 opens a 4h
    /// candle and hour 24 a day).
    fn hourly_chart(first_hour: i64, count: i64) -> Vec<Kline> {
        (first_hour..first_hour + count)
            .map(|hour| bar(hour * 3_600_000, 1.0 + hour as f64))
            .collect()
    }

    /// The first plot of `source` over `chart`, bar by bar, with no provider candles.
    fn plotted(source: &str, chart: &[Kline]) -> Vec<Option<f64>> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        run(&body, TestProvider::default(), chart).plots[0]
            .values
            .clone()
    }

    #[test]
    fn bar_index_in_a_request_counts_from_zero() {
        // 4h candles complete on chart bars 3, 7 and 11.
        let values = plotted(
            "plot(request.security(syminfo.tickerid, \"240\", bar_index))",
            &hourly_chart(0, 12),
        );
        assert_eq!(values[3], Some(0.0));
        assert_eq!(values[7], Some(1.0));
        assert_eq!(values[11], Some(2.0));
    }

    #[test]
    fn barstate_isfirst_in_a_request_is_true_on_its_first_bar_only() {
        let values = plotted(
            "plot(request.security(syminfo.tickerid, \"240\", barstate.isfirst ? 1.0 : 0.0))",
            &hourly_chart(0, 12),
        );
        assert_eq!(values[3], Some(1.0));
        assert_eq!(values[7], Some(0.0));
    }

    #[test]
    fn timeframe_change_is_false_on_the_charts_first_bar() {
        let chart = hourly_chart(13, 16);
        let values = plotted("plot(timeframe.change(\"1D\") ? 1.0 : 0.0)", &chart);
        assert_eq!(values[0], Some(0.0), "hour 13 opens mid-day");
        assert_eq!(values[10], Some(0.0));
        assert_eq!(values[11], Some(1.0), "hour 24 opens a day");
    }

    #[test]
    fn timeframe_change_in_a_request_is_false_on_the_series_first_bar() {
        // 4h candles open at hours 12, 16, 20 and 24 and complete on chart bars 2, 6, 10 and 14.
        let values = plotted(
            "plot(request.security(syminfo.tickerid, \"240\", timeframe.change(\"1D\") ? 1.0 : 0.0))",
            &hourly_chart(13, 16),
        );
        assert_eq!(values[2], Some(0.0), "the requested series' first candle");
        assert_eq!(values[10], Some(0.0), "20:00 is the same day as 16:00");
        assert_eq!(values[14], Some(1.0), "24:00 opens a day");
    }

    #[test]
    fn barstate_islast_in_a_request_holds_for_the_candle_with_the_charts_last_bar() {
        let request = |bars: i64| {
            plotted(
                "plot(request.security(syminfo.tickerid, \"240\", barstate.islast ? 1.0 : 0.0))",
                &hourly_chart(0, bars),
            )
        };
        let values = request(12);
        assert_eq!(
            values[7],
            Some(0.0),
            "the candle at hours 4-7 is not the last"
        );
        assert_eq!(
            values[11],
            Some(1.0),
            "the candle at hours 8-11 holds the last bar"
        );
        // A last bar in the middle of a candle: the developing candle (lookahead on) is the last.
        let developing = plotted(
            "plot(request.security(syminfo.tickerid, \"240\", barstate.islast ? 1.0 : 0.0, lookahead = barmerge.lookahead_on))",
            &hourly_chart(0, 10),
        );
        assert_eq!(developing[5], Some(0.0));
        assert_eq!(developing[9], Some(1.0));
    }

    #[test]
    fn a_request_on_history_is_history_even_for_a_developing_candle() {
        let states = |state: &str| {
            plotted(
                &format!(
                    "plot(request.security(syminfo.tickerid, \"240\", {state} ? 1.0 : 0.0, lookahead = barmerge.lookahead_on))"
                ),
                &hourly_chart(0, 8),
            )
        };
        assert_eq!(states("barstate.ishistory")[5], Some(1.0));
        assert_eq!(states("barstate.isrealtime")[5], Some(0.0));
    }

    #[test]
    fn a_request_on_a_live_tick_is_realtime() {
        let request = |state: &str| {
            plotted_after_each_tick(&format!(
                "plot(request.security(\"ETHUSDT\", \"1\", {state} ? 1.0 : 0.0))"
            ))
        };
        assert_eq!(request("barstate.isrealtime"), [Some(1.0), Some(1.0)]);
        assert_eq!(request("barstate.ishistory"), [Some(0.0), Some(0.0)]);
    }

    /// The first plot of `source` after `history` ran as history and `live` closed in real time.
    fn plotted_live_close(
        source: &str,
        provider: TestProvider,
        history: &[Kline],
        live: Kline,
    ) -> Vec<Option<f64>> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        let program = compiler::compile(&body, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(provider)),
            None,
        );
        let bars = history
            .iter()
            .map(|bar| (bar.clone(), BarPhase::Historical))
            .chain([(live, BarPhase::RealtimeClose { is_new: true })]);
        let count = history.len() + 1;
        for (index, (candle, phase)) in bars.enumerate() {
            let _ = session.execute(BarEvent { candle, phase }, index + 1 == count, None);
        }
        session.output().plots[0].values.clone()
    }

    /// The first plot of `source` after each tick of the live bar `live`, which follows `history`.
    fn plotted_per_live_tick(
        source: &str,
        history: &[Kline],
        live: Kline,
        ticks: &[bool],
    ) -> Vec<Option<f64>> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        let program = compiler::compile(&body, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(TestProvider::default())),
            None,
        );
        for candle in history {
            let _ = session.execute(
                BarEvent {
                    candle: candle.clone(),
                    phase: BarPhase::Historical,
                },
                false,
                None,
            );
        }
        ticks
            .iter()
            .map(|&is_new| {
                let _ = session.execute(
                    BarEvent {
                        candle: live.clone(),
                        phase: BarPhase::RealtimeTick { is_new },
                    },
                    true,
                    None,
                );
                session.output().plots[0].values.last().copied().flatten()
            })
            .collect()
    }

    #[test]
    fn barstate_isnew_in_a_request_is_true_on_history_and_on_the_first_tick_of_a_candle_it_opens() {
        let source =
            "plot(request.security(syminfo.tickerid, \"240\", barstate.isnew ? 1.0 : 0.0))";
        let history = plotted(source, &hourly_chart(0, 12));
        assert_eq!(&history[3], &Some(1.0));
        assert_eq!(&history[7], &Some(1.0));
        assert_eq!(&history[11], &Some(1.0));
        // The live bar at hour 4 opens the 4h candle at hour 4: new on its first tick only.
        let opening = plotted_per_live_tick(
            source,
            &hourly_chart(0, 4),
            bar(4 * 3_600_000, 5.0),
            &[true, false, false],
        );
        assert_eq!(opening, [Some(1.0), Some(0.0), Some(0.0)]);
    }

    #[test]
    fn barstate_isnew_in_a_request_is_false_on_a_chart_bar_inside_a_candle_that_opened_earlier() {
        // The live bar at hour 5 sits inside the 4h candle that opened at hour 4.
        let values = plotted_per_live_tick(
            "plot(request.security(syminfo.tickerid, \"240\", barstate.isnew ? 1.0 : 0.0))",
            &hourly_chart(0, 5),
            bar(5 * 3_600_000, 6.0),
            &[true, false],
        );
        assert_eq!(values, [Some(0.0), Some(0.0)]);
    }

    #[test]
    fn a_request_first_reached_on_a_live_close_replays_older_candles_as_history() {
        // ETHUSDT's hourly candles 0-3 all end by the live chart bar's close, and the request is
        // first reached on it: only the candle that opens with the chart bar is realtime.
        let candles = hourly_chart(0, 4);
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: "ETHUSDT".to_owned(),
                interval: 3600.0,
                recent_bars: 0,
            },
            candles.clone(),
        );
        let values = plotted_live_close(
            "plot(barstate.isrealtime ? request.security(\"ETHUSDT\", \"60\", ta.cum(barstate.isrealtime ? 1.0 : 0.0)) : na)",
            provider,
            &candles[..3],
            candles[3].clone(),
        );
        assert_eq!(values.last(), Some(&Some(1.0)));
    }

    #[test]
    fn a_candle_completed_late_on_a_live_bar_is_history() {
        // The chart has no 03:00 bar, so the 00:00 4h candle is completed when the live 04:00 bar
        // arrives; it is history, and only the developing 04:00 candle is realtime.
        let history: Vec<Kline> = [0, 1, 2]
            .into_iter()
            .map(|hour| bar(hour * 3_600_000, 1.0 + hour as f64))
            .collect();
        let values = plotted_live_close(
            "plot(request.security(syminfo.tickerid, \"240\", ta.cum(barstate.isrealtime ? 1.0 : 0.0)))",
            TestProvider::default(),
            &history,
            bar(4 * 3_600_000, 5.0),
        );
        assert_eq!(values.last(), Some(&Some(1.0)));
    }

    #[test]
    fn barstate_islast_in_a_request_holds_when_the_chart_bar_length_does_not_divide_the_day() {
        // 5h bars: the one at 20:00 ends at midnight, where the 5h candle is cut.
        let chart: Vec<Kline> = [0, 5, 10, 15, 20]
            .into_iter()
            .map(|hour| bar(hour * 3_600_000, 1.0 + hour as f64))
            .collect();
        for state in ["barstate.islast", "barstate.islastconfirmedhistory"] {
            let values = plotted(
                &format!("plot(request.security(syminfo.tickerid, \"300\", {state} ? 1.0 : 0.0))"),
                &chart,
            );
            assert_eq!(values[3], Some(0.0), "{state} on the 15:00 candle");
            assert_eq!(values[4], Some(1.0), "{state} on the 20:00 candle");
        }
    }

    /// The first plot of `source` over `chart` run as one history (`Session::evaluate`), which
    /// tells the session where the history ends.
    fn plotted_as_history(source: &str, chart: &[Kline]) -> Vec<Option<f64>> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        let program = compiler::compile(&body, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        session.evaluate(chart).expect("runs");
        session.output().plots[0].values.clone()
    }

    #[test]
    fn the_last_confirmed_history_candle_of_a_request_is_the_last_one_that_closed_with_the_history()
    {
        // TradingView (probe P12, 2026-11-21): on the chart's last history bar, `"1D"` returns the
        // last completed day, and that day is the request's last confirmed history bar.
        let lch = |expression: &str, bars: i64| {
            plotted_as_history(
                &format!(
                    "plot(request.security(syminfo.tickerid, \"{expression}\", barstate.islastconfirmedhistory ? 1.0 : 0.0))"
                ),
                &hourly_chart(0, bars),
            )
        };
        // Hours 0-29: day 0 closed at hour 24, day 1 is still running when the history ends.
        let days = lch("1D", 30);
        assert_eq!(days[22], None, "no day has closed yet");
        assert_eq!(days[23], Some(1.0), "day 0 closes with the chart's hour 23");
        assert_eq!(
            days[29],
            Some(1.0),
            "and is still what the last history bar reads"
        );
        // Hours 0-11: the 4h candle 8-12 ends with the history, the one before it does not.
        let hours = lch("240", 12);
        assert_eq!(hours[7], Some(0.0));
        assert_eq!(hours[11], Some(1.0));
    }

    /// `count` monthly chart bars from `first_year`-`first_month`, each opening on the 1st and closing on
    /// the next 1st (so 28 to 31 days long, where the chart's length is the nominal 30 days).
    fn monthly_chart(first_year: i64, first_month: i64, count: i64) -> Vec<Kline> {
        let start = |index: i64| {
            let month = first_month - 1 + index;
            crate::pine::time::calendar::make(
                first_year + month.div_euclid(12),
                month.rem_euclid(12) + 1,
                1,
                0,
                0,
                0,
                0,
            )
            .unwrap()
        };
        (0..count)
            .map(|index| Kline {
                open_time: start(index),
                close_time: start(index + 1),
                ..bar(0, 1.0 + index as f64)
            })
            .collect()
    }

    /// The first plot of `source` over `chart` as the app runs a monthly chart: one history load with
    /// the chart's length fixed at Pine's 30-day month.
    fn plotted_on_a_monthly_chart(source: &str, chart: &[Kline]) -> Vec<Option<f64>> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        let program = compiler::compile(&body, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(TestProvider::default())),
            None,
        );
        session.fixed_bar_seconds = Some(2_592_000.0);
        session.load_history(chart, false, None).expect("runs");
        session.output().plots[0].values.clone()
    }

    #[test]
    fn a_monthly_chart_completes_a_quarter_and_a_year_on_their_last_month() {
        // TradingView (probe R5, BTCUSDT 1M): the 3M and 12M candles arrive on the chart bar that
        // closes them. A 31-day month is longer than the chart's 30 days, so the bar's end, not its
        // open plus 30 days, says it closes the bucket.
        let quarter = plotted_on_a_monthly_chart(
            "plot(request.security(syminfo.tickerid, \"3M\", bar_index))",
            &monthly_chart(2023, 10, 6),
        );
        // Oct-Dec 2023 is candle 0, Jan-Mar 2024 candle 1: each is read from its last month on.
        assert_eq!(quarter[1], None, "Nov: the quarter is still open");
        assert_eq!(quarter[2], Some(0.0), "Dec closes Q4");
        assert_eq!(quarter[4], Some(0.0), "Feb still reads Q4");
        assert_eq!(quarter[5], Some(1.0), "Mar closes Q1");
        let year = plotted_on_a_monthly_chart(
            "plot(request.security(syminfo.tickerid, \"12M\", bar_index))",
            &monthly_chart(2022, 1, 24),
        );
        assert_eq!(year[10], None, "Nov 2022: the year is still open");
        assert_eq!(year[11], Some(0.0), "Dec 2022 closes 2022");
        assert_eq!(year[22], Some(0.0), "Nov 2023 still reads 2022");
        assert_eq!(year[23], Some(1.0), "Dec 2023 closes 2023");
    }

    #[test]
    fn a_developing_candle_on_history_is_confirmed_when_it_closes_by_the_end_of_the_history() {
        // TradingView (probe R1, 4h BTCUSDT, `"D"` with lookahead on): every historical chart bar
        // reads 1, the developing day included; the final incomplete bucket reads 0 (probes A, B).
        let states = |lookahead: &str| {
            plotted_as_history(
                &format!(
                    "plot(request.security(syminfo.tickerid, \"1D\", barstate.isconfirmed ? 1.0 : 0.0{lookahead}))"
                ),
                &hourly_chart(0, 30),
            )
        };
        // Hours 0-29: day 0 closes at hour 24, day 1 is still open when the history ends.
        let developing = states(", lookahead = barmerge.lookahead_on");
        assert_eq!(
            developing[5],
            Some(1.0),
            "day 0 develops, and closes inside the history"
        );
        assert_eq!(developing[23], Some(1.0));
        assert_eq!(
            developing[24],
            Some(0.0),
            "day 1 is still open when the history ends"
        );
        assert_eq!(developing[29], Some(0.0));
        let completed = states("");
        assert_eq!(completed[29], Some(1.0), "the last completed day");
    }

    #[test]
    fn a_candle_developing_in_real_time_is_not_confirmed() {
        // The history ends at hour 5; the live bar sits inside the 4h candle of hours 4-8.
        let source = "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"240\", barstate.isconfirmed ? 1.0 : 0.0, lookahead = barmerge.lookahead_on))";
        let program = compiler::compile(source, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(TestProvider::default())),
            None,
        );
        session
            .load_history(&hourly_chart(0, 5), true, Some(3600.0))
            .expect("runs");
        let _ = session.execute(
            BarEvent {
                candle: bar(5 * 3_600_000, 6.0),
                phase: BarPhase::RealtimeTick { is_new: true },
            },
            true,
            None,
        );
        let values = &session.output().plots[0].values;
        assert_eq!(values.last(), Some(&Some(0.0)));
    }

    #[test]
    fn a_script_variable_is_computed_on_the_requested_series() {
        // The chart's `doubled` is 6; ETHUSDT's is 240.
        assert_eq!(
            last_plotted("doubled = close * 2\nx = request.security(\"ETHUSDT\", \"1\", doubled)"),
            Some(240.0)
        );
    }

    #[test]
    fn a_chain_of_variables_keeps_its_indicator_state_and_history() {
        // `ta.sma` runs over ETHUSDT's closes (110, 120), and `[1]` reads that series' previous bar.
        assert_eq!(
            last_plotted("mean = ta.sma(close, 2)\nx = request.security(\"ETHUSDT\", \"1\", mean)"),
            Some(115.0)
        );
        assert_eq!(
            last_plotted(
                "doubled = close * 2\nx = request.security(\"ETHUSDT\", \"1\", doubled[1])"
            ),
            Some(220.0)
        );
    }

    #[test]
    fn a_reassigned_variable_and_a_function_that_reads_one_follow_the_series() {
        assert_eq!(
            last_plotted(
                "level = 0.0\nif close > 105\n    level := close\nx = request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(120.0)
        );
        assert_eq!(
            last_plotted(
                "factor = 2\nscaled() =>\n    close * factor\nx = request.security(\"ETHUSDT\", \"1\", scaled())"
            ),
            Some(240.0)
        );
    }

    #[test]
    fn a_helper_reads_the_requested_global_after_its_unused_branch() {
        // Only the helper reads `level`; the request still computes it on ETHUSDT.
        assert_eq!(
            last_plotted(
                "level = close\nf() =>\n    if false\n        level = 1.0\n    level\nx = request.security(\"ETHUSDT\", \"1\", f())"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_function_reads_the_requested_global_its_unused_branch_declares_again() {
        // The expression reads `level` itself, so the request computes it on ETHUSDT; the helper's
        // branch-local `level` must not hide that value after the branch.
        assert_eq!(
            last_plotted(
                "level = close\nf() =>\n    if false\n        level = 1.0\n    level\nx = request.security(\"ETHUSDT\", \"1\", level > 0 ? f() : 0.0)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_function_in_a_request_inside_a_function_reads_the_global_not_the_callers_local() {
        // A request inside a function is not sliced: `len` is the chart's global, never `f`'s own.
        assert_eq!(
            last_plotted(
                "len = 1.0\ng() =>\n    len\nf() =>\n    len = 50.0\n    request.security(\"ETHUSDT\", \"1\", g())\nx = f()"
            ),
            Some(1.0)
        );
    }

    #[test]
    fn code_after_the_request_is_not_part_of_what_it_computes() {
        // TradingView computes the variable as it stands where the request is written.
        assert_eq!(
            last_plotted(
                "value = close\nx = request.security(\"ETHUSDT\", \"1\", value)\nvalue := value * 2"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_collection_a_call_fills_is_filled_in_the_request_too() {
        let request = "x = request.security(\"ETHUSDT\", \"1\", ";
        // The array lives in the request's own state and gets one value per requested bar.
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\narray.push(samples, close)\n{request}array.size(samples))"
            )),
            Some(3.0)
        );
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\nsamples.push(close)\nn = samples.size()\n{request}n)"
            )),
            Some(3.0)
        );
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\narray.push(samples, close)\nfirst = array.get(samples, 0)\n{request}first)"
            )),
            Some(100.0),
            "no out-of-bounds read of an array nothing filled"
        );
    }

    #[test]
    fn overloads_run_only_the_dependencies_of_the_one_called() {
        // `bad` is only read by the string overload, and fails where the chart does not run it
        // (ETHUSDT closes above 50, the chart below).
        assert_eq!(
            last_plotted(
                "arr = array.new_float()\nbad = close > 50 ? array.get(arr, 5) : 0.0\nf(float v) =>\n    v * 2\nf(string s) =>\n    str.tostring(bad)\nx = request.security(\"ETHUSDT\", \"1\", f(close))"
            ),
            Some(240.0)
        );
    }

    #[test]
    fn a_function_that_only_reads_its_argument_is_not_replayed() {
        // `checked` fails on the requested series (ETHUSDT closes above 50) but not on the chart's.
        // It cannot change `level`, so the request has no business running it.
        assert_eq!(
            last_plotted(
                "id(a) =>\n    a\nchecked(v) =>\n    if v > 50\n        runtime.error(\"boom\")\n    v\nlevel = id(close)\nunused = checked(level)\nx = request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_collection_a_function_returned_is_still_filled() {
        assert_eq!(
            last_plotted(
                "makeArray() =>\n    array.new_float()\nvar samples = makeArray()\nsamples.push(close)\nn = samples.size()\nx = request.security(\"ETHUSDT\", \"1\", n)"
            ),
            Some(3.0)
        );
        assert_eq!(
            last_plotted(
                "makeArray() =>\n    array.new_float()\nfill(a) =>\n    a.push(close)\nvar samples = makeArray()\nfill(samples)\nn = array.size(samples)\nx = request.security(\"ETHUSDT\", \"1\", n)"
            ),
            Some(3.0),
            "a function that fills its parameter"
        );
    }

    #[test]
    fn a_local_in_a_block_is_not_resolved_to_the_global_with_its_name() {
        // The request sees the `level` of the block it is written in.
        assert_eq!(
            last_plotted(
                "level = 10.0\nvar float x = na\nif close > 0\n    level = 20.0\n    x := request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(20.0)
        );
    }

    #[test]
    fn a_read_only_overload_is_not_replayed_for_the_sake_of_a_mutating_one() {
        // `checked(float)` fails on the requested series; only `checked(array<float>)` changes
        // its parameter, and the call is the float one.
        assert_eq!(
            last_plotted(
                "checked(float v) =>\n    if v > 50\n        runtime.error(\"boom\")\n    v\nchecked(array<float> a) =>\n    array.push(a, 1.0)\n    1.0\nlevel = close\nunused = checked(level)\nx = request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_block_local_assignment_is_not_a_write_of_the_global() {
        // The block's own `level` is changed; the global is not, so the block stays out of the
        // request (it would raise the error on the requested series).
        assert_eq!(
            last_plotted(
                "level = close\nif close > 50\n    level = 5.0\n    level += 1\n    runtime.error(\"boom\")\nx = request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_collection_changed_through_an_alias_or_a_helper_is_filled() {
        let request = "x = request.security(\"ETHUSDT\", \"1\", array.size(samples))";
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\nalias = samples\nalias.push(close)\n{request}"
            )),
            Some(3.0),
            "through an alias"
        );
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\naddSample() =>\n    array.push(samples, close)\naddSample()\n{request}"
            )),
            Some(3.0),
            "a helper without arguments"
        );
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\naddSample() =>\n    array.push(samples, close)\nrecord() =>\n    addSample()\nrecord()\n{request}"
            )),
            Some(3.0),
            "a helper of a helper"
        );
    }

    #[test]
    fn a_block_reading_its_own_variable_does_not_pull_in_the_global() {
        // The global `bad` fails on the requested series (ETHUSDT closes above 50); the block reads
        // its own `bad`, so the request has no business running the global one.
        assert_eq!(
            last_plotted(
                "arr = array.new_float()\nbad = close > 50 ? array.get(arr, 5) : 0.0\nlevel = close\nif close > 0\n    bad = close\n    level := bad\nx = request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_collection_changed_through_a_local_alias_is_filled() {
        let request = "x = request.security(\"ETHUSDT\", \"1\", array.size(samples))";
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\nif close > 0\n    alias = samples\n    alias.push(close)\n{request}"
            )),
            Some(3.0),
            "an alias a block makes"
        );
        assert_eq!(
            last_plotted(&format!(
                "var samples = array.new_float()\nfill(a) =>\n    alias = a\n    alias.push(close)\nfill(samples)\n{request}"
            )),
            Some(3.0),
            "an alias a helper makes of its parameter"
        );
    }

    #[test]
    fn the_collection_of_a_builtin_is_found_by_label_or_position() {
        let request = "x = request.security(\"ETHUSDT\", \"1\", array.size(samples))";
        for call in [
            "array.push(value = close, id = samples)",
            "array.push(samples, value = close)",
            "array.push(samples, close)",
        ] {
            assert_eq!(
                last_plotted(&format!(
                    "var samples = array.new_float()\n{call}\n{request}"
                )),
                Some(3.0),
                "{call}"
            );
        }
    }

    /// A line that only runs, and fails, on the ETHUSDT series (its closes are above 50).
    const FAILS_ON_ETH: &str = "bad = close > 50 ? array.get(array.new_float(), 5) : 0.0\n";

    #[test]
    fn blocks_alias_names_do_not_merge_in_the_request() {
        assert_eq!(
            last_plotted(
                "var samples = array.new_float()\nvar other = array.new_float()\nif close > 0\n    alias = samples\n    alias.push(close)\nif close > 50\n    alias = other\n    alias.push(array.get(other, 5))\nx = request.security(\"ETHUSDT\", \"1\", array.size(samples))"
            ),
            Some(3.0)
        );
    }

    #[test]
    fn a_builtin_does_not_replay_the_script_function_of_its_name() {
        assert_eq!(
            last_plotted(&format!(
                "{FAILS_ON_ETH}sma(float src, int len) =>\n    src + bad\nx = request.security(\"ETHUSDT\", \"1\", ta.sma(close, 2))"
            )),
            Some(115.0)
        );
    }

    #[test]
    fn a_read_only_query_is_not_replayed_in_the_request() {
        assert_eq!(
            last_plotted(
                "var samples = array.new_float()\narray.push(samples, close)\nunused = close > 50 ? array.get(array.new_float(), 1) + array.percentrank(samples, 0) : 0.0\nx = request.security(\"ETHUSDT\", \"1\", array.size(samples))"
            ),
            Some(3.0)
        );
    }

    #[test]
    fn a_read_only_user_method_is_not_replayed_in_the_request() {
        assert_eq!(
            last_plotted(&format!(
                "{FAILS_ON_ETH}level = close\nmethod push(float this, float v) =>\n    v + bad\nr = level.push(1.0)\nx = request.security(\"ETHUSDT\", \"1\", level)"
            )),
            Some(120.0)
        );
    }

    #[test]
    fn a_matrix_predicate_is_not_replayed_in_the_request() {
        assert_eq!(
            last_plotted(&format!(
                "{FAILS_ON_ETH}m = matrix.new<float>(2, 2, 0.0)\nunused = matrix.is_zero(m) ? bad : 0.0\nx = request.security(\"ETHUSDT\", \"1\", matrix.rows(m))"
            )),
            Some(2.0)
        );
    }

    #[test]
    fn a_namespaced_builtin_does_not_replay_the_script_function_that_fills_a_global() {
        assert_eq!(
            last_plotted(&format!(
                "{FAILS_ON_ETH}var samples = array.new_float()\nsma(float src, int len) =>\n    array.push(samples, src)\n    src\nunused = bad + ta.sma(close, 2)\nx = request.security(\"ETHUSDT\", \"1\", array.size(samples))"
            )),
            Some(0.0)
        );
    }

    #[test]
    fn an_alias_reassigned_in_an_inner_block_is_filled_through_both_sources() {
        // On the chart (closes 1, 2, 3) `b` gets two values; on ETHUSDT (all above 1) three.
        assert_eq!(
            last_plotted(
                "var a = array.new_float()\nvar b = array.new_float()\nif close > 0\n    alias = a\n    if close > 1\n        alias := b\n    alias.push(close)\nx = request.security(\"ETHUSDT\", \"1\", array.size(b))"
            ),
            Some(3.0)
        );
    }

    #[test]
    fn a_global_a_callee_fills_is_not_the_callers_parameter_of_that_name() {
        // `fill` runs with its own lexical scope: it fills the global `samples`, not the `samples`
        // that `f` is handed (`other`). The chart does the same, so the request must too.
        let script = "var samples = array.new_float()\nvar other = array.new_float()\nfill() =>\n    array.push(samples, close)\n    1.0\nf(samples) =>\n    fill()\nf(other)\n";
        assert_eq!(
            last_plotted(&format!("{script}x = array.size(samples)")),
            Some(3.0),
            "on the chart"
        );
        assert_eq!(
            last_plotted(&format!(
                "{script}x = request.security(\"ETHUSDT\", \"1\", array.size(samples))"
            )),
            Some(3.0)
        );
        assert_eq!(
            last_plotted(&format!(
                "{script}x = request.security(\"ETHUSDT\", \"1\", array.size(other))"
            )),
            Some(0.0),
            "`other` is never filled"
        );
    }

    #[test]
    fn an_alias_an_expression_block_makes_is_followed() {
        // `alias := samples` sits in the block of `y = if …`; the push through `alias` fills
        // `samples`, on the chart (three bars) and in the request.
        let script = "var samples = array.new_float()\nvar alias = array.new_float()\ny = if close > 0\n    alias := samples\n    1.0\nelse\n    0.0\nalias.push(close)\n";
        assert_eq!(
            last_plotted(&format!("{script}x = array.size(samples)")),
            Some(3.0),
            "on the chart"
        );
        assert_eq!(
            last_plotted(&format!(
                "{script}x = request.security(\"ETHUSDT\", \"1\", array.size(samples))"
            )),
            Some(3.0)
        );
    }

    #[test]
    fn an_alias_made_after_the_mutation_does_not_make_the_request_replay_it() {
        // `a` and `b` are new on every bar. The chart fills and updates `a`; the request reads `b`
        // on 5m bars, where `a` is empty, so replaying `a.set(0, 7)` there would stop the script.
        // `a := b` comes after: it makes them one array only from there on.
        let head = "a = array.new_float()\nb = array.new_float()\nif close < 50\n    a.push(close)\na.set(0, 7.0)\n";
        let request = "x = request.security(\"ETHUSDT\", \"1\", array.size(b))";
        for copy in [
            "flag = if close > 0\n    a := b\n    1.0\nelse\n    0.0\n",
            "a := b\n",
        ] {
            assert_eq!(
                last_plotted(&format!("{head}{copy}{request}")),
                Some(0.0),
                "{copy}"
            );
        }
    }

    #[test]
    fn a_var_alias_written_after_the_mutation_still_counts() {
        // `a` is persistent: the copy `a := b` at the end of one bar is what the next bar's
        // `a.push` fills, so the request must replay that push to see `b` filled: on the second and
        // third bar, as on the chart.
        let script =
            "var a = array.new_float()\nvar b = array.new_float()\na.push(close)\na := b\n";
        assert_eq!(
            last_plotted(&format!("{script}x = array.size(b)")),
            Some(2.0),
            "on the chart"
        );
        assert_eq!(
            last_plotted(&format!(
                "{script}x = request.security(\"ETHUSDT\", \"1\", array.size(b))"
            )),
            Some(2.0)
        );
    }

    #[test]
    fn two_globals_copied_from_a_builtin_are_not_aliases() {
        // `a` and `b` both hold `close`, which is no object: filling one fills nothing, and what
        // the unreached branch does to `a` (it fails on ETHUSDT, whose closes are above 50) is not
        // a reason to replay it for the request that reads `b`.
        assert_eq!(
            last_plotted(
                "a = close\nb = close\nif close > 50\n    a.push(1.0)\nx = request.security(\"ETHUSDT\", \"1\", b)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_block_that_redeclares_a_name_keeps_what_it_assigned_before() {
        // `c0 := b` runs before the inner block's own `c0 = c0` and is the outer `c0`'s: while
        // `close > 1` the push below fills `b`, and `a` gets the other bars'. The chart's bars 2
        // and 3 qualify, and so do all of ETHUSDT's, so the request must fill `b` as well.
        let script = "var a = array.new_float()\nvar b = array.new_float()\nif close > 0\n    c0 = a\n    if close > 1\n        c0 := b\n        c0 = c0\n    c0.push(close)\n";
        // The same through a sibling branch that declares a `c0` of its own.
        let sibling = "var a = array.new_float()\nvar b = array.new_float()\nif close > 0\n    c0 = a\n    if close > 1\n        c0 := b\n    else\n        c0 = array.new_float()\n    c0.push(close)\n";
        for script in [script, sibling] {
            for (read, on_chart, requested) in [("a", 1.0, 0.0), ("b", 2.0, 3.0)] {
                assert_eq!(
                    last_plotted(&format!("{script}x = array.size({read})")),
                    Some(on_chart),
                    "{read} on the chart"
                );
                assert_eq!(
                    last_plotted(&format!(
                        "{script}x = request.security(\"ETHUSDT\", \"1\", array.size({read}))"
                    )),
                    Some(requested),
                    "{read} in the request"
                );
            }
        }
    }

    #[test]
    fn a_write_in_the_block_of_the_request_before_it_follows_the_series() {
        assert_eq!(
            last_plotted(
                "level = 0.0\nx = 0.0\nif close > 0\n    level := close\n    x := request.security(\"ETHUSDT\", \"1\", level)"
            ),
            Some(120.0)
        );
    }

    #[test]
    fn a_name_an_expression_statement_declares_is_read_as_declared() {
        assert_eq!(
            last_plotted(
                "level = close\nx = if close > 0\n    level = 20.0\n    request.security(\"ETHUSDT\", \"1\", level)\nelse\n    0.0"
            ),
            Some(20.0)
        );
    }

    #[test]
    fn a_variable_is_computed_as_it_stands_where_it_is_read() {
        let request = "x = request.security(\"ETHUSDT\", \"1\", snapshot)";
        // The later `level` fails on the requested series (ETHUSDT closes above 50).
        assert_eq!(
            last_plotted(&format!(
                "arr = array.new_float()\nlevel = close\nsnapshot = level\nlevel := close > 50 ? array.get(arr, 5) : close\n{request}"
            )),
            Some(120.0)
        );
        // A `var` and a history read see what the previous bar wrote after them.
        assert_eq!(
            last_plotted(&format!(
                "var level = 0.0\nsnapshot = level\nlevel := close\n{request}"
            )),
            Some(110.0)
        );
        assert_eq!(
            last_plotted(&format!(
                "level = close\nsnapshot = level[1]\nlevel := level + 1\n{request}"
            )),
            Some(111.0)
        );
    }

    /// Runs `source` on one chart bar that gets two live ticks, as the session does for a bar that
    /// has not closed, and returns what `read` makes of the session after each tick.
    fn after_each_tick<T>(source: &str, read: impl Fn(&Session) -> T) -> Vec<T> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        let program = compiler::compile(&body, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(eth_provider())),
            None,
        );
        [true, false]
            .into_iter()
            .map(|is_new| {
                let _ = session.execute(
                    BarEvent {
                        candle: bar(0, 1.0),
                        phase: BarPhase::RealtimeTick { is_new },
                    },
                    true,
                    None,
                );
                read(&session)
            })
            .collect()
    }

    /// The last value of the first plot after each tick.
    fn plotted_after_each_tick(source: &str) -> Vec<Option<f64>> {
        after_each_tick(source, |session| {
            session.output().plots[0].values.last().copied().flatten()
        })
    }

    /// The messages of the alerts the session emitted for each tick.
    fn alerts_after_each_tick(source: &str) -> Vec<Vec<String>> {
        after_each_tick(source, |session| {
            session
                .emitted_alerts
                .iter()
                .map(|alert| alert.message.clone())
                .collect()
        })
    }

    #[test]
    fn a_request_does_not_touch_the_charts_varip_state() {
        let counter = "varip n = 0\nn += 1\nplot(n)\n";
        let alone = plotted_after_each_tick(counter);
        assert_eq!(alone, [Some(1.0), Some(2.0)]);
        // The same counter, read by a request whose result nothing uses.
        let requested = plotted_after_each_tick(&format!(
            "{counter}ignored = request.security(\"ETHUSDT\", \"1\", n)"
        ));
        assert_eq!(requested, alone);
    }

    #[test]
    fn an_alert_in_a_slice_does_not_fire_from_the_requested_series() {
        let request = "ignored = request.security(\"ETHUSDT\", \"1\", level)";
        // The slice replays the whole `if`, alert included; only the chart's own run fires it.
        assert_eq!(
            alerts_after_each_tick(&format!(
                "level = 0.0\nif close > 0\n    level := close\n    alert(\"hi\", alert.freq_all)\n{request}"
            )),
            [["hi"], ["hi"]]
        );
        assert_eq!(
            alerts_after_each_tick(&format!(
                "level = 0.0\nnotify() =>\n    alert(\"hi\", alert.freq_all)\n    close\nif close > 0\n    level := notify()\n{request}"
            )),
            [["hi"], ["hi"]],
            "through a function the slice calls"
        );
    }

    #[test]
    fn a_request_does_not_use_up_the_once_per_bar_alert() {
        // The request runs `notify` first: on ETHUSDT (closes above 50) the message would read `true`
        // and the chart's own call would find the bar's alert already given.
        assert_eq!(
            alerts_after_each_tick(
                "notify() =>\n    alert(str.tostring(close > 50), alert.freq_once_per_bar)\n    close\nignored = request.security(\"ETHUSDT\", \"1\", notify())\nplot(notify())"
            ),
            [vec!["false"], vec![]]
        );
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
    /// Every plot of `source` over `chart` loaded as the app loads a history: with the chart's bar
    /// length fixed at `seconds`, and when `live` is given (a bar and the wall-clock time) with that
    /// bar ticking after it.
    fn loaded(
        source: &str,
        provider: TestProvider,
        chart: &[Kline],
        seconds: f64,
        live: Option<(Kline, i64)>,
    ) -> Vec<Vec<Option<f64>>> {
        let body = format!("//@version=6\nindicator(\"T\")\n{source}");
        let program = compiler::compile(&body, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            Some(Box::new(provider)),
            None,
        );
        session.fixed_bar_seconds = Some(seconds);
        session.now_ms = live.as_ref().map(|(_, now)| *now);
        session
            .load_history(chart, live.is_some(), Some(seconds))
            .expect("runs");
        if let Some((candle, _)) = live {
            let _ = session.execute(
                BarEvent {
                    candle,
                    phase: BarPhase::RealtimeTick { is_new: true },
                },
                true,
                None,
            );
        }
        session
            .output()
            .plots
            .iter()
            .map(|plot| plot.values.clone())
            .collect()
    }

    fn finer_candles(interval: f64, from: i64, until: i64) -> TestProvider {
        let step = (interval * 1000.0) as i64;
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: String::new(),
                interval,
                recent_bars: 0,
            },
            (0..)
                .map(|index| from + index * step)
                .take_while(|open| *open <= until)
                .map(|open| bar(open, open as f64))
                .collect(),
        );
        provider
    }

    #[test]
    fn security_to_a_finer_timeframe_with_lookahead_reads_the_first_intrabar() {
        // TradingView, probe S1: an AAPL 1D chart reads a "60" request's last candle of the day
        // (15:30), and its first one (9:30) with lookahead_on.
        let provider = || finer_candles(15.0, 60_000, 105_000);
        let chart = [bar(0, 10.0), bar(60_000, 11.0)];
        let source = |lookahead: &str| {
            format!(
                "//@version=6\nindicator(\"T\")\nplot(request.security(syminfo.tickerid, \"15S\", close{lookahead}))"
            )
        };
        let off = run(&source(""), provider(), &chart);
        assert_eq!(off.plots[0].values.last(), Some(&Some(105_000.0)));
        let on = run(
            &source(", lookahead = barmerge.lookahead_on"),
            provider(),
            &chart,
        );
        assert_eq!(on.plots[0].values.last(), Some(&Some(60_000.0)));
    }

    #[test]
    fn a_stock_bar_reads_the_candles_that_end_inside_it_not_the_next_sessions() {
        // TradingView, probes R2 and S1: a "240" request on an AAPL 1D chart reads the last candle
        // of the day, whose predecessor is on the same day. The daily bar opens at 14:30 UTC and
        // the 4h candles are cut at 12, 16 and 20 UTC, so the next day's 12:00 candle opens before
        // the bar's nominal end but ends after the session.
        let hour = 3_600_000;
        let day = 24 * hour;
        let chart: Vec<Kline> = (0..3)
            .map(|d| bar(d * day + 14 * hour + hour / 2, 1.0))
            .collect();
        let mut provider = TestProvider::default();
        provider.candles.insert(
            SecurityKey {
                symbol: String::new(),
                interval: 14_400.0,
                recent_bars: 0,
            },
            (0..4)
                .flat_map(|d| [12, 16, 20].map(|h| bar(d * day + h * hour, 1.0)))
                .collect(),
        );
        let plots = loaded(
            "plot(request.security(syminfo.tickerid, \"240\", time))\n\
             plot(request.security(syminfo.tickerid, \"240\", timeframe.change(\"D\") ? 1.0 : 0.0))",
            provider,
            &chart,
            86_400.0,
            None,
        );
        let last_candle: Vec<_> = (0..3).map(|d| Some((d * day + 20 * hour) as f64)).collect();
        assert_eq!(plots[0], last_candle);
        assert_eq!(
            plots[1],
            [Some(0.0); 3],
            "the last candle of a day opens no day"
        );
    }

    #[test]
    fn a_monthly_bar_reads_the_candles_of_its_own_month() {
        // TradingView, probe R5: a monthly bar is a nominal 30 days, and October's last day still
        // belongs to it.
        let chart = monthly_chart(2026, 9, 3);
        let day = 86_400_000;
        let first = chart[0].open_time;
        let provider = finer_candles(86_400.0, first, chart[2].open_time + 29 * day);
        let plots = loaded(
            "plot(request.security(syminfo.tickerid, \"1D\", time))",
            provider,
            &chart,
            2_592_000.0,
            None,
        );
        let last_day = |month: usize| Some((chart[month + 1].open_time - day) as f64);
        assert_eq!(plots[0][0], last_day(0));
        assert_eq!(plots[0][1], last_day(1), "31 days in October");
    }

    #[test]
    fn a_finer_requests_last_confirmed_history_candle_is_the_last_to_close_before_the_load() {
        // TradingView, probe S2 (BTCUSDT 4h, loaded at 13:57 inside the live bar 12:00-16:00): of
        // the 15m candles of the live bar the 13:30 one is the last confirmed history candle, the
        // 13:45 one forming at the load is the last and the only realtime one; the chart's last
        // history bar has none of them.
        let hour = 3_600_000;
        let minute = 60_000;
        let chart = hourly_chart(0, 4);
        let provider = finer_candles(900.0, 0, 4 * hour + 30 * minute);
        let plots = loaded(
            "plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.islastconfirmedhistory ? 1.0 : 0.0), 1.0))\n\
             plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.islast ? 1.0 : 0.0), 1.0))\n\
             plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.isrealtime ? 1.0 : 0.0), 1.0))",
            provider,
            &chart,
            3_600.0,
            Some((bar(4 * hour, 1.0), 4 * hour + 40 * minute)),
        );
        let flagged = |plot: usize, at: usize| plots[plot][at];
        for plot in 0..3 {
            assert_eq!(
                flagged(plot, 3),
                Some(-1.0),
                "the last history bar, plot {plot}"
            );
        }
        assert_eq!(
            flagged(0, 4),
            Some(1.0),
            "last confirmed history: the 4:15 candle"
        );
        assert_eq!(flagged(1, 4), Some(2.0), "last: the 4:30 candle");
        assert_eq!(
            flagged(2, 4),
            Some(2.0),
            "realtime: the 4:30 candle forming at the load"
        );
    }

    #[test]
    fn the_last_candle_of_a_request_is_the_last_on_every_chart_bar_that_reads_it() {
        // TradingView, probes A, R3 and R4: a "D" request with lookahead_on reads the day still
        // forming with islast true on every chart bar of that day, not on the chart's last bar only.
        let chart = hourly_chart(0, 6);
        let plots = loaded(
            "plot(request.security(syminfo.tickerid, \"240\", barstate.islast ? 1.0 : 0.0, lookahead = barmerge.lookahead_on))",
            TestProvider::default(),
            &chart,
            3_600.0,
            None,
        );
        let flags: Vec<f64> = plots[0].iter().map(|value| value.unwrap()).collect();
        assert_eq!(flags, [0.0, 0.0, 0.0, 0.0, 1.0, 1.0]);
    }
}
