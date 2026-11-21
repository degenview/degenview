//! Runtime values, ported from `PineRuntimeValue.swift` + `PineRefKind.swift`. These are the
//! leaf types every other runtime module builds on.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// The value as a finite number, or None: `na`, a non-numeric value, and any float that is
    /// infinite or NaN, which are na to every builtin, plot, drawing, order and `ta.*` input.
    /// The one place that rule lives; `raw_number` is for the few readers that need the IEEE value.
    pub fn number(&self) -> Option<f64> {
        self.raw_number().filter(|number| number.is_finite())
    }

    /// The value as an `f64` whatever it is, an infinity included. Only arithmetic, the ordering
    /// operators and `math.max`/`math.min` read this: TradingView carries a zero-divisor quotient as
    /// an IEEE infinity through them while `na()`, `nz()`, `bool()` and `==` see na.
    pub fn raw_number(&self) -> Option<f64> {
        match self {
            Self::Int(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// Whether `na()` is true: `na`, or a float that is not finite.
    pub fn is_na(&self) -> bool {
        match self {
            Self::Na => true,
            Self::Float(value) => !value.is_finite(),
            _ => false,
        }
    }

    /// A float result of arithmetic: an infinity stays one, NaN is `na`.
    pub fn ieee(value: f64) -> Self {
        if value.is_nan() {
            Self::Na
        } else {
            Self::Float(value)
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
    fn number_is_finite_and_raw_number_is_ieee() {
        for non_finite in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(RuntimeValue::Float(non_finite).number(), None);
            assert_eq!(RuntimeValue::Float(non_finite).int_value(), None);
            assert!(RuntimeValue::Float(non_finite).is_na());
        }
        assert_eq!(
            RuntimeValue::Float(f64::INFINITY).raw_number(),
            Some(f64::INFINITY)
        );
        assert_eq!(RuntimeValue::Float(2.5).raw_number(), Some(2.5));
        assert_eq!(RuntimeValue::Int(3).raw_number(), Some(3.0));
        assert_eq!(RuntimeValue::Na.raw_number(), None);
        assert_eq!(RuntimeValue::String("x".into()).raw_number(), None);
        assert!(RuntimeValue::Na.is_na());
        assert!(!RuntimeValue::Float(0.0).is_na());
        assert!(!RuntimeValue::Bool(false).is_na());
    }

    #[test]
    fn ieee_turns_nan_into_na_and_keeps_infinities() {
        assert_eq!(RuntimeValue::ieee(f64::NAN), RuntimeValue::Na);
        assert_eq!(
            RuntimeValue::ieee(f64::INFINITY),
            RuntimeValue::Float(f64::INFINITY)
        );
        assert_eq!(RuntimeValue::ieee(1.5), RuntimeValue::Float(1.5));
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
