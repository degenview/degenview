//! `alert(message, freq)` and `alertcondition(condition, title, message)`, ported from
//! `PineRuntimeSession+Alerts.swift`. Both record an `AlertEvent` in `RuntimeState` (capped), and
//! return the event to emit when it fired on a realtime bar — the session owns the emission channel
//! and the per-bar once-only ledger, so both are passed in.

pub mod call;
pub mod frequency;

use std::collections::HashSet;

use self::frequency::AlertFrequency;
use super::number_format;
use crate::model::Kline;
use crate::model::alert::message::{AlertMessageContext, render};
use crate::pine::compiler::program::SymbolInfo;
use crate::pine::outputs::drawings::{Arg, bind};
use crate::pine::runtime::state::{AlertEvent, RuntimeState};
use crate::pine::runtime::types::BarFlags;
use crate::pine::runtime::value::RuntimeValue;
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourceRange};

const ALERT_LIMIT: usize = 200;

/// Whether `name` is an alert function this module handles.
pub fn handles(name: &str) -> bool {
    matches!(name, "alert" | "alertcondition")
}

/// Runs one alert call. Returns the event to emit (`Some`) when it fired on a realtime bar; the
/// event is already appended to `state.alerts`. `ledger` is the session's once-per-bar site set.
#[allow(clippy::too_many_arguments)]
pub fn call(
    name: &str,
    args: &[Arg],
    site: usize,
    state: &mut RuntimeState,
    flags: &BarFlags,
    ledger: &mut HashSet<usize>,
    symbol: &SymbolInfo,
    mintick: f64,
    bar_seconds: f64,
    bar: &Kline,
    range: SourceRange,
) -> Result<Option<AlertEvent>, Diagnostic> {
    if name == "alert" {
        let b = bind(args, &["message", "freq"]);
        let message = b
            .get("message")
            .map(|value| number_format::format(value, None, mintick))
            .unwrap_or_default();
        let mut frequency = AlertFrequency::OncePerBar;
        if let Some(name) = b.get("freq").and_then(RuntimeValue::text_value) {
            frequency = AlertFrequency::from_pine_name(name).ok_or_else(|| {
                runtime_error(
                    "PINE4009",
                    format!("alert() frequency must be an alert.freq_* constant, not '{name}'."),
                    range,
                )
            })?;
        }
        Ok(record(state, site, message, frequency, flags, ledger, bar))
    } else {
        let b = bind(args, &["condition", "title", "message"]);
        if b.get("condition").and_then(RuntimeValue::bool) != Some(true) {
            return Ok(None);
        }
        let template = b
            .get("message")
            .and_then(RuntimeValue::text_value)
            .or_else(|| b.get("title").and_then(RuntimeValue::text_value))
            .unwrap_or("");
        let message = render(template, &message_context(symbol, bar, flags, bar_seconds));
        Ok(record(
            state,
            site,
            message,
            AlertFrequency::All,
            flags,
            ledger,
            bar,
        ))
    }
}

fn record(
    state: &mut RuntimeState,
    site: usize,
    message: String,
    frequency: AlertFrequency,
    flags: &BarFlags,
    ledger: &mut HashSet<usize>,
    bar: &Kline,
) -> Option<AlertEvent> {
    if frequency == AlertFrequency::OncePerBarClose && !flags.is_confirmed {
        return None;
    }
    if frequency == AlertFrequency::OncePerBar && !ledger.insert(site) {
        return None;
    }
    let event = AlertEvent {
        id: allocate(state),
        site,
        bar: state.bar_index as usize,
        time: bar.open_time,
        message,
        frequency,
        is_realtime: flags.is_realtime,
        is_confirmed: flags.is_confirmed,
    };
    state.alerts.push(event.clone());
    if state.alerts.len() > ALERT_LIMIT {
        let excess = state.alerts.len() - ALERT_LIMIT;
        state.alerts.drain(..excess);
    }
    event.is_realtime.then_some(event)
}

fn allocate(state: &mut RuntimeState) -> usize {
    state.next_reference += 1;
    state.next_reference
}

/// The values `{{ticker}}`, `{{close}}` etc. mean for the bar being executed.
fn message_context(
    symbol: &SymbolInfo,
    bar: &Kline,
    flags: &BarFlags,
    bar_seconds: f64,
) -> AlertMessageContext {
    AlertMessageContext {
        ticker: (!symbol.ticker.is_empty()).then(|| symbol.ticker.clone()),
        exchange: exchange_prefix(symbol),
        interval: interval_label(bar_seconds),
        open: Some(bar.open),
        high: Some(bar.high),
        low: Some(bar.low),
        close: Some(bar.close),
        volume: Some(bar.volume),
        time_ms: Some(bar.open_time),
        now_ms: if flags.is_realtime {
            now_millis()
        } else {
            bar.open_time
        },
    }
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

/// TradingView's name for a bar length: minutes (`60`), then `D`, `W`, `M`.
fn interval_label(bar_seconds: f64) -> Option<String> {
    if bar_seconds < 60.0 {
        return None;
    }
    let day = 86_400.0;
    if bar_seconds < day {
        return Some(((bar_seconds / 60.0) as i64).to_string());
    }
    let days = (bar_seconds / day).round() as i64;
    Some(match days {
        1 => "D".to_owned(),
        7 => "W".to_owned(),
        28..=31 => "M".to_owned(),
        88..=93 => "3M".to_owned(),
        360..=370 => "12M".to_owned(),
        _ => format!("{days}D"),
    })
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

fn runtime_error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Bool, Float, String as Str};

    fn arg(value: RuntimeValue) -> Arg {
        Arg { name: None, value }
    }

    fn s(text: &str) -> RuntimeValue {
        Str(text.to_owned())
    }

    fn bar() -> Kline {
        Kline {
            open_time: 1_000,
            close_time: 2_000,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 10.0,
        }
    }

    fn symbol() -> SymbolInfo {
        SymbolInfo {
            ticker: "BTCUSDT".to_owned(),
            ticker_id: "BINANCE:BTCUSDT".to_owned(),
            ..SymbolInfo::default()
        }
    }

    fn realtime() -> BarFlags {
        BarFlags {
            is_realtime: true,
            is_confirmed: true,
            ..BarFlags::default()
        }
    }

    fn run(
        state: &mut RuntimeState,
        ledger: &mut HashSet<usize>,
        name: &str,
        args: &[Arg],
        flags: &BarFlags,
    ) -> Result<Option<AlertEvent>, Diagnostic> {
        call(
            name,
            args,
            0,
            state,
            flags,
            ledger,
            &symbol(),
            0.01,
            3600.0,
            &bar(),
            SourceRange::ZERO,
        )
    }

    #[test]
    fn alert_records_and_returns_the_event_on_realtime() {
        let mut state = RuntimeState::default();
        let mut ledger = HashSet::new();
        let event = run(
            &mut state,
            &mut ledger,
            "alert",
            &[arg(s("hi"))],
            &realtime(),
        )
        .unwrap();
        assert!(event.is_some());
        assert_eq!(state.alerts.len(), 1);
        assert_eq!(state.alerts[0].message, "hi");
        assert_eq!(state.alerts[0].frequency, AlertFrequency::OncePerBar);
    }

    #[test]
    fn once_per_bar_fires_once_per_site() {
        let mut state = RuntimeState::default();
        let mut ledger = HashSet::new();
        let flags = realtime();
        run(&mut state, &mut ledger, "alert", &[arg(s("a"))], &flags).unwrap();
        let second = run(&mut state, &mut ledger, "alert", &[arg(s("b"))], &flags).unwrap();
        assert!(second.is_none());
        assert_eq!(state.alerts.len(), 1);
    }

    #[test]
    fn once_per_bar_close_skips_unconfirmed() {
        let mut state = RuntimeState::default();
        let mut ledger = HashSet::new();
        let flags = BarFlags {
            is_realtime: false,
            is_confirmed: false,
            ..BarFlags::default()
        };
        let event = run(
            &mut state,
            &mut ledger,
            "alert",
            &[arg(s("x")), arg(s("alert.freq_once_per_bar_close"))],
            &flags,
        )
        .unwrap();
        assert!(event.is_none());
        assert!(state.alerts.is_empty());
    }

    #[test]
    fn an_unknown_frequency_is_pine4009() {
        let mut state = RuntimeState::default();
        let mut ledger = HashSet::new();
        let error = run(
            &mut state,
            &mut ledger,
            "alert",
            &[arg(s("x")), arg(s("alert.freq_nope"))],
            &realtime(),
        )
        .unwrap_err();
        assert_eq!(error.code, "PINE4009");
    }

    #[test]
    fn alertcondition_renders_the_template() {
        let mut state = RuntimeState::default();
        let mut ledger = HashSet::new();
        run(
            &mut state,
            &mut ledger,
            "alertcondition",
            &[arg(Bool(true)), arg(s("T")), arg(s("{{ticker}} {{close}}"))],
            &realtime(),
        )
        .unwrap();
        assert_eq!(state.alerts[0].message, "BTCUSDT 1.5");
        assert_eq!(state.alerts[0].frequency, AlertFrequency::All);
    }

    #[test]
    fn interval_labels_follow_trading_views_names() {
        assert_eq!(interval_label(60.0).as_deref(), Some("1"));
        assert_eq!(interval_label(3600.0).as_deref(), Some("60"));
        assert_eq!(interval_label(86_400.0).as_deref(), Some("D"));
        assert_eq!(interval_label(7.0 * 86_400.0).as_deref(), Some("W"));
        assert_eq!(interval_label(30.0 * 86_400.0).as_deref(), Some("M"));
        assert_eq!(interval_label(10.0), None);
    }

    #[test]
    fn handles_covers_alert_names() {
        assert!(handles("alert") && handles("alertcondition"));
        assert!(!handles("plot") && !handles("strategy.entry"));
    }
}
