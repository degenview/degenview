//! Exact money arithmetic. Sources hand over `f64`s (the paper engine) or text (files, brokers);
//! everything the journal adds up is a [`Decimal`], so a thousand fills never drift.

use rust_decimal::Decimal;
use std::str::FromStr;

/// The decimal a finite `f64` prints as: its shortest round-trip form, so `0.1` is exactly
/// `0.1` and not the binary neighbour. `None` for NaN, infinities and values too large for a
/// `Decimal`.
pub fn from_f64(value: f64) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    // Rust never prints an `f64` in scientific notation with `{}`.
    Decimal::from_str(&format!("{value}")).ok()
}

/// `numerator / denominator`, or `None` for a zero denominator (never a panic, never infinity).
pub fn ratio(numerator: Decimal, denominator: Decimal) -> Option<Decimal> {
    numerator.checked_div(denominator)
}

/// `part` of `total` in proportion `quantity / of_quantity`, kept exact where the split leaves no
/// remainder and rounded to 28 digits (the type's limit) otherwise. The caller keeps the
/// remainder as `total - share` so shares always add back up.
pub fn share(total: Decimal, quantity: Decimal, of_quantity: Decimal) -> Decimal {
    if of_quantity.is_zero() {
        return Decimal::ZERO;
    }
    if quantity == of_quantity {
        return total;
    }
    total
        .checked_mul(quantity)
        .and_then(|product| product.checked_div(of_quantity))
        .unwrap_or(Decimal::ZERO)
}

/// Volume-weighted average of `(quantity, price)` pairs; `None` when there is no quantity.
pub fn weighted_average(parts: impl IntoIterator<Item = (Decimal, Decimal)>) -> Option<Decimal> {
    let mut quantity = Decimal::ZERO;
    let mut value = Decimal::ZERO;
    for (part_quantity, price) in parts {
        quantity += part_quantity;
        value += part_quantity * price;
    }
    ratio(value, quantity)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    #[test]
    fn f64_converts_to_its_shortest_decimal() {
        assert_eq!(from_f64(0.1), Some(d("0.1")));
        assert_eq!(from_f64(100_000.0), Some(d("100000")));
        assert_eq!(from_f64(0.000_000_01), Some(d("0.00000001")));
        assert_eq!(from_f64(-2.5), Some(d("-2.5")));
        assert_eq!(from_f64(0.1 + 0.2), Some(d("0.30000000000000004")));
    }

    #[test]
    fn non_finite_values_have_no_decimal() {
        assert_eq!(from_f64(f64::NAN), None);
        assert_eq!(from_f64(f64::INFINITY), None);
        assert_eq!(from_f64(f64::NEG_INFINITY), None);
        assert_eq!(from_f64(f64::MAX), None);
    }

    #[test]
    fn ratio_never_divides_by_zero() {
        assert_eq!(ratio(d("1"), Decimal::ZERO), None);
        assert_eq!(ratio(d("1"), d("4")), Some(d("0.25")));
    }

    #[test]
    fn a_share_of_the_whole_is_the_whole() {
        assert_eq!(share(d("1.5"), d("2"), d("2")), d("1.5"));
        assert_eq!(share(d("3"), d("1"), d("4")), d("0.75"));
        assert_eq!(share(d("3"), d("1"), Decimal::ZERO), Decimal::ZERO);
    }

    #[test]
    fn weighted_average_of_fills() {
        let average = weighted_average([(d("1"), d("100")), (d("0.5"), d("103"))]).unwrap();
        // (100 + 51.5) / 1.5
        assert_eq!(average.round_dp(10), d("101.0000000000"));
        assert_eq!(weighted_average([]), None);
    }
}
