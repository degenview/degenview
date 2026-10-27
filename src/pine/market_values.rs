//! The market/symbol/timeframe identifiers the session's `market()` does not already handle,
//! ported from the tail of `PineRuntimeSession+Market.swift`. Pure: everything it needs to resolve
//! a name arrives in the [`MarketContext`], so the session folds [`read`] into its identifier
//! resolver. The `strategy.*` reads live in `strategy_builtins::strategy_value`.

use super::program::SymbolInfo;
use super::runtime_value::RuntimeValue;
use super::state::RuntimeState;
use super::ta;
use super::time;
use super::trading_session::MarketSessions;
use crate::model::{ChartKind, Kline};

const MILLIS_PER_DAY: i64 = 86_400_000;

/// What [`read`] needs from the session to resolve a market identifier.
pub struct MarketContext<'a> {
    pub state: &'a RuntimeState,
    pub bar: &'a Kline,
    pub symbol: &'a SymbolInfo,
    pub bar_seconds: f64,
    pub visible_range: Option<(i64, i64)>,
    pub first_bar_time: Option<i64>,
    pub last_bar_time: Option<i64>,
    /// The chart's foreground and background, read as `chart.fg_color` / `chart.bg_color`.
    pub theme: super::program::ChartTheme,
    /// The length of the chart's own bars, which `timeframe.main_period` keeps inside a
    /// `request.*()` call that reads another timeframe.
    pub chart_bar_seconds: f64,
    /// Whether the script runs for an alert, not on a chart.
    pub alert_mode: bool,
}

/// The value of a market/symbol/timeframe identifier, or None when `name` is not one of them.
pub fn read(name: &str, context: &MarketContext) -> Option<RuntimeValue> {
    match name {
        "time_tradingday" => Some(RuntimeValue::Int(
            context.bar.open_time.div_euclid(MILLIS_PER_DAY) * MILLIS_PER_DAY,
        )),
        "timenow" => Some(RuntimeValue::Int(now_millis())),
        "ta.tr" => Some(RuntimeValue::Float(ta::true_range(
            context.bar,
            previous_close(context.state),
        ))),
        "syminfo.prefix" => Some(
            exchange_prefix(context.symbol)
                .map(RuntimeValue::String)
                .unwrap_or(RuntimeValue::Na),
        ),
        "syminfo.timezone" => Some(RuntimeValue::String(
            MarketSessions::for_kind(&context.symbol.kind)
                .zone()
                .to_owned(),
        )),
        "syminfo.session" => Some(RuntimeValue::String("session.regular".to_owned())),
        "syminfo.main_tickerid" => Some(RuntimeValue::String(context.symbol.ticker_id.clone())),
        // Neither crypto pairs nor stocks are futures contracts.
        "syminfo.current_contract" | "syminfo.expiration_date" => Some(RuntimeValue::Na),
        "runtime.calculation.mode" => Some(RuntimeValue::String(
            if context.alert_mode {
                "runtime.calculation.mode.alert"
            } else {
                "runtime.calculation.mode.chart"
            }
            .to_owned(),
        )),
        "timeframe.main_period" => time::timeframe(name, context.chart_bar_seconds),
        "session.isfirstbar"
        | "session.isfirstbar_regular"
        | "session.islastbar"
        | "session.islastbar_regular"
        | "session.ismarket"
        | "session.ispremarket"
        | "session.ispostmarket" => Some(RuntimeValue::Bool(session_flag(name, context))),
        // Every standard chart type reports as standard: the script sees the real OHLC however
        // the chart draws it. The price-based types are their own kinds.
        "chart.is_standard" => Some(RuntimeValue::Bool(context.symbol.chart.is_standard())),
        "chart.is_heikinashi" => Some(chart_is(context, ChartKind::HeikinAshi)),
        "chart.is_renko" => Some(chart_is(context, ChartKind::Renko)),
        "chart.is_kagi" => Some(chart_is(context, ChartKind::Kagi)),
        "chart.is_linebreak" => Some(chart_is(context, ChartKind::LineBreak)),
        "chart.is_pnf" => Some(chart_is(context, ChartKind::PointAndFigure)),
        "chart.is_range" => Some(chart_is(context, ChartKind::Range)),
        "chart.left_visible_bar_time" => Some(
            context
                .visible_range
                .map(|range| range.0)
                .or(context.first_bar_time)
                .map(RuntimeValue::Int)
                .unwrap_or(RuntimeValue::Na),
        ),
        "chart.right_visible_bar_time" => Some(
            context
                .visible_range
                .map(|range| range.1)
                .or(context.last_bar_time)
                .map(RuntimeValue::Int)
                .unwrap_or(RuntimeValue::Na),
        ),
        "chart.fg_color" => Some(RuntimeValue::Color(context.theme.foreground)),
        "chart.bg_color" => Some(RuntimeValue::Color(context.theme.background)),
        _ if name.starts_with("timeframe.") => time::timeframe(name, context.bar_seconds),
        // Declared-but-unimplemented variables (`syminfo.description`, `syminfo.sector`, …) read as `na`.
        _ if super::symbol_catalog::is_unimplemented_variable(name) => Some(RuntimeValue::Na),
        _ => None,
    }
}

/// The `session.*` flags of the bar being run. The app's charts show the regular session, so
/// `session.isfirstbar` and `islastbar` mean the same as their `_regular` forms. A bar of a day or
/// more is a whole session of its own.
fn session_flag(name: &str, context: &MarketContext) -> bool {
    let sessions = MarketSessions::for_kind(&context.symbol.kind);
    let open = context.bar.open_time;
    let length = (context.bar_seconds * 1_000.0).round() as i64;
    let intraday = context.bar_seconds > 0.0 && context.bar_seconds < 86_400.0;
    match name {
        "session.ismarket" => !intraday || sessions.is_regular(open),
        "session.ispremarket" => intraday && sessions.is_premarket(open),
        "session.ispostmarket" => intraday && sessions.is_postmarket(open),
        "session.isfirstbar" | "session.isfirstbar_regular" => {
            let previous = context
                .state
                .histories
                .get("time")
                .and_then(|times| times.last())
                .and_then(RuntimeValue::int_value);
            !intraday || sessions.starts_regular_session(open, previous)
        }
        _ => !intraday || sessions.ends_regular_session(open, length),
    }
}

fn chart_is(context: &MarketContext, kind: ChartKind) -> RuntimeValue {
    RuntimeValue::Bool(context.symbol.chart == kind)
}

fn previous_close(state: &RuntimeState) -> Option<f64> {
    state
        .histories
        .get("close")?
        .last()
        .and_then(RuntimeValue::number)
}

/// The exchange part of `<exchange>:<ticker>`, upper-cased as TradingView shows it.
fn exchange_prefix(symbol: &SymbolInfo) -> Option<String> {
    let (prefix, _) = symbol.ticker_id.split_once(':')?;
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_uppercase())
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        state: RuntimeState,
        bar: Kline,
        symbol: SymbolInfo,
        bar_seconds: f64,
        visible_range: Option<(i64, i64)>,
        first_bar_time: Option<i64>,
        last_bar_time: Option<i64>,
        chart_bar_seconds: Option<f64>,
    }

    impl Fixture {
        fn new(open_time: i64, bar_seconds: f64) -> Self {
            Self {
                state: RuntimeState::default(),
                bar: Kline {
                    open_time,
                    close_time: open_time + 60_000,
                    open: 10.0,
                    high: 15.0,
                    low: 8.0,
                    close: 12.0,
                    volume: 100.0,
                },
                symbol: SymbolInfo {
                    ticker: "BTCUSDT".to_owned(),
                    ticker_id: "BINANCE:BTCUSDT".to_owned(),
                    ..SymbolInfo::default()
                },
                bar_seconds,
                visible_range: None,
                first_bar_time: Some(1_000),
                last_bar_time: Some(2_000),
                chart_bar_seconds: None,
            }
        }

        fn read(&self, name: &str) -> Option<RuntimeValue> {
            read(
                name,
                &MarketContext {
                    state: &self.state,
                    bar: &self.bar,
                    symbol: &self.symbol,
                    bar_seconds: self.bar_seconds,
                    visible_range: self.visible_range,
                    first_bar_time: self.first_bar_time,
                    last_bar_time: self.last_bar_time,
                    theme: crate::pine::program::ChartTheme::DARK,
                    chart_bar_seconds: self.chart_bar_seconds.unwrap_or(self.bar_seconds),
                    alert_mode: false,
                },
            )
        }
    }

    #[test]
    fn trading_day_floors_to_utc_midnight() {
        // 2024-05-01 13:45:00 UTC = 1714567500000 ms; the day starts 1714521600000.
        let fixture = Fixture::new(1_714_567_500_000, 60.0);
        assert_eq!(
            fixture.read("time_tradingday"),
            Some(RuntimeValue::Int(1_714_521_600_000))
        );
    }

    #[test]
    fn true_range_uses_the_previous_close() {
        let mut fixture = Fixture::new(1_000, 60.0);
        fixture
            .state
            .histories
            .insert("close".to_owned(), vec![RuntimeValue::Float(9.0)]);
        // max(high, prev_close) - min(low, prev_close) = 15 - 8 = 7.
        assert_eq!(fixture.read("ta.tr"), Some(RuntimeValue::Float(7.0)));
    }

    #[test]
    fn symbol_and_chart_facts() {
        let fixture = Fixture::new(1_000, 60.0);
        assert_eq!(
            fixture.read("syminfo.prefix"),
            Some(RuntimeValue::String("BINANCE".to_owned()))
        );
        assert_eq!(
            fixture.read("syminfo.timezone"),
            Some(RuntimeValue::String("Etc/UTC".to_owned()))
        );
        assert_eq!(
            fixture.read("chart.is_standard"),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            fixture.read("chart.is_renko"),
            Some(RuntimeValue::Bool(false))
        );
        assert_eq!(fixture.read("syminfo.nope"), None);
    }

    #[test]
    fn visible_bar_times_fall_back_to_the_run_bounds() {
        let mut fixture = Fixture::new(1_000, 60.0);
        assert_eq!(
            fixture.read("chart.left_visible_bar_time"),
            Some(RuntimeValue::Int(1_000))
        );
        assert_eq!(
            fixture.read("chart.right_visible_bar_time"),
            Some(RuntimeValue::Int(2_000))
        );
        fixture.visible_range = Some((500, 5_000));
        assert_eq!(
            fixture.read("chart.left_visible_bar_time"),
            Some(RuntimeValue::Int(500))
        );
        assert_eq!(
            fixture.read("chart.right_visible_bar_time"),
            Some(RuntimeValue::Int(5_000))
        );
    }

    #[test]
    fn main_period_stays_the_charts_while_the_period_follows_a_request() {
        let mut fixture = Fixture::new(1_000, 300.0);
        fixture.chart_bar_seconds = Some(60.0);
        assert_eq!(
            fixture.read("timeframe.period"),
            Some(RuntimeValue::String("5".to_owned()))
        );
        assert_eq!(
            fixture.read("timeframe.main_period"),
            Some(RuntimeValue::String("1".to_owned()))
        );
        assert_eq!(Fixture::new(1_000, 0.0).read("timeframe.main_period"), None);
    }

    #[test]
    fn the_chart_theme_and_unimplemented_names_read() {
        let fixture = Fixture::new(1_000, 60.0);
        assert_eq!(
            fixture.read("chart.fg_color"),
            Some(RuntimeValue::Color(0xd1d4_dcff)),
            "the chart's colours are the theme's"
        );
        assert_eq!(
            fixture.read("chart.bg_color"),
            Some(RuntimeValue::Color(0x1317_22ff))
        );
        assert_eq!(fixture.read("syminfo.description"), Some(RuntimeValue::Na));
        assert_eq!(fixture.read("syminfo.isin"), Some(RuntimeValue::Na));
    }
}
