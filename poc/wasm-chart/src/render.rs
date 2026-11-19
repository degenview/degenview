//! Candlestick drawing on a Canvas 2D context, in CSS pixels.

use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

use degenview_market::{Kline, price_ticks};

const AXIS_WIDTH: f64 = 70.0;
const AXIS_HEIGHT: f64 = 26.0;
const PAD_TOP: f64 = 12.0;
const PAD_LEFT: f64 = 8.0;
const PRICE_ROW_PX: f64 = 56.0;
const TIME_LABEL_PX: f64 = 84.0;

const BACKGROUND: &str = "#131722";
const GRID: &str = "#232733";
const TEXT: &str = "#8b93a7";
const UP: &str = "#26a69a";
const DOWN: &str = "#ef5350";

/// The drawing area and its price scale.
struct Plot {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    low: f64,
    high: f64,
}

impl Plot {
    fn new(size: (f64, f64), candles: &[Kline]) -> Self {
        let low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let high = candles
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let pad = ((high - low) * 0.06).max(f64::EPSILON);
        Self {
            left: PAD_LEFT,
            top: PAD_TOP,
            width: (size.0 - PAD_LEFT - AXIS_WIDTH).max(1.0),
            height: (size.1 - PAD_TOP - AXIS_HEIGHT).max(1.0),
            low: low - pad,
            high: high + pad,
        }
    }

    fn right(&self) -> f64 {
        self.left + self.width
    }

    fn bottom(&self) -> f64 {
        self.top + self.height
    }

    fn y(&self, price: f64) -> f64 {
        self.bottom() - (price - self.low) / (self.high - self.low) * self.height
    }
}

/// Paints `candles` (oldest first) filling a `size` canvas. `granularity` is a bar's length in
/// seconds and picks the time-axis label format.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    size: (f64, f64),
    candles: &[Kline],
    granularity: u32,
) -> Result<(), JsValue> {
    ctx.set_fill_style_str(BACKGROUND);
    ctx.fill_rect(0.0, 0.0, size.0, size.1);
    if candles.is_empty() {
        return Ok(());
    }
    let plot = Plot::new(size, candles);
    let slot = plot.width / candles.len() as f64;

    ctx.set_font("11px -apple-system, system-ui, sans-serif");
    draw_price_axis(ctx, &plot)?;
    draw_time_axis(ctx, &plot, candles, slot, granularity)?;
    for (i, candle) in candles.iter().enumerate() {
        draw_candle(
            ctx,
            &plot,
            candle,
            plot.left + (i as f64 + 0.5) * slot,
            slot,
        );
    }
    draw_last_price(ctx, &plot, &candles[candles.len() - 1])
}

fn draw_price_axis(ctx: &CanvasRenderingContext2d, plot: &Plot) -> Result<(), JsValue> {
    let budget = (plot.height / PRICE_ROW_PX).max(1.0) as usize;
    let ticks = price_ticks(plot.low, plot.high, false, budget);
    let step = ticks.windows(2).map(|pair| pair[1] - pair[0]).next();
    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    for price in ticks {
        let y = plot.y(price).round() + 0.5;
        ctx.set_stroke_style_str(GRID);
        ctx.set_line_width(1.0);
        line(ctx, plot.left, y, plot.right(), y);
        ctx.set_fill_style_str(TEXT);
        let label = format_price(price, step.unwrap_or_else(|| resolution(price)));
        ctx.fill_text(&label, plot.right() + 8.0, y)?;
    }
    Ok(())
}

fn draw_time_axis(
    ctx: &CanvasRenderingContext2d,
    plot: &Plot,
    candles: &[Kline],
    slot: f64,
    granularity: u32,
) -> Result<(), JsValue> {
    let every = (TIME_LABEL_PX / slot).ceil().max(1.0) as usize;
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    for (i, candle) in candles.iter().enumerate().skip(every / 2).step_by(every) {
        let x = (plot.left + (i as f64 + 0.5) * slot).round() + 0.5;
        ctx.set_stroke_style_str(GRID);
        line(ctx, x, plot.top, x, plot.bottom());
        ctx.set_fill_style_str(TEXT);
        ctx.fill_text(
            &time_label(candle.open_time, granularity),
            x,
            plot.bottom() + 8.0,
        )?;
    }
    Ok(())
}

fn draw_candle(ctx: &CanvasRenderingContext2d, plot: &Plot, candle: &Kline, x: f64, slot: f64) {
    let color = if candle.is_up() { UP } else { DOWN };
    let wick_x = x.round() + 0.5;
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.0);
    line(ctx, wick_x, plot.y(candle.high), wick_x, plot.y(candle.low));

    let body_width = (slot * 0.72).max(1.0).floor();
    let top = plot.y(candle.open.max(candle.close));
    let height = (plot.y(candle.open.min(candle.close)) - top).max(1.0);
    ctx.set_fill_style_str(color);
    ctx.fill_rect(
        (x - body_width / 2.0).round(),
        top.round(),
        body_width,
        height.round(),
    );
}

fn draw_last_price(
    ctx: &CanvasRenderingContext2d,
    plot: &Plot,
    last: &Kline,
) -> Result<(), JsValue> {
    let color = if last.is_up() { UP } else { DOWN };
    let y = plot.y(last.close).round();
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.0);
    ctx.set_line_dash(&js_sys::Array::of2(&4.0.into(), &4.0.into()))?;
    line(ctx, plot.left, y + 0.5, plot.right(), y + 0.5);
    ctx.set_line_dash(&js_sys::Array::new())?;

    ctx.set_fill_style_str(color);
    ctx.fill_rect(plot.right(), y - 9.0, AXIS_WIDTH, 18.0);
    ctx.set_fill_style_str("#ffffff");
    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    ctx.fill_text(
        &format_price(last.close, resolution(last.close)),
        plot.right() + 8.0,
        y,
    )
}

fn line(ctx: &CanvasRenderingContext2d, x0: f64, y0: f64, x1: f64, y1: f64) {
    ctx.begin_path();
    ctx.move_to(x0, y0);
    ctx.line_to(x1, y1);
    ctx.stroke();
}

/// A step that prints `price` to about five significant digits, for a price that is not on a tick.
fn resolution(price: f64) -> f64 {
    (price.abs() / 100_000.0).max(1e-10)
}

/// `price` with just enough decimals to tell `step` apart.
fn format_price(price: f64, step: f64) -> String {
    let decimals = (-step.log10().floor()).clamp(0.0, 10.0) as usize;
    format!("{price:.decimals$}")
}

fn time_label(open_time_ms: i64, granularity: u32) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = js_sys::Date::new(&JsValue::from_f64(open_time_ms as f64));
    let day = format!(
        "{} {}",
        MONTHS[date.get_month() as usize % 12],
        date.get_date()
    );
    if granularity >= 86_400 || (date.get_hours() == 0 && date.get_minutes() == 0) {
        day
    } else {
        format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_price_matches_the_step() {
        assert_eq!(format_price(91_500.0, 500.0), "91500");
        assert_eq!(format_price(3.5, 0.5), "3.5");
        assert_eq!(format_price(0.1234, 0.001), "0.123");
    }

    #[test]
    fn price_y_axis_is_inverted() {
        let candles = [Kline {
            open_time: 0,
            close_time: 59_999,
            open: 10.0,
            high: 20.0,
            low: 10.0,
            close: 20.0,
            volume: 1.0,
        }];
        let plot = Plot::new((400.0, 300.0), &candles);
        assert!(plot.y(20.0) < plot.y(10.0));
        assert!(plot.y(plot.high) >= plot.top - 1e-9);
        assert!(plot.y(plot.low) <= plot.bottom() + 1e-9);
    }
}
