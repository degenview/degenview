//! `scalar_call`: the builtins that take and return plain values, matched by name: `na`, `nz`,
//! the type casts, `color.*`, `timeframe.*`, `time`, `time_close`, `timestamp` and `str.tostring`.

use std::collections::HashMap;

use super::Session;
use crate::pine::builtins::number_format;
use crate::pine::runtime::types::RuntimeContext;
use crate::pine::runtime::value::RuntimeValue;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

impl Session {
    pub(crate) fn scalar_call(
        &mut self,
        name: &str,
        arguments: &[crate::pine::syntax::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        match name {
            "na" => Ok(Some(RuntimeValue::Bool(
                self.arg_value(arguments, 0, None, context)? == RuntimeValue::Na,
            ))),
            "nz" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                if v != RuntimeValue::Na {
                    return Ok(Some(v));
                }
                let replacement = self.arg_value(arguments, 1, None, context)?;
                Ok(Some(if replacement == RuntimeValue::Na {
                    RuntimeValue::Float(0.0)
                } else {
                    replacement
                }))
            }
            // `int(x)` truncates toward zero; only `na` and non-finite values have no integer.
            "int" => Ok(Some(
                match self.arg_value(arguments, 0, None, context)?.number() {
                    Some(number) if number.is_finite() => RuntimeValue::Int(number.trunc() as i64),
                    _ => RuntimeValue::Na,
                },
            )),
            "float" => Ok(Some(
                self.arg_value(arguments, 0, None, context)?
                    .number()
                    .map(RuntimeValue::Float)
                    .unwrap_or(RuntimeValue::Na),
            )),
            "bool" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                Ok(Some(match &v {
                    RuntimeValue::Bool(_) => v,
                    _ => RuntimeValue::Bool(v.number().is_some_and(|n| n != 0.0)),
                }))
            }
            "string" | "color" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                let keeps = matches!(
                    (&v, name),
                    (RuntimeValue::String(_), "string") | (RuntimeValue::Color(_), "color")
                );
                Ok(Some(if keeps { v } else { RuntimeValue::Na }))
            }
            "color.new" => {
                let c = self.arg_value(arguments, 0, None, context)?;
                let transparency = self.arg_value(arguments, 1, None, context)?.number();
                Ok(Some(match (&c, transparency) {
                    (RuntimeValue::Color(c), Some(t)) => {
                        RuntimeValue::Color(crate::pine::builtins::with_transparency(*c, t))
                    }
                    _ => RuntimeValue::Na,
                }))
            }
            "str.format_time" => {
                let bound = self.bind_named(arguments, &["time", "format", "timezone"], context)?;
                let text = |key: &str| match bound.get(key) {
                    Some(RuntimeValue::String(text)) => Some(text.clone()),
                    _ => None,
                };
                Ok(Some(
                    match bound.get("time").and_then(RuntimeValue::int_value) {
                        Some(ms) => RuntimeValue::String(crate::pine::time::formatted(
                            ms,
                            text("format").as_deref(),
                            text("timezone").as_deref(),
                        )),
                        None => RuntimeValue::Na,
                    },
                ))
            }
            "color.r" | "color.g" | "color.b" | "color.t" => {
                let color = self.arg_value(arguments, 0, Some("color"), context)?;
                Ok(Some(match color {
                    RuntimeValue::Color(c) => {
                        RuntimeValue::Float(crate::pine::builtins::color_component(
                            c,
                            name.chars().last().unwrap_or('r'),
                        ))
                    }
                    _ => RuntimeValue::Na,
                }))
            }
            "color.from_gradient" => {
                let value = self.arg_value(arguments, 0, Some("value"), context)?;
                let bottom = self.arg_value(arguments, 1, Some("bottom_value"), context)?;
                let top = self.arg_value(arguments, 2, Some("top_value"), context)?;
                let from = self.arg_value(arguments, 3, Some("bottom_color"), context)?;
                let to = self.arg_value(arguments, 4, Some("top_color"), context)?;
                Ok(Some(
                    match (value.number(), bottom.number(), top.number(), from, to) {
                        (
                            Some(v),
                            Some(b),
                            Some(t),
                            RuntimeValue::Color(f),
                            RuntimeValue::Color(c),
                        ) => RuntimeValue::Color(crate::pine::builtins::gradient(v, b, t, f, c)),
                        _ => RuntimeValue::Na,
                    },
                ))
            }
            "timeframe.in_seconds" => {
                let timeframe = self.arg_value(arguments, 0, Some("timeframe"), context)?;
                let seconds = match timeframe {
                    RuntimeValue::String(text) => crate::pine::time::seconds_of_timeframe(&text),
                    // No argument: the chart's own bar length.
                    RuntimeValue::Na if self.bar_seconds > 0.0 => Some(self.bar_seconds),
                    _ => None,
                };
                Ok(Some(match seconds {
                    Some(seconds) => RuntimeValue::Int(seconds.round() as i64),
                    None => RuntimeValue::Na,
                }))
            }
            "timeframe.change" => {
                let timeframe = self.arg_value(arguments, 0, Some("timeframe"), context)?;
                let seconds = match timeframe {
                    RuntimeValue::String(text) => crate::pine::time::seconds_of_timeframe(&text),
                    _ => None,
                };
                Ok(Some(match seconds {
                    Some(seconds) => RuntimeValue::Bool(crate::pine::time::timeframe_changed(
                        self.last_committed_open_time,
                        context.bar.open_time,
                        seconds,
                    )),
                    None => RuntimeValue::Na,
                }))
            }
            "timeframe.from_seconds" => {
                let seconds = self.arg_value(arguments, 0, Some("seconds"), context)?;
                Ok(Some(
                    match seconds
                        .int_value()
                        .and_then(crate::pine::time::timeframe_from_seconds)
                    {
                        Some(text) => RuntimeValue::String(text),
                        None => RuntimeValue::Na,
                    },
                ))
            }
            "color.rgb" => {
                let r = self
                    .arg_value(arguments, 0, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let g = self
                    .arg_value(arguments, 1, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let b = self
                    .arg_value(arguments, 2, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let transparency = self
                    .arg_value(arguments, 3, None, context)?
                    .number()
                    .unwrap_or(0.0);
                Ok(Some(RuntimeValue::Color(crate::pine::builtins::rgb(
                    r,
                    g,
                    b,
                    transparency,
                ))))
            }
            // `hour(time, timezone)` and the other calendar parts: of `time` (the bar's own when
            // there are no arguments), read in `timezone` (UTC when omitted).
            _ if crate::pine::time::PART_NAMES.contains(&name) => {
                let bound = self.bind_named(arguments, &["time", "timezone"], context)?;
                let stamp = if arguments.is_empty() {
                    Some(context.bar.open_time)
                } else {
                    bound.get("time").and_then(RuntimeValue::int_value)
                };
                let zone = match bound.get("timezone") {
                    Some(RuntimeValue::String(zone)) => Some(zone.as_str()),
                    _ => None,
                };
                let offset = crate::pine::time::zone_offset_secs(zone, stamp.unwrap_or_default());
                Ok(Some(match stamp {
                    Some(stamp) => crate::pine::time::part(name, stamp, offset),
                    None => RuntimeValue::Na,
                }))
            }
            // `time(timeframe, session, timezone)`: the open time of the `timeframe` bar that
            // contains the current bar. With a `session` it is `na` unless that bar opens inside
            // the session, read in `timezone` (UTC when omitted). Without a usable timeframe it is
            // the bar's own `time`.
            "time" => {
                let bound =
                    self.bind_named(arguments, &["timeframe", "session", "timezone"], context)?;
                let mut start = context.bar.open_time;
                if let Some(RuntimeValue::String(text)) = bound.get("timeframe")
                    && !text.is_empty()
                {
                    let Some(seconds) = crate::pine::time::seconds_of_timeframe(text) else {
                        return Err(Diagnostic::error(
                            "PINE4021",
                            DiagnosticCategory::Runtime,
                            "time() needs a timeframe such as \"60\" or \"1D\".",
                            range,
                        ));
                    };
                    // The chart's own timeframe keeps the bar's real open.
                    if seconds != self.bar_seconds {
                        start = crate::pine::runtime::security_call::bucket_start(start, seconds);
                    }
                }
                if let Some(RuntimeValue::String(text)) = bound.get("session")
                    && !text.is_empty()
                {
                    let Some(session) =
                        crate::pine::time::trading_session::TradingSession::parse(text)
                    else {
                        return Ok(Some(RuntimeValue::Na));
                    };
                    let zone = match bound.get("timezone") {
                        Some(RuntimeValue::String(zone)) => Some(zone.as_str()),
                        _ => None,
                    };
                    let offset = crate::pine::time::zone_offset_secs(zone, start);
                    if !session.contains(&crate::pine::time::components(start, offset)) {
                        return Ok(Some(RuntimeValue::Na));
                    }
                }
                Ok(Some(RuntimeValue::Int(start)))
            }
            // `time_close(timeframe)`: the close of the `timeframe` bar that contains the
            // current bar.
            "time_close" => {
                let timeframe = self.arg_value(arguments, 0, Some("timeframe"), context)?;
                let RuntimeValue::String(text) = timeframe else {
                    return Ok(Some(
                        self.market("time_close", context)
                            .unwrap_or(RuntimeValue::Na),
                    ));
                };
                if text.is_empty() {
                    return Ok(Some(
                        self.market("time_close", context)
                            .unwrap_or(RuntimeValue::Na),
                    ));
                }
                let Some(seconds) = crate::pine::time::seconds_of_timeframe(&text) else {
                    return Err(Diagnostic::error(
                        "PINE4021",
                        DiagnosticCategory::Runtime,
                        "time_close() needs a timeframe such as \"60\" or \"1D\".",
                        range,
                    ));
                };
                let start = crate::pine::runtime::security_call::bucket_start(
                    context.bar.open_time,
                    seconds,
                );
                Ok(Some(RuntimeValue::Int(
                    crate::pine::runtime::security_call::bucket_end(start, seconds),
                )))
            }
            "timestamp" => {
                let mut positional = Vec::new();
                let mut named = HashMap::new();
                for argument in arguments {
                    let v = self.eval(&argument.value, context)?;
                    if let Some(label) = &argument.name {
                        named.insert(label.clone(), v);
                    } else {
                        positional.push(v);
                    }
                }
                Ok(Some(
                    crate::pine::time::timestamp::evaluate(&positional, &named)
                        .map(RuntimeValue::Int)
                        .unwrap_or(RuntimeValue::Na),
                ))
            }
            "str.tostring" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                let pattern = self
                    .arg_value(arguments, 1, Some("format"), context)?
                    .text_value()
                    .map(str::to_owned);
                Ok(Some(RuntimeValue::String(number_format::format(
                    &v,
                    pattern.as_deref(),
                    self.mintick,
                ))))
            }
            _ => Ok(None),
        }
    }
}
