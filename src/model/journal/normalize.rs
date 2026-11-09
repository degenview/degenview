//! The one validation every execution passes before it is stored, whatever its source. A record
//! that fails is reported back with its reason, never stored and never silently dropped.

use rust_decimal::Decimal;

use super::model::Execution;

/// Quantity, price, fee and multiplier must stay far inside a `Decimal`'s range so that sums and
/// products over a whole history cannot overflow.
const LIMIT: i64 = 1_000_000_000_000; // 1e12

pub fn validate(execution: &Execution) -> Result<(), String> {
    if execution.uid.trim().is_empty() {
        return Err("the execution has no identity".to_owned());
    }
    if execution.account_uid.trim().is_empty() {
        return Err("the execution names no account".to_owned());
    }
    if execution.source_exec_id.trim().is_empty() {
        return Err("the execution has no source id".to_owned());
    }
    if execution.instrument.key.trim().is_empty() {
        return Err("the execution names no instrument".to_owned());
    }
    if execution.quantity <= Decimal::ZERO {
        return Err(format!("quantity {} is not positive", execution.quantity));
    }
    if execution.price <= Decimal::ZERO {
        return Err(format!("price {} is not positive", execution.price));
    }
    if execution.multiplier <= Decimal::ZERO {
        return Err(format!(
            "contract multiplier {} is not positive",
            execution.multiplier
        ));
    }
    if let Some(fee) = execution.fee
        && fee < Decimal::ZERO
    {
        return Err(format!("fee {fee} is negative"));
    }
    if let Some(stop) = execution.planned_stop
        && stop <= Decimal::ZERO
    {
        return Err(format!("planned stop {stop} is not positive"));
    }
    let limit = Decimal::from(LIMIT);
    for (name, value) in [
        ("quantity", execution.quantity),
        ("price", execution.price),
        ("multiplier", execution.multiplier),
        ("fee", execution.fee.unwrap_or(Decimal::ZERO)),
    ] {
        if value > limit {
            return Err(format!("{name} {value} is out of range"));
        }
    }
    if execution.ts_ms <= 0 {
        return Err(format!("timestamp {} is not a time", execution.ts_ms));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::Side;
    use crate::model::journal::testing::{d, exec};

    fn good() -> Execution {
        exec("1", Side::Buy, "1", "100", 1_000)
    }

    #[test]
    fn a_normal_fill_passes() {
        assert_eq!(validate(&good()), Ok(()));
    }

    #[test]
    fn zero_and_negative_values_are_refused() {
        let mut zero_quantity = good();
        zero_quantity.quantity = Decimal::ZERO;
        assert!(validate(&zero_quantity).unwrap_err().contains("quantity"));

        let mut negative_price = good();
        negative_price.price = d("-1");
        assert!(validate(&negative_price).unwrap_err().contains("price"));

        assert!(validate(&good().fee("-0.1")).unwrap_err().contains("fee"));
        assert!(
            validate(&good().times("0"))
                .unwrap_err()
                .contains("multiplier")
        );
        assert!(validate(&good().stop("0")).unwrap_err().contains("stop"));
    }

    #[test]
    fn huge_values_are_refused_before_they_can_overflow() {
        let mut huge = good();
        huge.price = d("1000000000001");
        assert!(validate(&huge).unwrap_err().contains("out of range"));
    }

    #[test]
    fn identity_and_time_are_required() {
        let mut no_uid = good();
        no_uid.uid.clear();
        assert!(validate(&no_uid).is_err());
        let mut no_time = good();
        no_time.ts_ms = 0;
        assert!(validate(&no_time).unwrap_err().contains("timestamp"));
    }
}
