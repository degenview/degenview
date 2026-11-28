//! The preview's chart: the highlighted market's last day. A market is only fetched once the
//! highlight has rested on it, so arrowing down the list asks the providers for nothing.

use crate::i18n::tr;
use crate::model::InstrumentId;
use crate::model::market::trend::Trend;
use crate::model::watchlist::quote::WatchlistQuote;
use crate::service::market::trend::{Fetch, TrendLoader};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::{self, REGULAR, sparkline};
use crate::ui::theme;
use eframe::egui::{self, Rect, Ui, Vec2};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the highlight rests on a market before its day is fetched.
const SETTLE: Duration = Duration::from_millis(200);

pub struct PreviewChart {
    loader: TrendLoader,
    /// The market under the highlight, read by queued fetches to drop the ones scrolled past.
    wanted: Arc<Mutex<String>>,
    key: String,
    since: Instant,
    /// The frame that last drew the chart, so a test knows whether it is still waited on.
    #[cfg(test)]
    shown_in: Option<u64>,
}

impl PreviewChart {
    pub fn new(fetch: Fetch) -> Self {
        Self {
            loader: TrendLoader::last_day(fetch),
            wanted: Arc::default(),
            key: String::new(),
            since: Instant::now(),
            #[cfg(test)]
            shown_in: None,
        }
    }

    /// The same chart for a freshly opened picker: what was read stays, what failed is asked
    /// again.
    pub fn reopened(&self) -> Self {
        self.loader.forget_unavailable();
        Self {
            loader: self.loader.clone(),
            wanted: Arc::clone(&self.wanted),
            key: String::new(),
            since: Instant::now(),
            #[cfg(test)]
            shown_in: None,
        }
    }

    /// Whether the last frame of `ctx` drew a chart whose day has not been read yet (the highlight
    /// is still resting, or the fetch is under way).
    #[cfg(test)]
    pub fn is_loading(&self, ctx: &egui::Context) -> bool {
        self.shown_in
            .is_some_and(|frame| frame + 1 == ctx.cumulative_frame_nr())
            && self.loader.get(&self.key).is_none()
    }

    /// Draws `market`'s last day in `rect`, ending at the price quoted beside it and coloured as
    /// its change is.
    pub fn show(
        &mut self,
        ui: &Ui,
        rect: Rect,
        market: &InstrumentId,
        quote: Option<&WatchlistQuote>,
    ) {
        #[cfg(test)]
        {
            self.shown_in = Some(ui.ctx().cumulative_frame_nr());
        }
        let key = market.key();
        if self.key != key {
            self.key.clone_from(&key);
            self.since = Instant::now();
            if let Ok(mut wanted) = self.wanted.lock() {
                wanted.clone_from(&key);
            }
        }
        let wait = SETTLE.saturating_sub(self.since.elapsed());
        if wait.is_zero() {
            let wanted = Arc::clone(&self.wanted);
            let asked = key.clone();
            let ctx = ui.ctx().clone();
            self.loader.request_if(
                market,
                move || wanted.lock().is_ok_and(|wanted| *wanted == asked),
                Arc::new(move || ctx.request_repaint()),
            );
        } else {
            ui.ctx().request_repaint_after(wait);
        }
        paint(ui, rect, self.loader.get(&key), quote);
    }
}

fn paint(ui: &Ui, rect: Rect, trend: Option<Trend>, quote: Option<&WatchlistQuote>) {
    match trend {
        Some(Trend::Ready(mut closes)) => {
            if let Some(last) = quote.and_then(|quote| quote.last) {
                closes.push(last as f32);
            }
            let rising = quote
                .and_then(|quote| quote.change_percent)
                .map_or_else(|| sparkline::rising(&closes), |percent| percent >= 0.0);
            let color = if rising { theme::UP } else { theme::DOWN };
            sparkline::paint_area(ui.painter(), rect, &closes, color);
        }
        Some(Trend::Unavailable) => {
            let galley = kit::galley(
                ui,
                tr!("No chart for this market"),
                13.0,
                REGULAR,
                theme::text_tertiary(),
                rect.width(),
            );
            ui.painter().galley(
                rect.center() - galley.size() / 2.0,
                galley,
                theme::text_tertiary(),
            );
        }
        None => {
            let size = rect.height().min(28.0);
            LoadingIndicator::new().size(size).paint_at(
                ui,
                egui::Rect::from_center_size(rect.center(), Vec2::splat(size)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, Kline, Timeframe};
    use crate::service::ProviderError;
    use crate::ui::render_tests::Screen;
    use eframe::egui::{pos2, vec2};

    static ASKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn fetch(
        _: DataSource,
        symbol: &str,
        _: Timeframe,
        _: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        ASKED.lock().expect("asked").push(symbol.to_owned());
        Err(ProviderError::other("no history"))
    }

    #[test]
    fn only_the_market_the_highlight_rests_on_is_fetched() {
        let mut chart = PreviewChart::new(fetch);
        let mut screen = Screen::new(vec2(320.0, 200.0));
        let ctx = screen.ctx.clone();
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(300.0, 100.0));
        let mut show = |chart: &mut PreviewChart, symbol: &str| {
            let market = InstrumentId::new(DataSource::Binance, symbol);
            screen.run(Vec::new(), |ui| chart.show(ui, rect, &market, None));
        };
        for symbol in ["AAAUSDT", "BBBUSDT", "CCCUSDT"] {
            show(&mut chart, symbol);
        }
        assert!(
            ASKED.lock().expect("asked").is_empty(),
            "passing over asks nothing"
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            show(&mut chart, "CCCUSDT");
            if !chart.is_loading(&ctx) {
                break;
            }
            assert!(Instant::now() < deadline, "the chart was never read");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*ASKED.lock().expect("asked"), ["CCCUSDT"]);
    }
}
