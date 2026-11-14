//! The candles: when to fetch and refresh them, taking a fetch's answer, and switching the
//! market or timeframe.

use super::{
    CANDLE_LIMIT, MARKET_KEY, PreviewMarket, PreviewState, RECENTS_KEY, RECENTS_LIMIT,
    TIMEFRAME_KEY,
};
use crate::format;
use crate::model::Timeframe;
use crate::service::work_queue;
use crate::service::{self, ProviderError};
use eframe::egui;
use std::time::Instant;

impl PreviewState {
    pub(super) fn want_fetch(&self) -> bool {
        let key = (self.market.clone(), self.timeframe);
        self.fetched_for.as_ref() != Some(&key) && self.inbox.is_none()
    }

    pub(super) fn fetch(&mut self, ctx: &egui::Context) {
        // Unit tests draw the pane headlessly and must not touch the network.
        if cfg!(test) {
            self.fetched_for = Some((self.market.clone(), self.timeframe));
            return;
        }
        let market = self.market.clone();
        let interval = self.timeframe.interval();
        self.inbox = Some(work_queue::start_for(market.source, Some(ctx), move || {
            service::fetch_klines(market.source, &market.ticker, interval, CANDLE_LIMIT)
        }));
        self.loading = true;
        self.last_fetch = Some(Instant::now());
        self.fetched_for = Some((self.market.clone(), self.timeframe));
    }

    pub(super) fn receive_candles(&mut self) {
        let Some(fetch) = &mut self.inbox else {
            return;
        };
        let Some(result) =
            fetch.poll_or(|| Err(ProviderError::other("The candles could not be loaded.")))
        else {
            return;
        };
        self.inbox = None;
        self.loading = false;
        match result {
            Ok(candles) => {
                self.set_candles(candles);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn select_market(&mut self, market: PreviewMarket) {
        if market != self.market {
            self.set_candles(Vec::new());
            self.show_newest_bars();
            self.market = market.clone();
            self.fetched_for = None;
            self.error = None;
        }
        self.recents.retain(|m| m != &market);
        self.recents.insert(0, market);
        self.recents.truncate(RECENTS_LIMIT);
        self.save(MARKET_KEY, &self.market);
        self.save(RECENTS_KEY, &self.recents);
    }

    pub(in crate::pine_studio) fn select_timeframe(&mut self, timeframe: Timeframe) {
        if timeframe != self.timeframe {
            self.timeframe = timeframe;
            self.set_candles(Vec::new());
            self.show_newest_bars();
            self.fetched_for = None;
            self.save(TIMEFRAME_KEY, &timeframe);
        }
    }

    /// Back to the newest bars with the price axis on autoscale, as the workspace does when a
    /// chart's market or timeframe changes. A price range placed by dragging is in the old
    /// market's prices and would leave the new one off screen. The candle count and price zoom
    /// are the person's saved choices, so they stay.
    fn show_newest_bars(&mut self) {
        let (candles, zoom) = (self.card.visible_candles(), self.card.y_zoom());
        self.card.reset_view();
        self.card.set_visible_candles(candles);
        self.card.set_y_zoom(zoom);
    }

    /// The last close and the change across the candles on screen. The warm-up candles off the
    /// left edge are not counted: they would report a move nobody can see.
    pub(super) fn price_line(&self) -> Option<(String, f64)> {
        let last = self.candles.last()?;
        let change = self.card.visible_change(&self.candles)?;
        Some((format::price(last.close), change))
    }
}
