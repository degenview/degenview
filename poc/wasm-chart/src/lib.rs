//! A Coinbase candlestick chart, fetched and drawn by Rust running as WebAssembly.
//!
//! JavaScript only wires the page: it calls [`load_chart`] to fetch candles and
//! [`Chart::draw`] to paint them on a `<canvas>`.

mod coinbase;
mod render;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use degenview_market::Kline;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Candles for one market and timeframe, ready to paint.
#[wasm_bindgen]
pub struct Chart {
    candles: Vec<Kline>,
    granularity: u32,
}

#[wasm_bindgen]
impl Chart {
    /// Paints the chart over the whole canvas, sharp on high-DPI screens. Call it again after the
    /// canvas is resized.
    pub fn draw(&self, canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
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
        render::draw(&ctx, size, &self.candles, self.granularity)
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

/// Fetches the latest candles of `product` (for example `BTC-USD`). `granularity` is a bar's
/// length in seconds: 60, 300, 900, 3600, 21600 or 86400.
#[wasm_bindgen]
pub async fn load_chart(product: String, granularity: u32) -> Result<Chart, JsValue> {
    let candles = coinbase::fetch_candles(&product, granularity).await?;
    Ok(Chart {
        candles,
        granularity,
    })
}
