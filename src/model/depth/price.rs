//! A price as an exact integer, so a book can be keyed and folded by price without float rounding.

use crate::model::{PriceFormat, SmallPrices};
use std::ops::{Add, Sub};

/// Prices are kept in units of 1e-8: the finest an exchange quotes (Binance spot has 8 decimals).
const SCALE: i64 = 100_000_000;
const DECIMALS: usize = 8;

/// A price, or a step between two (a tick, a row size), in units of 1e-8.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Price(i64);

impl Price {
    pub const ZERO: Self = Self(0);

    /// Exactly the decimal `text` says (`"58884.13000000"`). `None` for anything that is not a
    /// plain non-negative decimal, that needs more than eight places, or that does not fit.
    pub fn parse(text: &str) -> Option<Self> {
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
            return None;
        }
        let (kept, extra) = fraction.split_at(fraction.len().min(DECIMALS));
        if extra.bytes().any(|byte| byte != b'0') {
            return None;
        }
        let whole: i64 = if whole.is_empty() {
            0
        } else {
            whole.parse().ok()?
        };
        let fraction: i64 = if kept.is_empty() {
            0
        } else {
            kept.parse().ok()?
        };
        let places = DECIMALS - kept.len();
        let units = whole
            .checked_mul(SCALE)?
            .checked_add(fraction.checked_mul(10_i64.pow(places as u32))?)?;
        Some(Self(units))
    }

    pub const fn from_units(units: i64) -> Self {
        Self(units)
    }

    pub const fn units(self) -> i64 {
        self.0
    }

    /// For arithmetic that does not need to be exact: sizes times a price, scales, positions.
    pub fn as_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }

    /// How many places after the point write this price exactly: 2 for `0.01`, 0 for `10`.
    pub fn decimals(self) -> u8 {
        let mut units = self.0;
        let mut places = DECIMALS;
        while places > 0 && units % 10 == 0 {
            units /= 10;
            places -= 1;
        }
        places as u8
    }

    /// The largest multiple of `step` not above this price.
    pub fn floor_to(self, step: Self) -> Self {
        Self(self.0.div_euclid(step.0) * step.0)
    }

    /// The smallest multiple of `step` not below this price.
    pub fn ceil_to(self, step: Self) -> Self {
        Self(-(-self.0).div_euclid(step.0) * step.0)
    }

    /// This price times a whole number (a step times a count of rows).
    pub fn times(self, count: i64) -> Self {
        Self(self.0.saturating_mul(count))
    }

    /// How many whole `step`s fit between zero and this price.
    pub fn steps_of(self, step: Self) -> i64 {
        self.0.div_euclid(step.0)
    }

    /// Written with exactly `decimals` places and thousands grouped, as a chart prints a price.
    pub fn format(self, decimals: u8) -> String {
        PriceFormat {
            decimals: Some(decimals),
            small: SmallPrices::Decimals,
        }
        .format(self.as_f64())
    }
}

impl Add for Price {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl Sub for Price {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
