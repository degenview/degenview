//! Expands TradingView-style `{{placeholder}}` tokens in alert messages.
//!
//! One left-to-right pass over exact `{{name}}` tokens: substituted values are never
//! scanned again, an unknown name or an unclosed `{{` stays as written, and text around a
//! token is untouched. Rendering finishes before the Content-Type is decided, so a
//! template that only becomes JSON once its values are in still goes out as JSON.

use crate::format::civil_from_days;

/// What a price alert with a webhook but no message of its own posts.
pub const DEFAULT_PRICE_ALERT_TEMPLATE: &str = "{{ticker}} alert: {{close}}";

/// Every placeholder the renderer can resolve; documentation and the editors read this.
pub const SUPPORTED_PLACEHOLDERS: [&str; 10] = [
    "ticker", "exchange", "open", "high", "low", "close", "volume", "time", "timenow", "interval",
];

/// Values known when an alert triggers. Anything the trigger did not carry stays `None`,
/// and its placeholder is left in the text instead of being invented.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlertMessageContext {
    pub ticker: Option<String>,
    pub exchange: Option<String>,
    /// TradingView-style: `1`, `60`, `D`. See `trading_view_interval`.
    pub interval: Option<String>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    /// Bar open time, epoch ms.
    pub time_ms: Option<i64>,
    /// When the message is rendered, epoch ms.
    pub now_ms: i64,
}

pub fn render(template: &str, context: &AlertMessageContext) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("}}")
            && let Some(value) = value(&after[..end], context)
        {
            output.push_str(&value);
            rest = &after[end + 2..];
            continue;
        }
        // Not a token we know: keep one brace and look again from the next, so `{{{{close}}`
        // still finds the real token.
        output.push('{');
        rest = &rest[start + 1..];
    }
    output.push_str(rest);
    output
}

fn value(name: &str, context: &AlertMessageContext) -> Option<String> {
    match name {
        "ticker" => context.ticker.clone(),
        "exchange" => context.exchange.clone(),
        "interval" => context.interval.clone(),
        "open" => context.open.map(number),
        "high" => context.high.map(number),
        "low" => context.low.map(number),
        "close" => context.close.map(number),
        "volume" => context.volume.map(number),
        "time" => context.time_ms.map(timestamp),
        "timenow" => Some(timestamp(context.now_ms)),
        _ => None,
    }
}

/// Plain decimal digits, no grouping, no exponent: `100000.5`, `0.00000278`. Up to twelve
/// fraction digits, trailing zeros trimmed; a non-finite value renders as nothing.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    // The shortest digits that read back as `value` (`121480.37`, not the
    // `121480.369999999995` a fixed twelve places would expose), unless that needs more
    // than twelve fraction digits.
    let shortest = format!("{value}");
    let fraction_digits = shortest
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let mut text = if fraction_digits <= 12 {
        shortest
    } else {
        format!("{value:.12}")
    };
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

/// ISO 8601 in UTC, like TradingView's `{{time}}`: `2024-05-01T13:45:00Z`.
pub fn timestamp(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Maps DegenView's interval names (`1m`, `1h`, `1d`, `1w`, `1M`) to TradingView's (`1`,
/// `60`, `D`, `W`, `M`). Unknown names pass through unchanged.
pub fn trading_view_interval(interval: &str) -> String {
    match interval {
        "1d" => return "D".to_owned(),
        "1w" => return "W".to_owned(),
        "1M" => return "M".to_owned(),
        "3M" => return "3M".to_owned(),
        "1Y" => return "12M".to_owned(),
        _ => {}
    }
    let (digits, unit) = interval.split_at(interval.len().saturating_sub(1));
    let Ok(count) = digits.parse::<u32>() else {
        return interval.to_owned();
    };
    match unit {
        "m" => count.to_string(),
        "h" => (count * 60).to_string(),
        "d" => format!("{count}D"),
        _ => interval.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-05-01 13:45:30 UTC.
    const BAR: i64 = 1_714_571_130_000;

    fn context() -> AlertMessageContext {
        AlertMessageContext {
            ticker: Some("BTCUSDT".into()),
            exchange: Some("Binance".into()),
            interval: Some("60".into()),
            open: Some(100_000.5),
            high: Some(101_000.0),
            low: Some(0.000_002_78),
            close: Some(100_500.25),
            volume: Some(12.0),
            time_ms: Some(BAR),
            now_ms: BAR + 60_000,
        }
    }

    #[test]
    fn every_documented_placeholder_resolves() {
        let all = SUPPORTED_PLACEHOLDERS
            .iter()
            .map(|name| format!("{name}={{{{{name}}}}}"))
            .collect::<Vec<_>>()
            .join(";");
        assert_eq!(
            render(&all, &context()),
            "ticker=BTCUSDT;exchange=Binance;open=100000.5;high=101000;low=0.00000278;\
             close=100500.25;volume=12;time=2024-05-01T13:45:30Z;timenow=2024-05-01T13:46:30Z;interval=60"
        );
        assert_eq!(
            render(DEFAULT_PRICE_ALERT_TEMPLATE, &context()),
            "BTCUSDT alert: 100500.25"
        );
    }

    #[test]
    fn unknown_unclosed_and_spaced_tokens_stay_as_written() {
        let context = context();
        for text in [
            "{{plot_0}}",
            "{{strategy.position_size}}",
            "{{ close }}",
            "{{close",
            "{{}}",
            "}}{{",
            "{{CLOSE}}",
            "{close}",
            "no tokens",
            "",
        ] {
            assert_eq!(render(text, &context), text, "{text:?}");
        }
    }

    #[test]
    fn a_value_the_trigger_did_not_carry_is_left_as_the_literal_placeholder() {
        let sparse = AlertMessageContext {
            ticker: Some("X".into()),
            now_ms: BAR,
            ..Default::default()
        };
        assert_eq!(
            render(
                "{{ticker}} {{open}} {{time}} {{interval}} {{timenow}}",
                &sparse
            ),
            "X {{open}} {{time}} {{interval}} 2024-05-01T13:45:30Z"
        );
    }

    #[test]
    fn substituted_values_are_never_rescanned() {
        let tricky = AlertMessageContext {
            ticker: Some("{{close}}".into()),
            close: Some(1.0),
            now_ms: 0,
            ..Default::default()
        };
        assert_eq!(render("{{ticker}} {{close}}", &tricky), "{{close}} 1");
        // A brace right before a token does not hide it.
        assert_eq!(render("{{{{close}}", &tricky), "{{1");
        assert_eq!(render("{{{close}}}", &tricky), "{1}");
    }

    #[test]
    fn text_around_tokens_and_unicode_survive() {
        let context = context();
        assert_eq!(
            render(
                "🚀 {{ticker}} → {{close}}€\n{\"k\":\"{{ticker}}\"}",
                &context
            ),
            "🚀 BTCUSDT → 100500.25€\n{\"k\":\"BTCUSDT\"}"
        );
    }

    #[test]
    fn numbers_are_plain_decimals() {
        assert_eq!(number(100_000.5), "100000.5");
        assert_eq!(number(0.000_002_78), "0.00000278");
        assert_eq!(number(12.0), "12");
        assert_eq!(number(-1.5), "-1.5");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(1e21), "1000000000000000000000");
        assert_eq!(number(1.0 / 3.0), "0.333333333333");
        assert_eq!(
            number(121_480.37),
            "121480.37",
            "no float noise from fixed places"
        );
        assert_eq!(number(83_356.01), "83356.01");
        assert_eq!(number(f64::NAN), "");
        assert_eq!(number(f64::INFINITY), "");
        let infinite = AlertMessageContext {
            close: Some(f64::INFINITY),
            now_ms: 0,
            ..Default::default()
        };
        assert_eq!(render("[{{close}}]", &infinite), "[]");
    }

    #[test]
    fn timestamps_are_iso_8601_utc() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(BAR), "2024-05-01T13:45:30Z");
        assert_eq!(timestamp(-1_000), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn intervals_map_to_trading_views_names() {
        for (ours, theirs) in [
            ("1m", "1"),
            ("15m", "15"),
            ("1h", "60"),
            ("4h", "240"),
            ("1d", "D"),
            ("3d", "3D"),
            ("1w", "W"),
            ("1M", "M"),
            ("3M", "3M"),
            ("1Y", "12M"),
            ("weird", "weird"),
            ("", ""),
            ("m", "m"),
        ] {
            assert_eq!(trading_view_interval(ours), theirs, "{ours}");
        }
    }
}
