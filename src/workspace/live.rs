//! The live Binance feed: keeping it on the workspace's charts and folding its candles in.

use super::ChartWorkspace;
use crate::model::{DataSource, Timeframe};
use crate::service::live::BinanceLiveFeed;
use eframe::egui;

impl ChartWorkspace {
    pub(super) fn update_live_feed(&mut self, ctx: &egui::Context) {
        let symbols: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market() && chart.source == DataSource::Binance)
            .map(|chart| chart.symbol.to_uppercase())
            .collect();
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

        let mut updates = Vec::new();
        if let Some(feed) = &self.live_feed {
            while let Ok((symbol, kline)) = feed.receiver.try_recv() {
                updates.push((symbol, kline));
            }
        }
        for (symbol, kline) in updates {
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
    }
}
