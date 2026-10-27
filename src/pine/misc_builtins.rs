//! Small builtins that carry no state: `max_bars_back`, the `ticker.*` builders, drawing-type
//! casts (`box(na)`), the `log.*` messages and the `request.*` data feeds this app does not have.
//! The feeds read as `na`, and `ignored_features` warns about every call, so none is silent.

use super::ast::Argument;
use super::logs::LogLevel;
use super::number_format;
use super::runtime_types::RuntimeContext;
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::Session;
use super::strings;
use super::token::{Diagnostic, SourceRange};

/// `request.*` functions that return a series from a feed the app has no access to.
pub const UNAVAILABLE_FEEDS: [&str; 8] = [
    "request.seed",
    "request.dividends",
    "request.earnings",
    "request.economic",
    "request.financial",
    "request.quandl",
    "request.splits",
    "request.currency_rate",
];

/// Functions the engine does not implement: they return `na`, and `ignored_features` warns.
pub const UNIMPLEMENTED_FUNCTIONS: [&str; 0] = [];

/// `strategy.convert_to_*`: the amount comes back unchanged, which is right while the account
/// and the symbol share a currency (no exchange rates are available here).
pub const CURRENCY_CONVERSIONS: [&str; 2] =
    ["strategy.convert_to_account", "strategy.convert_to_symbol"];

fn drawing_kind(name: &str) -> Option<RefKind> {
    Some(match name {
        "line" => RefKind::Line,
        "label" => RefKind::Label,
        "box" => RefKind::Box,
        "table" => RefKind::Table,
        "linefill" => RefKind::Linefill,
        "polyline" => RefKind::Polyline,
        _ => return None,
    })
}

/// The text of a `log.*()` call: the message as given, or the format string filled with the
/// arguments after it, as `str.format()` does.
fn log_message(values: &[RuntimeValue], mintick: f64) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    let template = match first.text_value() {
        Some(text) => text.to_owned(),
        None => number_format::format(first, None, mintick),
    };
    if values.len() == 1 {
        template
    } else {
        strings::format_template(&template, &values[1..], mintick)
    }
}

impl Session {
    pub(crate) fn misc_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        if let Some(kind) = drawing_kind(name) {
            // `box(na)` types an `na`; a handle of the same kind passes through.
            let value = self.arg_value(arguments, 0, None, context)?;
            return Ok(Some(match value {
                RuntimeValue::Ref(found, _) if found == kind => value,
                _ => RuntimeValue::Na,
            }));
        }
        Ok(Some(match name {
            "max_bars_back" => {
                // The whole history is always kept, so a larger buffer changes nothing.
                RuntimeValue::Void
            }
            "log.info" | "log.warning" | "log.error" => {
                let values = self.eval_all(arguments, context)?;
                // The arguments are evaluated either way, as they may have effects; only a run
                // somebody reads keeps the message.
                if let Some(level) =
                    LogLevel::from_function(name).filter(|_| self.logs.is_enabled())
                {
                    let message = log_message(&values, self.mintick);
                    self.logs
                        .record(level, message, context.scope.clone(), range);
                }
                RuntimeValue::Void
            }
            "ticker.new" => {
                let text = |value: &RuntimeValue| value.text_value().unwrap_or_default().to_owned();
                let prefix = text(&self.arg_value(arguments, 0, Some("prefix"), context)?);
                let ticker = text(&self.arg_value(arguments, 1, Some("ticker"), context)?);
                RuntimeValue::String(if prefix.is_empty() {
                    ticker
                } else {
                    format!("{prefix}:{ticker}")
                })
            }
            "ticker.standard" | "ticker.modify" | "ticker.inherit" => {
                // The modifiers (session, adjustment) are not modelled: the symbol stays as given,
                // for `inherit` the one that takes the modifiers.
                let index = usize::from(name == "ticker.inherit");
                self.eval_all(arguments, context)?
                    .get(index)
                    .cloned()
                    .unwrap_or(RuntimeValue::Na)
            }
            "ticker.heikinashi" | "ticker.renko" | "ticker.kagi" | "ticker.linebreak"
            | "ticker.pointfigure" => self.chart_type_ticker(name, arguments, context)?,
            "request.currency_rate" => {
                let values = self.eval_all(arguments, context)?;
                match (values.first(), values.get(1)) {
                    (Some(from), Some(to)) if from.text_value().is_some() && from == to => {
                        RuntimeValue::Float(1.0)
                    }
                    _ => RuntimeValue::Na,
                }
            }
            _ if UNAVAILABLE_FEEDS.contains(&name) || UNIMPLEMENTED_FUNCTIONS.contains(&name) => {
                RuntimeValue::Na
            }
            _ if CURRENCY_CONVERSIONS.contains(&name) => self
                .eval_all(arguments, context)?
                .first()
                .cloned()
                .unwrap_or(RuntimeValue::Na),
            _ => return Ok(None),
        }))
    }
}

impl Session {
    /// `ticker.heikinashi()`, `.renko()`, `.kagi()`, `.linebreak()` and `.pointfigure()`: the
    /// symbol with the construction attached, for `request.security()` to serve through the same
    /// provider as any other symbol. An argument the builder cannot use gives `na`, the way
    /// TradingView's own ticker builders refuse one.
    fn chart_type_ticker(
        &mut self,
        name: &str,
        arguments: &[Argument],
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        use crate::model::chart_type::{ConstructionSource, SizeRule};
        use crate::model::synthetic::ticker::{self, TickerModifier};

        let symbol = match self.arg_value(arguments, 0, Some("symbol"), context)? {
            RuntimeValue::String(symbol) => symbol,
            _ => return Ok(RuntimeValue::Na),
        };
        let text = |value: &RuntimeValue| value.text_value().map(str::to_owned);
        let source_of = |value: &RuntimeValue, fallback: ConstructionSource| match value {
            RuntimeValue::Na => Some(fallback),
            other => match text(other)?.to_ascii_lowercase().as_str() {
                "close" => Some(ConstructionSource::Close),
                "ohlc" | "hl" | "highlow" => Some(ConstructionSource::HighLow),
                _ => None,
            },
        };
        let modifier = match name {
            "ticker.heikinashi" => Some(TickerModifier::HeikinAshi),
            "ticker.renko" => {
                let style = self.arg_value(arguments, 1, Some("style"), context)?;
                let param = self.arg_value(arguments, 2, Some("param"), context)?;
                let wicks = self.arg_value(arguments, 3, Some("request_wicks"), context)?;
                let source = self.arg_value(arguments, 4, Some("source"), context)?;
                text(&style)
                    .zip(param.number())
                    .and_then(|(style, param)| ticker::size_rule(&style, param))
                    .zip(source_of(&source, ConstructionSource::Close))
                    .map(|(size, source)| TickerModifier::Renko {
                        size,
                        wicks: matches!(wicks, RuntimeValue::Bool(true)),
                        source,
                    })
            }
            "ticker.kagi" => self
                .arg_value(arguments, 1, Some("reversal"), context)?
                .number()
                .and_then(|reversal| ticker::size_rule("Traditional", reversal))
                .map(|size| TickerModifier::Kagi { size }),
            "ticker.linebreak" => self
                .arg_value(arguments, 1, Some("number_of_lines"), context)?
                .number()
                .filter(|lines| *lines >= 1.0)
                .map(|lines| TickerModifier::LineBreak {
                    lines: lines.round().min(20.0) as u8,
                }),
            _ => {
                let source = self.arg_value(arguments, 1, Some("source"), context)?;
                let style = self.arg_value(arguments, 2, Some("style"), context)?;
                let param = self.arg_value(arguments, 3, Some("param"), context)?;
                let reversal = self.arg_value(arguments, 4, Some("reversal"), context)?;
                let size: Option<SizeRule> = text(&style)
                    .zip(param.number())
                    .and_then(|(style, param)| ticker::size_rule(&style, param));
                size.zip(source_of(&source, ConstructionSource::HighLow))
                    .zip(reversal.number().filter(|boxes| *boxes >= 1.0))
                    .map(|((size, source), boxes)| TickerModifier::PointFigure {
                        size,
                        reversal: boxes.round().min(20.0) as u8,
                        source,
                    })
            }
        };
        Ok(modifier
            .map(|modifier| RuntimeValue::String(ticker::encode(&symbol, &modifier)))
            .unwrap_or(RuntimeValue::Na))
    }
}
