//! Where a drawing TradingView keeps beside a bar, rather than at a price, comes to sit. The
//! distance from the bar is a share of the room to the chart's edge, so it takes the candles
//! around the bar: they stand for the chart as it was looked at.

use super::drawing::RawDrawing;
use super::{interval, plan::Market};
use crate::model::{DataSource, Kline, Timeframe};

/// How many bars either side of the drawing's stand for the chart in view.
pub const WINDOW: i64 = 75;
/// The room TradingView leaves above and below the candles, as a share of their range.
const MARGIN: f64 = 0.1;

/// A place to be worked out from a market's candles.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Spot {
    pub source: DataSource,
    pub symbol: String,
    pub timeframe: Timeframe,
    /// The time of the bar the drawing is beside.
    pub time_ms: i64,
    /// [`RawDrawing::beside_bar`], as bits so a spot can be a key.
    position: u64,
}

impl Spot {
    /// The spot `raw` needs on `market`; `None` when it has a price, or its interval is not a
    /// length of time.
    pub fn of(raw: &RawDrawing, market: &Market) -> Option<Self> {
        let position = raw.beside_bar()?;
        Some(Self {
            source: market.source,
            symbol: market.symbol.clone(),
            timeframe: interval::timeframe(&raw.interval)?,
            time_ms: raw.points.first()?.time_ms(),
            position: position.to_bits(),
        })
    }

    /// The times, `[start, end)`, whose candles the price takes.
    pub fn range_ms(&self) -> (i64, i64) {
        let span = self.timeframe.nominal_ms().saturating_mul(WINDOW);
        (
            self.time_ms.saturating_sub(span),
            self.time_ms
                .saturating_add(span)
                .saturating_add(self.timeframe.nominal_ms()),
        )
    }

    /// The price, from candles over [`Self::range_ms`], oldest first; `None` when none of them is
    /// the drawing's own bar.
    pub fn price(&self, candles: &[Kline]) -> Option<f64> {
        let bar = candles
            .iter()
            .rev()
            .find(|candle| candle.open_time <= self.time_ms)
            .filter(|candle| self.time_ms - candle.open_time < self.timeframe.nominal_ms())?;
        let high = candles.iter().map(|c| c.high).fold(bar.high, f64::max);
        let low = candles.iter().map(|c| c.low).fold(bar.low, f64::min);
        let margin = (high - low) * MARGIN;
        let share = f64::from_bits(self.position) / 100.0;
        let price = if share >= 0.0 {
            bar.high + share * (high + margin - bar.high)
        } else {
            bar.low + share * (bar.low - (low - margin))
        };
        price.is_finite().then_some(price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tradingview::fixtures;
    use serde_json::json;

    const DAY: i64 = 86_400_000;

    fn spot(position: f64) -> Spot {
        let source = fixtures::drawing(
            "t",
            "LineToolTweet",
            "X:Y",
            json!({"interval": "D", "position": position}),
            &[(0, 7.0)],
        );
        let market = Market {
            source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            name: "BTCUSDT".into(),
            image_url: None,
        };
        Spot::of(&RawDrawing::read("t", &source).unwrap(), &market).unwrap()
    }

    fn candle(open_time: i64, low: f64, high: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + DAY - 1,
            open: low,
            high,
            low,
            close: high,
            volume: 1.0,
        }
    }

    /// Its bar runs from 90 to 110; the candles from 80 to 180, so with the margin 70 to 190.
    fn candles(spot: &Spot) -> Vec<Kline> {
        vec![
            candle(spot.time_ms - DAY, 80.0, 100.0),
            candle(spot.time_ms, 90.0, 110.0),
            candle(spot.time_ms + DAY, 120.0, 180.0),
        ]
    }

    #[test]
    fn a_share_of_the_room_above_the_bar_or_below_it() {
        for (position, price) in [(0.0, 110.0), (50.0, 150.0), (100.0, 190.0), (-50.0, 80.0)] {
            let spot = spot(position);
            assert_eq!(spot.price(&candles(&spot)), Some(price), "{position}");
        }
    }

    #[test]
    fn without_its_own_bar_there_is_no_price() {
        let spot = spot(50.0);
        assert_eq!(spot.price(&[]), None);
        assert_eq!(
            spot.price(&[candle(spot.time_ms - 3 * DAY, 1.0, 2.0)]),
            None
        );
        assert_eq!(spot.price(&[candle(spot.time_ms + DAY, 1.0, 2.0)]), None);
    }

    #[test]
    fn the_candles_asked_for_surround_the_bar() {
        let spot = spot(50.0);
        let (start, end) = spot.range_ms();
        assert_eq!(spot.time_ms - start, WINDOW * DAY);
        assert_eq!(end - spot.time_ms, (WINDOW + 1) * DAY);
    }
}
