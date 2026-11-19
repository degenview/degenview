//! A Coinbase candlestick chart, fetched and drawn by Rust running as WebAssembly.
//!
//! JavaScript only wires the page: it calls [`load_chart`] to fetch candles and
//! [`Chart::draw`] to paint them on a `<canvas>`.

mod coinbase;
mod live;
mod render;
mod view;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use degenview_market::coinbase::{STREAM_HOST, parse_ticker, subscribe_message};
use degenview_market::{Kline, Timeframe};

use coinbase::Fetched;
use live::Live;

use render::Frame;
use view::View;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Candles for one market and timeframe, and what part of them the user is looking at.
#[wasm_bindgen]
pub struct Chart {
    candles: Vec<Kline>,
    granularity: u32,
    product: String,
    view: View,
    live: Live,
}

/// A fresh REST answer to merge into a chart that has been taking live trades.
#[wasm_bindgen]
pub struct Snapshot(Fetched);

#[wasm_bindgen]
impl Chart {
    /// Paints the chart over the whole canvas, sharp on high-DPI screens. Call it again after the
    /// canvas is resized or the view changes.
    pub fn draw(&mut self, canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
        let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        let size = (
            f64::from(canvas.client_width()),
            f64::from(canvas.client_height()),
        );
        canvas.set_width((size.0 * ratio) as u32);
        canvas.set_height((size.1 * ratio) as u32);

        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or("canvas has no 2d context")?
            .dyn_into()?;
        ctx.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0)?;
        let frame = Frame {
            candles: &self.candles,
            granularity: self.granularity,
            viewport: self.view.viewport(),
            pointer: self.view.pointer(),
        };
        let layout = render::draw(&ctx, size, &frame)?;
        self.view.set_layout(layout);
        Ok(())
    }

    /// A press at `(x, y)`, in the canvas's CSS pixels: starts dragging the chart.
    pub fn pointer_down(&mut self, x: f32, y: f32) {
        self.view.press(x, y);
    }

    /// The pointer moved: drags the chart if it is pressed, else only moves the crosshair.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.view.move_to(x, y);
    }

    pub fn pointer_up(&mut self) {
        self.view.release();
    }

    /// The pointer left the chart: the crosshair goes.
    pub fn pointer_leave(&mut self) {
        self.view.leave();
    }

    /// Whether a drag is under way.
    #[wasm_bindgen(getter)]
    pub fn dragging(&self) -> bool {
        self.view.is_dragging()
    }

    /// A wheel turn of `delta_y` at `x`: zooms the time axis about the pointer.
    pub fn wheel(&mut self, x: f32, delta_y: f64) {
        self.view.wheel(x, delta_y);
    }

    /// Back to the newest bars at the natural scale.
    pub fn reset_view(&mut self) {
        self.view.reset();
    }

    fn bars_changed(&mut self, before: usize) {
        let now = self.candles.len();
        self.view.bars_changed(now, now.saturating_sub(before));
    }

    /// The websocket feed's address.
    #[wasm_bindgen(getter)]
    pub fn feed_url(&self) -> String {
        format!("wss://{STREAM_HOST}")
    }

    /// What to send once the feed's socket is open.
    #[wasm_bindgen(getter)]
    pub fn subscribe_message(&self) -> String {
        subscribe_message(&[&self.product])
    }

    /// Folds a feed message that arrived at `now` (the page's `performance.now()`) into the
    /// candles. Returns whether they changed.
    pub fn on_message(&mut self, text: &str, now: f64) -> bool {
        let Some(tick) = parse_ticker(text).filter(|tick| tick.product_id == self.product) else {
            return false;
        };
        let before = self.candles.len();
        let changed = self.live.apply(&tick, now, &mut self.candles);
        self.bars_changed(before);
        changed
    }

    /// Merges a REST answer whose request began at `started` (`performance.now()`): the bars the
    /// feed delivered since win over it.
    pub fn merge(&mut self, snapshot: &Snapshot, started: f64) {
        let before = self.candles.len();
        self.live.merge(&snapshot.0, started, &mut self.candles);
        self.bars_changed(before);
    }

    /// Close of the newest bar, or NaN when Coinbase returned none.
    #[wasm_bindgen(getter)]
    pub fn last_close(&self) -> f64 {
        self.candles.last().map_or(f64::NAN, |c| c.close)
    }

    /// Move from the oldest bar's open to the newest close, in percent, or NaN without bars.
    #[wasm_bindgen(getter)]
    pub fn change_percent(&self) -> f64 {
        match (self.candles.first(), self.candles.last()) {
            (Some(first), Some(last)) => (last.close / first.open - 1.0) * 100.0,
            _ => f64::NAN,
        }
    }
}

/// Fetches the latest candles of `product` (for example `BTC-USD`) at `timeframe`: any length
/// the app takes, such as `1h`, `4h`, `22m`, `2D` or `1W`. Lengths Coinbase does not serve are
/// folded from finer candles.
#[wasm_bindgen]
pub async fn load_chart(product: String, timeframe: String) -> Result<Chart, JsValue> {
    let timeframe = Timeframe::parse(&timeframe).map_err(coinbase::CoinbaseError::from)?;
    let fetched = coinbase::fetch_timeframe(&product, timeframe).await?;
    Ok(Chart {
        view: View::new(fetched.candles.len()),
        live: Live::new(&fetched),
        candles: fetched.candles,
        granularity: (timeframe.nominal_ms() / 1_000) as u32,
        product: product.to_uppercase(),
    })
}

/// The same request as [`load_chart`], to merge into a chart already on screen.
#[wasm_bindgen]
pub async fn load_snapshot(product: String, timeframe: String) -> Result<Snapshot, JsValue> {
    let timeframe = Timeframe::parse(&timeframe).map_err(coinbase::CoinbaseError::from)?;
    Ok(Snapshot(
        coinbase::fetch_timeframe(&product, timeframe).await?,
    ))
}
