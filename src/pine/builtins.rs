//! Builtin constants and color math shared by compile-time folding and the runtime, ported from
//! `PineBuiltins.swift`.

use super::runtime_value::RuntimeValue;
use crate::pine::ValueType;

pub fn color(name: &str) -> Option<u32> {
    match name {
        "color.aqua" => Some(0x00bc_d4ff),
        "color.black" => Some(0x0000_00ff),
        "color.blue" => Some(0x2196_f3ff),
        "color.fuchsia" => Some(0xe040_fbff),
        "color.gray" => Some(0x787b_86ff),
        "color.green" => Some(0x4caf_50ff),
        "color.lime" => Some(0x00e6_76ff),
        "color.maroon" => Some(0x880e_4fff),
        "color.navy" => Some(0x0d47_a1ff),
        "color.olive" => Some(0x8277_17ff),
        "color.orange" => Some(0xff98_00ff),
        "color.purple" => Some(0x9c27_b0ff),
        "color.red" => Some(0xf236_45ff),
        "color.silver" => Some(0xb2b5_beff),
        "color.teal" => Some(0x0089_7bff),
        "color.white" => Some(0xffff_ffff),
        "color.yellow" => Some(0xffeb_3bff),
        _ => None,
    }
}

/// A named constant that is not a color. Enumeration-like values and `display.*`/`dayofweek.*`
/// are integers because scripts do arithmetic on them; the rest evaluate to their own name.
pub fn constant(name: &str) -> Option<RuntimeValue> {
    match name {
        "display.none" => Some(RuntimeValue::Int(0)),
        "display.pane" => Some(RuntimeValue::Int(1)),
        "display.data_window" => Some(RuntimeValue::Int(2)),
        "display.price_scale" => Some(RuntimeValue::Int(4)),
        "display.status_line" => Some(RuntimeValue::Int(8)),
        "display.all" => Some(RuntimeValue::Int(15)),
        "dayofweek.sunday" => Some(RuntimeValue::Int(1)),
        "dayofweek.monday" => Some(RuntimeValue::Int(2)),
        "dayofweek.tuesday" => Some(RuntimeValue::Int(3)),
        "dayofweek.wednesday" => Some(RuntimeValue::Int(4)),
        "dayofweek.thursday" => Some(RuntimeValue::Int(5)),
        "dayofweek.friday" => Some(RuntimeValue::Int(6)),
        "dayofweek.saturday" => Some(RuntimeValue::Int(7)),
        "math.pi" => Some(RuntimeValue::Float(std::f64::consts::PI)),
        "math.e" => Some(RuntimeValue::Float(std::f64::consts::E)),
        "math.phi" => Some(RuntimeValue::Float((1.0 + 5.0_f64.sqrt()) / 2.0)),
        "math.rphi" => Some(RuntimeValue::Float((5.0_f64.sqrt() - 1.0) / 2.0)),
        "strategy.long"
        | "strategy.short"
        | "strategy.cash"
        | "strategy.fixed"
        | "strategy.percent_of_equity"
        | "strategy.commission.percent"
        | "strategy.commission.cash_per_order"
        | "strategy.commission.cash_per_contract"
        | "strategy.oca.none"
        | "strategy.oca.cancel"
        | "strategy.oca.reduce"
        | "format.inherit"
        | "format.price"
        | "format.volume"
        | "format.percent"
        | "format.mintick"
        | "order.ascending"
        | "order.descending"
        | "strategy.direction.long"
        | "strategy.direction.short"
        | "strategy.direction.all"
        | "alert.freq_all"
        | "alert.freq_once_per_bar"
        | "alert.freq_once_per_bar_close" => Some(RuntimeValue::String(name.to_owned())),
        _ => None,
    }
}

/// The value type an `input.*` function declares; anything unlisted is a string.
pub fn input_type(function: &str) -> ValueType {
    match function {
        "input.int" => ValueType::Int,
        "input.float" | "input.price" => ValueType::Float,
        "input.bool" => ValueType::Bool,
        "input.color" => ValueType::Color,
        "input.time" => ValueType::Time,
        _ => ValueType::String,
    }
}

/// The value type of an input: what its function declares, or for the generic `input()` what its
/// default value is.
pub fn input_kind(function: &str, default: &super::program::InputValue) -> ValueType {
    use super::program::InputValue;
    if function != "input" {
        return input_type(function);
    }
    match default {
        InputValue::Int(_) => ValueType::Int,
        InputValue::Float(_) | InputValue::Source(_) => ValueType::Float,
        InputValue::Bool(_) => ValueType::Bool,
        InputValue::Color(_) => ValueType::Color,
        InputValue::String(_) => ValueType::String,
    }
}

/// How an `input.*` function's input is edited, beyond its value type.
pub fn input_control(function: &str) -> super::program::InputControl {
    use super::program::InputControl;
    match function {
        "input.price" => InputControl::Price,
        "input.symbol" => InputControl::Symbol,
        "input.timeframe" => InputControl::Timeframe,
        "input.session" => InputControl::Session,
        "input.text_area" => InputControl::TextArea,
        _ => InputControl::Standard,
    }
}

/// Pine transparency runs from 0 (opaque) to 100 (invisible); the alpha byte is the rest.
pub fn with_transparency(rgba: u32, transparency: f64) -> u32 {
    let clamped = if transparency.is_finite() {
        transparency.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let alpha = ((100.0 - clamped) * 2.55).round() as u32;
    (rgba & 0xFFFF_FF00) | alpha
}

/// `color.r/g/b/t`: a channel (0–255) or the transparency (0 opaque … 100 invisible).
pub fn color_component(rgba: u32, which: char) -> f64 {
    match which {
        'r' => f64::from(rgba >> 24),
        'g' => f64::from((rgba >> 16) & 0xff),
        'b' => f64::from((rgba >> 8) & 0xff),
        _ => 100.0 - f64::from(rgba & 0xff) / 2.55,
    }
}

pub fn rgb(r: f64, g: f64, b: f64, transparency: f64) -> u32 {
    fn channel(value: f64) -> u32 {
        let value = if value.is_finite() { value } else { 0.0 };
        value.clamp(0.0, 255.0) as u32
    }
    with_transparency(
        (channel(r) << 24) | (channel(g) << 16) | (channel(b) << 8) | 0xFF,
        transparency,
    )
}

/// `color.from_gradient`: each RGBA channel runs linearly from `bottom_color` at `bottom` to
/// `top_color` at `top`.
pub fn gradient(value: f64, bottom: f64, top: f64, bottom_color: u32, top_color: u32) -> u32 {
    if top == bottom {
        return if value >= top {
            top_color
        } else {
            bottom_color
        };
    }
    let t = ((value - bottom) / (top - bottom)).clamp(0.0, 1.0);
    let mut result = 0u32;
    for shift in [24u32, 16, 8, 0] {
        let from = ((bottom_color >> shift) & 0xFF) as f64;
        let to = ((top_color >> shift) & 0xFF) as f64;
        result |= ((from + (to - from) * t).round() as u32) << shift;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_sets_the_alpha_byte() {
        assert_eq!(with_transparency(0xFF00_00FF, 0.0), 0xFF00_00FF);
        assert_eq!(with_transparency(0xFF00_0000, 100.0), 0xFF00_0000);
        assert_eq!(with_transparency(0x0000_00ff, 50.0) & 0xFF, 127);
    }

    #[test]
    fn rgb_packs_channels() {
        assert_eq!(rgb(255.0, 128.0, 0.0, 0.0), 0xFF80_00FF);
    }

    #[test]
    fn input_types_are_string_by_default() {
        assert_eq!(input_type("input.int"), ValueType::Int);
        assert_eq!(input_type("input.float"), ValueType::Float);
        assert_eq!(input_type("input.session"), ValueType::String);
    }

    #[test]
    fn colour_components_and_gradients() {
        let teal = 0x0089_7bff;
        assert_eq!(color_component(teal, 'r'), 0.0);
        assert_eq!(color_component(teal, 'g'), 137.0);
        assert_eq!(color_component(teal, 'b'), 123.0);
        assert_eq!(color_component(teal, 't').round(), 0.0);
        assert_eq!(
            color_component(with_transparency(teal, 50.0), 't').round(),
            50.0
        );

        let (red, blue) = (0xff00_00ff, 0x0000_ffff);
        assert_eq!(gradient(0.0, 0.0, 10.0, red, blue), red);
        assert_eq!(gradient(10.0, 0.0, 10.0, red, blue), blue);
        assert_eq!(gradient(5.0, 0.0, 10.0, red, blue), 0x8000_80ff);
        assert_eq!(gradient(99.0, 0.0, 10.0, red, blue), blue, "clamped");
        assert_eq!(gradient(1.0, 5.0, 5.0, red, blue), red, "a flat range");
    }
}
