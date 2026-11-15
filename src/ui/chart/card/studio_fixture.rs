//! A chart card with data that has been drawn until it settled: what the Screenshot Studio's tests
//! capture. The harness is the card's own tests', so the card is in the state a drawn chart is
//! in (its size, its scripts' runs), which a bare card is not.

use super::canvas_tests::{Harness, series, with_panes};
use super::snapshot_tests::{everything_on, settled};
use super::*;
use crate::model::{Anchor, ChartCanvas, Drawing, Tool};

const HOUR: i64 = super::canvas_tests::HOUR;

pub(in crate::ui) struct Fixture {
    harness: Harness,
}

impl Fixture {
    /// A plain chart of `bars` candles.
    pub(in crate::ui) fn plain(canvas: ChartCanvas, bars: usize) -> Self {
        let mut harness = Harness::new(canvas);
        harness.candles = series(bars);
        settled(&mut harness);
        Self { harness }
    }

    /// A chart of `bars` candles with every built-in indicator, two script panes and a trend
    /// line over its last bars.
    pub(in crate::ui) fn busy(canvas: ChartCanvas, bars: usize) -> Self {
        let mut harness = with_panes(canvas);
        harness.indicators = everything_on();
        harness.candles = series(bars);
        harness.drawings = vec![trend_line_near_the_end(&harness.candles)];
        settled(&mut harness);
        Self { harness }
    }

    /// Puts a trend line over the last bars.
    pub(in crate::ui) fn with_a_drawing(mut self) -> Self {
        self.harness.drawings = vec![trend_line_near_the_end(&self.harness.candles)];
        settled(&mut self.harness);
        self
    }

    pub(in crate::ui) fn with_indicators(mut self) -> Self {
        self.harness.indicators = everything_on();
        settled(&mut self.harness);
        self
    }

    pub(in crate::ui) fn card(&self) -> &ChartCard {
        &self.harness.card
    }

    /// Pretends the chart was drawn at `size`, as a chart much larger than any screen is.
    pub(in crate::ui) fn sized(mut self, size: Vec2) -> Self {
        self.harness.card.set_snapshot_size_for_tests(size);
        self
    }

    /// Everything about the card a picture of it must leave as it was, written out.
    pub(in crate::ui) fn fingerprint(&self) -> String {
        let card = &self.harness.card;
        format!(
            "{:?} {:?} {:?} {} {:?} {:?} {:?}",
            card.saved_viewport(),
            card.pane_ids,
            card.script_extent,
            card.near_oldest,
            card.constructed.as_ref().map(Arc::as_ptr),
            card.snapshot_size(),
            card.alert_lines,
        )
    }

    pub(in crate::ui) fn candles(&self) -> Vec<Kline> {
        self.harness.candles.clone()
    }

    pub(in crate::ui) fn indicators(&self) -> IndicatorConfig {
        self.harness.indicators
    }

    pub(in crate::ui) fn drawings(&self) -> Vec<Drawing> {
        self.harness.drawings.clone()
    }
}

/// A trend line across the chart's last stretch, rising over a few dozen bars.
fn trend_line_near_the_end(candles: &[Kline]) -> Drawing {
    let last = candles.len() as i64;
    let price = |bar: i64| candles[(bar as usize).min(candles.len() - 1)].close;
    let (from, to) = (last - 45, last - 8);
    Drawing::build(
        Tool::TrendLine,
        1,
        &[
            Anchor {
                time_ms: from * HOUR,
                price: price(from) + 6.0,
            },
            Anchor {
                time_ms: to * HOUR,
                price: price(to) + 12.0,
            },
        ],
        Tool::TrendLine.default_style(),
    )
    .expect("builds")
}
