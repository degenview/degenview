//! Candlestick drawing on a Canvas 2D context, in CSS pixels.

use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

use degenview_market::price_format::PriceFormat;
use degenview_market::viewport::zoom::Slots;
use degenview_market::viewport::{ChartViewport, PriceRange, visible_window};
use degenview_market::{Kline, price_ticks};

use crate::view::{Layout, RESERVED, Rect};

const AXIS_WIDTH: f64 = 70.0;
const AXIS_HEIGHT: f64 = 26.0;
const PAD_TOP: f64 = 12.0;
const PAD_LEFT: f64 = 8.0;
const PRICE_ROW_PX: f64 = 56.0;
const TIME_LABEL_PX: f64 = 84.0;
const TAG_HEIGHT: f64 = 18.0;

/// How the app prints prices: adaptive precision, thousands grouped.
const PRICES: PriceFormat = PriceFormat::AUTO;

const BACKGROUND: &str = "#131722";
const GRID: &str = "#232733";
const TEXT: &str = "#8b93a7";
const UP: &str = "#26a69a";
const DOWN: &str = "#ef5350";
const CROSSHAIR: &str = "#5d6577";
const TAG: &str = "#2a2e39";
const TAG_TEXT: &str = "#d1d4dc";

/// What to paint: the candles, how much of them the viewport shows, and the pointer, if over the
/// chart.
pub struct Frame<'a> {
    pub candles: &'a [Kline],
    pub granularity: u32,
    pub viewport: &'a ChartViewport,
    pub pointer: Option<(f32, f32)>,
}

/// The plot rectangle and the prices it shows.
struct Plot {
    rect: Rect,
    range: PriceRange,
}

impl Plot {
    fn new(size: (f64, f64), range: PriceRange) -> Self {
        Self {
            rect: Rect {
                left: PAD_LEFT,
                top: PAD_TOP,
                width: (size.0 - PAD_LEFT - AXIS_WIDTH).max(1.0),
                height: (size.1 - PAD_TOP - AXIS_HEIGHT).max(1.0),
            },
            range,
        }
    }

    fn y(&self, price: f64) -> f64 {
        let span = self.range.maximum - self.range.minimum;
        self.rect.bottom() - (price - self.range.minimum) / span * self.rect.height
    }

    fn price_at(&self, y: f64) -> f64 {
        let span = self.range.maximum - self.range.minimum;
        self.range.minimum + (self.rect.bottom() - y) / self.rect.height * span
    }
}

/// The range the shown bars span, with room above and below: what autoscale shows.
fn auto_range(shown: &[Kline]) -> PriceRange {
    let low = shown.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let high = shown
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    if !(low.is_finite() && high.is_finite()) {
        return PriceRange::new(0.0, 1.0);
    }
    let pad = ((high - low) * 0.06).max(f64::EPSILON);
    PriceRange::new(low - pad, high + pad)
}

/// Paints the frame filling a `size` canvas and reports where it put the plot and its bars.
/// `granularity` is a bar's length in seconds and picks the time-axis label format.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    size: (f64, f64),
    frame: &Frame,
) -> Result<Layout, JsValue> {
    ctx.set_fill_style_str(BACKGROUND);
    ctx.fill_rect(0.0, 0.0, size.0, size.1);

    let horizontal = frame.viewport.horizontal;
    let window = visible_window(
        frame.candles.len(),
        horizontal.visible,
        horizontal.right_offset,
        RESERVED,
    );
    let auto = auto_range(&frame.candles[window.start..window.end]);
    let plot = Plot::new(size, frame.viewport.vertical.resolve(auto, false));
    let slots = Slots {
        first: window.start as f64 - f64::from(window.lead),
        left: plot.rect.left,
        width: plot.rect.width / window.slots as f64,
    };
    let layout = Layout {
        plot: plot.rect,
        shown: plot.range,
        slots,
        window,
    };
    if frame.candles.is_empty() {
        return Ok(layout);
    }

    ctx.set_font("11px -apple-system, system-ui, sans-serif");
    draw_price_axis(ctx, &plot)?;
    draw_time_axis(ctx, &plot, &layout, frame)?;
    draw_candles(ctx, &plot, &layout, frame.candles);
    draw_last_price(ctx, &plot, &frame.candles[frame.candles.len() - 1])?;
    if let Some(pointer) = frame.pointer {
        draw_crosshair(ctx, &plot, &layout, frame, pointer)?;
    }
    Ok(layout)
}

fn draw_price_axis(ctx: &CanvasRenderingContext2d, plot: &Plot) -> Result<(), JsValue> {
    let budget = (plot.rect.height / PRICE_ROW_PX).max(1.0) as usize;
    let ticks = price_ticks(plot.range.minimum, plot.range.maximum, false, budget);
    let step = ticks.windows(2).map(|pair| pair[1] - pair[0]).next();
    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    for price in ticks {
        let y = plot.y(price).round() + 0.5;
        ctx.set_stroke_style_str(GRID);
        ctx.set_line_width(1.0);
        line(ctx, plot.rect.left, y, plot.rect.right(), y);
        ctx.set_fill_style_str(TEXT);
        let label = match step {
            Some(step) => PRICES.format_tick(price, step),
            None => PRICES.format(price),
        };
        ctx.fill_text(&label, plot.rect.right() + 8.0, y)?;
    }
    Ok(())
}

fn draw_time_axis(
    ctx: &CanvasRenderingContext2d,
    plot: &Plot,
    layout: &Layout,
    frame: &Frame,
) -> Result<(), JsValue> {
    let every = (TIME_LABEL_PX / layout.slots.width).ceil().max(1.0) as usize;
    // Labels a day or more apart would all read the same time of day: name the day instead.
    let dates_only = every as u64 * u64::from(frame.granularity) >= 86_400;
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    let window = layout.window;
    // Labels sit on every n-th bar of the series, so they stay on their bars while the chart pans.
    for (i, candle) in frame.candles[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(offset, candle)| (window.start + offset, candle))
        .filter(|(i, _)| i % every == 0)
    {
        let x = layout.slots.x(i as f64 + 0.5).round() as f64 + 0.5;
        if x < plot.rect.left + 20.0 || x > plot.rect.right() - 20.0 {
            continue;
        }
        ctx.set_stroke_style_str(GRID);
        line(ctx, x, plot.rect.top, x, plot.rect.bottom());
        ctx.set_fill_style_str(TEXT);
        ctx.fill_text(
            &time_label(candle.open_time, frame.granularity, dates_only),
            x,
            plot.rect.bottom() + 8.0,
        )?;
    }
    Ok(())
}

/// The shown bars, clipped to the plot: the first and last may be cut by its edge.
fn draw_candles(ctx: &CanvasRenderingContext2d, plot: &Plot, layout: &Layout, candles: &[Kline]) {
    ctx.save();
    ctx.begin_path();
    ctx.rect(
        plot.rect.left,
        plot.rect.top,
        plot.rect.width,
        plot.rect.height,
    );
    ctx.clip();
    let window = layout.window;
    for (offset, candle) in candles[window.start..window.end].iter().enumerate() {
        let x = f64::from(layout.slots.x((window.start + offset) as f64 + 0.5));
        draw_candle(ctx, plot, candle, x, layout.slots.width);
    }
    ctx.restore();
}

fn draw_candle(ctx: &CanvasRenderingContext2d, plot: &Plot, candle: &Kline, x: f64, slot: f64) {
    let color = direction_color(candle);
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
    let y = plot.y(last.close).round();
    if y < plot.rect.top || y > plot.rect.bottom() {
        return Ok(());
    }
    let color = direction_color(last);
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.0);
    ctx.set_line_dash(&js_sys::Array::of2(&4.0.into(), &4.0.into()))?;
    line(ctx, plot.rect.left, y + 0.5, plot.rect.right(), y + 0.5);
    ctx.set_line_dash(&js_sys::Array::new())?;
    draw_price_tag(ctx, plot, y, &PRICES.format(last.close), color, "#ffffff")
}

/// A label on the price axis at `y`.
fn draw_price_tag(
    ctx: &CanvasRenderingContext2d,
    plot: &Plot,
    y: f64,
    text: &str,
    background: &str,
    foreground: &str,
) -> Result<(), JsValue> {
    ctx.set_fill_style_str(background);
    ctx.fill_rect(
        plot.rect.right(),
        y - TAG_HEIGHT / 2.0,
        AXIS_WIDTH,
        TAG_HEIGHT,
    );
    ctx.set_fill_style_str(foreground);
    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    ctx.fill_text(text, plot.rect.right() + 8.0, y)
}

/// Crosshair lines, the price and time under them, and the hovered bar's figures.
fn draw_crosshair(
    ctx: &CanvasRenderingContext2d,
    plot: &Plot,
    layout: &Layout,
    frame: &Frame,
    (px, py): (f32, f32),
) -> Result<(), JsValue> {
    let (x, y) = (f64::from(px), f64::from(py));
    let inside = (plot.rect.left..=plot.rect.right()).contains(&x)
        && (plot.rect.top..=plot.rect.bottom()).contains(&y);
    if !inside {
        return Ok(());
    }
    let bar = layout.bar_at(px, frame.candles.len());
    // The vertical line snaps to the bar's centre.
    let line_x = bar.map_or(x, |i| f64::from(layout.slots.x(i as f64 + 0.5)));

    ctx.set_stroke_style_str(CROSSHAIR);
    ctx.set_line_width(1.0);
    ctx.set_line_dash(&js_sys::Array::of2(&3.0.into(), &3.0.into()))?;
    line(
        ctx,
        line_x.round() + 0.5,
        plot.rect.top,
        line_x.round() + 0.5,
        plot.rect.bottom(),
    );
    line(
        ctx,
        plot.rect.left,
        y.round() + 0.5,
        plot.rect.right(),
        y.round() + 0.5,
    );
    ctx.set_line_dash(&js_sys::Array::new())?;

    draw_price_tag(
        ctx,
        plot,
        y,
        &PRICES.format(plot.price_at(y)),
        TAG,
        TAG_TEXT,
    )?;
    let Some(candle) = bar.map(|i| &frame.candles[i]) else {
        return Ok(());
    };
    let label = crosshair_time_label(candle.open_time, frame.granularity);
    ctx.set_fill_style_str(TAG);
    ctx.fill_rect(line_x - 44.0, plot.rect.bottom() + 2.0, 88.0, TAG_HEIGHT);
    ctx.set_fill_style_str(TAG_TEXT);
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    ctx.fill_text(&label, line_x, plot.rect.bottom() + 2.0 + TAG_HEIGHT / 2.0)?;

    ctx.set_fill_style_str(direction_color(candle));
    ctx.set_text_align("left");
    ctx.set_text_baseline("top");
    let change = (candle.close / candle.open - 1.0) * 100.0;
    ctx.fill_text(
        &format!(
            "O {}  H {}  L {}  C {}  {change:+.2}%",
            PRICES.format(candle.open),
            PRICES.format(candle.high),
            PRICES.format(candle.low),
            PRICES.format(candle.close),
        ),
        plot.rect.left + 8.0,
        plot.rect.top + 6.0,
    )
}

fn direction_color(candle: &Kline) -> &'static str {
    if candle.is_up() { UP } else { DOWN }
}

fn line(ctx: &CanvasRenderingContext2d, x0: f64, y0: f64, x1: f64, y1: f64) {
    ctx.begin_path();
    ctx.move_to(x0, y0);
    ctx.line_to(x1, y1);
    ctx.stroke();
}

/// A bar's time for the axis: the time of day, or the date at midnight, on a daily chart, or when
/// `dates_only`.
fn time_label(open_time_ms: i64, granularity: u32, dates_only: bool) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = js_sys::Date::new(&JsValue::from_f64(open_time_ms as f64));
    let day = format!(
        "{} {}",
        MONTHS[date.get_month() as usize % 12],
        date.get_date()
    );
    if dates_only || granularity >= 86_400 || (date.get_hours() == 0 && date.get_minutes() == 0) {
        day
    } else {
        format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
    }
}

/// The crosshair's bar: date, and the time of day on an intraday chart.
fn crosshair_time_label(open_time_ms: i64, granularity: u32) -> String {
    let day = time_label(open_time_ms, 86_400, true);
    if granularity >= 86_400 {
        day
    } else {
        format!("{day} {}", time_label(open_time_ms, granularity, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(low: f64, high: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 59_999,
            open: low,
            high,
            low,
            close: high,
            volume: 1.0,
        }
    }

    #[test]
    fn price_y_axis_is_inverted_and_round_trips() {
        let plot = Plot::new((400.0, 300.0), auto_range(&[bar(10.0, 20.0)]));
        assert!(plot.y(20.0) < plot.y(10.0));
        assert!(plot.y(plot.range.maximum) >= plot.rect.top - 1e-9);
        assert!(plot.y(plot.range.minimum) <= plot.rect.bottom() + 1e-9);
        assert!((plot.price_at(plot.y(15.0)) - 15.0).abs() < 1e-9);
    }

    #[test]
    fn autoscale_leaves_room_around_the_shown_bars() {
        let range = auto_range(&[bar(10.0, 20.0), bar(12.0, 30.0)]);
        assert!(range.minimum < 10.0 && range.maximum > 30.0);
        assert_eq!(auto_range(&[]), PriceRange::new(0.0, 1.0));
    }
}
