//! Runtime values, ported from `PineRuntimeValue.swift` + `PineRefKind.swift`. These are the
//! leaf types every other runtime module builds on.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefKind {
    Array,
    Line,
    Label,
    Box,
    Table,
    Plot,
    Object,
    Linefill,
    Map,
    Polyline,
    Matrix,
    /// A `request.footprint()` result; the handle is the chart bar's index (`pine::footprint_call`).
    Footprint,
    /// A row of a footprint; the handle is the bar's index and the row's number.
    VolumeRow,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Tuple(Vec<RuntimeValue>),
    /// Handle to a runtime-owned object (array, drawing, plot).
    Ref(RefKind, usize),
    Na,
    Void,
}

impl RuntimeValue {
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Int(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    /// The number as an `i64`, or None when not numeric or out of exact range.
    pub fn int_value(&self) -> Option<i64> {
        let number = self.number()?;
        if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
            Some(number as i64)
        } else {
            None
        }
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// A whole number as `.int` when it is exactly representable, otherwise `.float`.
    pub fn integral(value: f64) -> Self {
        if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
            Self::Int(value as i64)
        } else {
            Self::Float(value)
        }
    }
}

/// Absent → `fallback`; a color → itself; `na` (or anything else) → no color.
pub fn color_value(value: Option<&RuntimeValue>, fallback: Option<u32>) -> Option<u32> {
    match value {
        None => fallback,
        Some(RuntimeValue::Color(color)) => Some(*color),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_and_int_value() {
        assert_eq!(RuntimeValue::Int(3).number(), Some(3.0));
        assert_eq!(RuntimeValue::Float(2.5).number(), Some(2.5));
        assert_eq!(RuntimeValue::String("x".into()).number(), None);
        assert_eq!(RuntimeValue::Float(2.5).int_value(), None);
        assert_eq!(RuntimeValue::Float(3.0).int_value(), Some(3));
    }

    #[test]
    fn integral_picks_int_or_float() {
        assert_eq!(RuntimeValue::integral(5.0), RuntimeValue::Int(5));
        assert_eq!(RuntimeValue::integral(5.5), RuntimeValue::Float(5.5));
        assert_eq!(RuntimeValue::integral(1e300), RuntimeValue::Float(1e300));
    }

    #[test]
    fn color_value_prefers_the_color() {
        assert_eq!(color_value(None, Some(7)), Some(7));
        assert_eq!(color_value(Some(&RuntimeValue::Color(9)), Some(7)), Some(9));
        assert_eq!(color_value(Some(&RuntimeValue::Na), Some(7)), None);
    }
}
