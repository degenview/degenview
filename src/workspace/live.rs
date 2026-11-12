//! The live Binance feed: keeping it on the workspace's charts and folding its candles in.

use super::ChartWorkspace;
use crate::model::{DataSource, Timeframe};
use crate::service::market::live::BinanceLiveFeed;
use eframe::egui;

impl ChartWorkspace {
    pub(super) fn update_live_feed(&mut self, ctx: &egui::Context) {
        let mut symbols: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market() && chart.source == DataSource::Binance)
            .map(|chart| chart.symbol.to_uppercase())
            .collect();
        // The markets compared on a chart stream too, once however many overlays name them.
        for symbol in self.overlay_hub.binance_symbols() {
            if !symbols.contains(&symbol) {
                symbols.push(symbol);
            }
        }
        let interval = self.timeframe.interval().to_owned();
        // Binance streams no quarterly or yearly klines; those fold monthly candles.
        let monthly_fold = matches!(self.timeframe, Timeframe::Quarter | Timeframe::Year);
        let signature = (symbols, interval);

        let should_connect = !monthly_fold && !signature.0.is_empty();
        let needs_change = match &self.live_feed {
            Some(feed) => feed.signature() != &signature || !should_connect,
            None => should_connect,
        };
        if needs_change {
            self.live_feed = None;
            if should_connect {
                self.live_feed = Some(BinanceLiveFeed::connect(
                    signature.0.clone(),
                    signature.1.clone(),
                    ctx.clone(),
                ));
            }
        }

        let updates = self
            .live_feed
            .as_ref()
            .map(|feed| feed.inbox.drain())
            .unwrap_or_default();
        for (symbol, kline) in updates {
            self.overlay_hub.apply_live(&symbol, &kline);
            self.apply_live_kline(&symbol, kline);
        }
    }

    fn apply_live_kline(&mut self, symbol: &str, kline: crate::model::Kline) {
        let Some(chart) = self.charts.iter_mut().find(|chart| {
            chart.source == DataSource::Binance && chart.symbol.eq_ignore_ascii_case(symbol)
        }) else {
            return;
        };
        let replaces_last = chart
            .candles
            .last()
            .is_some_and(|last| last.open_time == kline.open_time);
        if replaces_last {
            if let Some(last) = chart.candles.last_mut() {
                *last = kline;
            }
        } else {
            chart.candles.push(kline);
        }
        chart.last_live_open_time = chart.candles.last().map(|candle| candle.open_time);
        chart.last_data_ms = Some(crate::model::watchlist::now_ms());
    }
}
