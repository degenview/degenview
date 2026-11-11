//! The tickers `ticker.heikinashi()`, `ticker.renko()`, `ticker.kagi()`, `ticker.linebreak()` and
//! `ticker.pointfigure()` return, and how they are served.
//!
//! A script hands such a ticker to `request.security()`. The ticker is the plain symbol with the
//! construction appended (`BINANCE:BTCUSDT@renko(atr,14,false,close)`), so it flows through the
//! engine as any other symbol string; the security data provider splits it, fetches the real
//! candles of the base symbol and builds the bars with the same builders the chart uses. There is
//! no second synthetic feed: [`apply`] is [`build_with`] over the same [`Construction`] a chart
//! of that type uses.

use super::{Construction, build_with};
use crate::model::Kline;
use crate::model::chart::kind::{ConstructionSource, SizeMethod, SizeRule};

#[derive(Clone, Debug, PartialEq)]
pub enum TickerModifier {
    HeikinAshi,
    Renko {
        size: SizeRule,
        wicks: bool,
        source: ConstructionSource,
    },
    Kagi {
        size: SizeRule,
    },
    LineBreak {
        lines: u8,
    },
    PointFigure {
        size: SizeRule,
        reversal: u8,
        source: ConstructionSource,
    },
}

const SEPARATOR: char = '@';

fn method_token(rule: &SizeRule) -> String {
    match rule.method {
        SizeMethod::Traditional => format!("traditional,{}", rule.amount),
        SizeMethod::Atr => format!("atr,{}", rule.atr_length),
        SizeMethod::Percent => format!("percent,{}", rule.percent),
    }
}

fn source_token(source: ConstructionSource) -> &'static str {
    match source {
        ConstructionSource::Close => "close",
        ConstructionSource::HighLow => "hl",
    }
}

/// `symbol` with `modifier` appended: the value of the matching `ticker.*()` call.
pub fn encode(symbol: &str, modifier: &TickerModifier) -> String {
    let base = split(symbol).0;
    let body = match modifier {
        TickerModifier::HeikinAshi => "heikinashi".to_owned(),
        TickerModifier::Renko {
            size,
            wicks,
            source,
        } => format!(
            "renko({},{},{})",
            method_token(size),
            wicks,
            source_token(*source)
        ),
        TickerModifier::Kagi { size } => format!("kagi({})", method_token(size)),
        TickerModifier::LineBreak { lines } => format!("linebreak({lines})"),
        TickerModifier::PointFigure {
            size,
            reversal,
            source,
        } => format!(
            "pointfigure({},{},{})",
            method_token(size),
            reversal,
            source_token(*source)
        ),
    };
    format!("{base}{SEPARATOR}{body}")
}

/// The plain symbol and the construction a ticker carries, if it carries one.
pub fn split(ticker: &str) -> (&str, Option<TickerModifier>) {
    match ticker.rsplit_once(SEPARATOR) {
        Some((base, body)) => match parse(body) {
            Some(modifier) => (base, Some(modifier)),
            None => (ticker, None),
        },
        None => (ticker, None),
    }
}

fn parse_rule(parts: &mut std::str::Split<'_, char>) -> Option<SizeRule> {
    let method = parts.next()?;
    let value = parts.next()?;
    let mut rule = SizeRule::default();
    match method {
        "traditional" => {
            rule.method = SizeMethod::Traditional;
            rule.amount = value.parse().ok()?;
        }
        "atr" => {
            rule.method = SizeMethod::Atr;
            rule.atr_length = value.parse().ok()?;
        }
        "percent" => {
            rule.method = SizeMethod::Percent;
            rule.percent = value.parse().ok()?;
        }
        _ => return None,
    }
    Some(rule.sanitized())
}

fn parse_source(token: &str) -> Option<ConstructionSource> {
    match token {
        "close" => Some(ConstructionSource::Close),
        "hl" => Some(ConstructionSource::HighLow),
        _ => None,
    }
}

fn parse(body: &str) -> Option<TickerModifier> {
    if body == "heikinashi" {
        return Some(TickerModifier::HeikinAshi);
    }
    let (name, rest) = body.split_once('(')?;
    let inner = rest.strip_suffix(')')?;
    let mut parts = inner.split(',');
    let modifier = match name {
        "renko" => {
            let size = parse_rule(&mut parts)?;
            let wicks = parts.next()?.parse().ok()?;
            let source = parse_source(parts.next()?)?;
            TickerModifier::Renko {
                size,
                wicks,
                source,
            }
        }
        "kagi" => TickerModifier::Kagi {
            size: parse_rule(&mut parts)?,
        },
        "linebreak" => TickerModifier::LineBreak {
            lines: parts.next()?.parse::<u8>().ok()?.clamp(1, 20),
        },
        "pointfigure" => {
            let size = parse_rule(&mut parts)?;
            let reversal = parts.next()?.parse::<u8>().ok()?.clamp(1, 20);
            let source = parse_source(parts.next()?)?;
            TickerModifier::PointFigure {
                size,
                reversal,
                source,
            }
        }
        _ => return None,
    };
    parts.next().is_none().then_some(modifier)
}

/// The bars `modifier` constructs from `candles` (the real candles of the base symbol), as the
/// candles `request.security()` reads.
pub fn apply(modifier: &TickerModifier, candles: &[Kline], now_ms: i64) -> Vec<Kline> {
    let construction = match modifier {
        TickerModifier::HeikinAshi => Construction::HeikinAshi,
        TickerModifier::Renko {
            size,
            wicks,
            source,
        } => Construction::Renko {
            size: *size,
            source: *source,
            wicks: *wicks,
        },
        TickerModifier::Kagi { size } => Construction::Kagi {
            size: *size,
            source: ConstructionSource::Close,
        },
        TickerModifier::LineBreak { lines } => Construction::LineBreak { lines: *lines },
        TickerModifier::PointFigure {
            size,
            reversal,
            source,
        } => Construction::PointFigure {
            size: *size,
            reversal: *reversal,
            source: *source,
        },
    };
    build_with(construction, candles, now_ms).klines()
}

/// The `style`/`param` pair of `ticker.renko()` and `ticker.pointfigure()` as a size rule.
/// `Traditional` reads `param` as the box size, `ATR` as the length, `PercentageLTP` as a
/// percentage of the last price; anything else is rejected.
pub fn size_rule(style: &str, param: f64) -> Option<SizeRule> {
    if !param.is_finite() || param <= 0.0 {
        return None;
    }
    let rule = SizeRule::default();
    let rule = match style.to_ascii_lowercase().as_str() {
        "traditional" => SizeRule {
            method: SizeMethod::Traditional,
            amount: param,
            ..rule
        },
        "atr" => SizeRule {
            method: SizeMethod::Atr,
            atr_length: param.round().max(1.0) as u32,
            ..rule
        },
        "percentageltp" => SizeRule {
            method: SizeMethod::Percent,
            percent: param,
            ..rule
        },
        _ => return None,
    };
    Some(rule.sanitized())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn traditional(amount: f64) -> SizeRule {
        SizeRule {
            method: SizeMethod::Traditional,
            amount,
            ..SizeRule::default()
        }
    }

    #[test]
    fn every_modifier_round_trips_through_the_ticker_string() {
        let modifiers = [
            TickerModifier::HeikinAshi,
            TickerModifier::Renko {
                size: traditional(0.25),
                wicks: true,
                source: ConstructionSource::HighLow,
            },
            TickerModifier::Renko {
                size: SizeRule::default(),
                wicks: false,
                source: ConstructionSource::Close,
            },
            TickerModifier::Kagi {
                size: traditional(12.5),
            },
            TickerModifier::LineBreak { lines: 4 },
            TickerModifier::PointFigure {
                size: SizeRule {
                    method: SizeMethod::Percent,
                    percent: 0.75,
                    ..SizeRule::default()
                },
                reversal: 3,
                source: ConstructionSource::Close,
            },
        ];
        for modifier in modifiers {
            let ticker = encode("BINANCE:BTCUSDT", &modifier);
            let (base, parsed) = split(&ticker);
            assert_eq!(base, "BINANCE:BTCUSDT", "{ticker}");
            assert_eq!(parsed.as_ref(), Some(&modifier), "{ticker}");
        }
    }

    #[test]
    fn a_plain_or_unknown_ticker_carries_nothing() {
        assert_eq!(split("BINANCE:BTCUSDT"), ("BINANCE:BTCUSDT", None));
        assert_eq!(split("A@nonsense(1)"), ("A@nonsense(1)", None));
        assert_eq!(split("A@renko(atr,x,false,close)").1, None);
        assert_eq!(split("A@renko(atr,14,false,close,extra)").1, None);
    }

    #[test]
    fn modifying_a_modified_ticker_replaces_the_modifier() {
        let once = encode("BINANCE:BTCUSDT", &TickerModifier::HeikinAshi);
        let twice = encode(&once, &TickerModifier::LineBreak { lines: 3 });
        assert_eq!(split(&twice).0, "BINANCE:BTCUSDT");
        assert_eq!(
            split(&twice).1,
            Some(TickerModifier::LineBreak { lines: 3 })
        );
    }

    #[test]
    fn applying_a_modifier_builds_the_chart_s_bars() {
        let source = closes(&[10.0, 12.5, 11.0, 9.0]);
        let now = after(&source);
        let ha = apply(&TickerModifier::HeikinAshi, &source, now);
        assert_eq!(ha.len(), source.len());
        assert_eq!(ha[1].close, 12.5, "(12.5 * 4) / 4");
        let renko = apply(
            &TickerModifier::Renko {
                size: traditional(1.0),
                wicks: false,
                source: ConstructionSource::Close,
            },
            &source,
            now,
        );
        assert_eq!(
            renko.iter().map(|bar| bar.close).collect::<Vec<_>>(),
            vec![11.0, 12.0, 10.0, 9.0]
        );
        assert!(
            renko
                .windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time)
        );
    }

    #[test]
    fn the_style_and_param_pair_follows_pines_names() {
        assert_eq!(size_rule("Traditional", 5.0), Some(traditional(5.0)));
        assert_eq!(
            size_rule("ATR", 14.0).map(|rule| (rule.method, rule.atr_length)),
            Some((SizeMethod::Atr, 14))
        );
        assert_eq!(
            size_rule("PercentageLTP", 2.0).map(|rule| rule.percent),
            Some(2.0)
        );
        assert_eq!(size_rule("Bogus", 1.0), None);
        assert_eq!(size_rule("ATR", 0.0), None);
    }
}
