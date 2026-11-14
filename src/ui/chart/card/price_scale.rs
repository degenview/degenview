//! The price scale: the one map between a price and a height, and what the axis says along it.

use super::*;

/// The one map between a price and a height, and what the axis says at a height.
///
/// The map is linear, or logarithmic where `logarithmic` is set. `mode` and `reference` only
/// change what the axis, its tags and the crosshair say: Percent and Indexed are linear in the
/// price, so they move no pixel and every price a chart stores or sends stays a real price.
pub(in crate::ui) struct PriceScale {
    pub(in crate::ui) minimum: f64,
    pub(in crate::ui) maximum: f64,
    pub(in crate::ui) top: f32,
    pub(in crate::ui) height: f32,
    /// Whether this frame is drawn on a logarithmic map: the mode asks for it and the view has
    /// no price a logarithm cannot take ([`chart::price_scale::can_draw_logarithmic`]).
    pub(in crate::ui) logarithmic: bool,
    pub(in crate::ui) mode: PriceScaleMode,
    /// What Percent and Indexed measure from: the close of the first bar shown.
    pub(in crate::ui) reference: Option<f64>,
}

/// The labelled prices of an axis, and the distance between two of them as the axis reads it.
pub(in crate::ui) struct AxisTicks {
    pub(in crate::ui) prices: Vec<f64>,
    pub(in crate::ui) step: f64,
}

impl PriceScale {
    /// A linear scale that reads prices.
    pub(in crate::ui) fn linear(minimum: f64, maximum: f64, top: f32, height: f32) -> Self {
        Self {
            minimum,
            maximum,
            top,
            height,
            logarithmic: false,
            mode: PriceScaleMode::Regular,
            reference: None,
        }
    }

    fn project(&self, price: f64) -> f64 {
        if self.logarithmic {
            // Only a price below the range gets here with no logarithm, and it is off the plot.
            price.max(f64::MIN_POSITIVE).ln()
        } else {
            price
        }
    }

    /// The mode the axis reads in: a relative one only while it has a reference to measure from.
    fn reading(&self) -> Option<(PriceScaleMode, f64)> {
        let reference = self.reference?;
        self.mode.is_relative().then_some((self.mode, reference))
    }

    /// The prices the axis labels, where the grid is drawn too. A relative mode picks its ticks
    /// in the change it shows, so they are round there (`-10%`, `0%`, `+10%`) and `0%` sits on
    /// the reference, and maps them back to prices.
    pub(in crate::ui) fn axis_ticks(&self, budget: usize) -> AxisTicks {
        let Some((mode, reference)) = self.reading() else {
            let prices =
                chart::canvas::price_ticks(self.minimum, self.maximum, self.logarithmic, budget);
            let step = tick_step(&prices);
            return AxisTicks { prices, step };
        };
        let shown = |price| chart::price_scale::display(mode, Some(reference), price);
        let (Some(low), Some(high)) = (shown(self.minimum), shown(self.maximum)) else {
            return AxisTicks {
                prices: Vec::new(),
                step: 0.0,
            };
        };
        let values = chart::canvas::price_ticks(low, high, false, budget);
        let step = tick_step(&values);
        let prices = values
            .into_iter()
            .filter_map(|value| chart::price_scale::from_display(mode, Some(reference), value))
            .collect();
        AxisTicks { prices, step }
    }

    /// An axis label for `price`; `step` is the distance between ticks as [`Self::axis_ticks`]
    /// gives it.
    pub(in crate::ui) fn tick_label(&self, format: PriceFormat, price: f64, step: f64) -> String {
        match self.reading() {
            Some((mode, reference)) => chart::price_scale::display(mode, Some(reference), price)
                .map_or_else(
                    || "—".to_owned(),
                    |value| chart::price_scale::format_relative(mode, value, Some(step)),
                ),
            None => format.format_tick(price, step),
        }
    }

    /// A tag on the axis (the last price, the crosshair, a line's level) as the axis reads.
    pub(in crate::ui) fn tag_label(&self, format: PriceFormat, price: f64) -> String {
        match self.reading() {
            Some((mode, reference)) => chart::price_scale::display(mode, Some(reference), price)
                .map_or_else(
                    || "—".to_owned(),
                    |value| chart::price_scale::format_relative(mode, value, None),
                ),
            None => format.format(price),
        }
    }

    pub(in crate::ui) fn y(&self, price: f64) -> f32 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction =
            ((self.project(price) - minimum) / (maximum - minimum).max(f64::EPSILON)) as f32;
        self.top + self.height * (1.0 - fraction)
    }

    pub(in crate::ui) fn price(&self, y: f32) -> f64 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction = 1.0 - f64::from((y - self.top) / self.height);
        let projected = minimum + fraction * (maximum - minimum);
        if self.logarithmic {
            projected.exp()
        } else {
            projected
        }
    }
}

/// The smallest distance between two neighbouring grid ticks, `0.0` when there are fewer than
/// two: how fine a label must be to tell its neighbours apart.
pub(super) fn tick_step(ticks: &[f64]) -> f64 {
    let step = ticks
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .filter(|gap| *gap > 0.0)
        .fold(f64::INFINITY, f64::min);
    if step.is_finite() { step } else { 0.0 }
}
