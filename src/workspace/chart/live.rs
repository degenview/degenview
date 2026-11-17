//! The live candle feeds: one per streamed source on the workspace, folding its candles into the
//! charts.

use crate::model::market::live::LiveMark;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::market::live::LiveFeed;
use crate::workspace::ChartWorkspace;
use eframe::egui;
use std::time::Instant;

impl ChartWorkspace {
    pub(in crate::workspace) fn update_live_feeds(&mut self, ctx: &egui::Context) {
        for source in DataSource::CANDLE_STREAMS {
            self.update_live_feed(source, ctx);
        }
    }

    fn update_live_feed(&mut self, source: DataSource, ctx: &egui::Context) {
        let mut symbols: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market() && chart.source == source)
            .map(|chart| InstrumentId::new(source, chart.symbol.as_str()).api_symbol())
            .collect();
        // The markets compared on a chart stream too, once however many overlays name them.
        for symbol in self.overlay_hub.streamed_symbols(source) {
            if !symbols.contains(&symbol) {
                symbols.push(symbol);
            }
        }
        // The stream is of the candle length the source serves the timeframe from (one-minute
        // klines for a 22-minute chart, monthly ones for a quarter); a timeframe it cannot reach
        // has none.
        let base = source
            .plan(self.timeframe)
            .map(|plan| plan.base())
            .filter(|_| !symbols.is_empty());
        let wanted = base.map(|base| (symbols, base));
        let changed = match (self.live_feeds.get(&source), &wanted) {
            (Some(feed), Some(wanted)) => feed.signature() != wanted,
            (None, None) => false,
            _ => true,
        };
        if changed {
            let feed = wanted
                .and_then(|(symbols, base)| LiveFeed::connect(source, symbols, base, ctx.clone()));
            match feed {
                Some(feed) => {
                    self.live_feeds.insert(source, feed);
                }
                None => {
                    self.live_feeds.remove(&source);
                }
            }
        }

        let updates = self
            .live_feeds
            .get(&source)
            .map(|feed| feed.inbox.drain())
            .unwrap_or_default();
        let Some(base) = base else {
            return;
        };
        let now = Instant::now();
        for (symbol, kline) in updates {
            self.overlay_hub
                .apply_live(source, self.timeframe, base, &symbol, &kline, now);
            self.apply_live_kline(source, base, &symbol, kline, now);
        }
    }

    /// Every chart of `symbol` on `source` takes the kline of `base` candles, whatever else it
    /// shows. A chart's symbol may be spelled any way the source takes (`BTC`, `XBT/USD`), the
    /// feed names the market as the source does.
    fn apply_live_kline(
        &mut self,
        source: DataSource,
        base: Timeframe,
        symbol: &str,
        kline: Kline,
        now: Instant,
    ) {
        let timeframe = self.timeframe;
        for chart in self.charts.iter_mut().filter(|chart| {
            chart.source == source
                && InstrumentId::new(source, chart.symbol.as_str())
                    .api_symbol()
                    .eq_ignore_ascii_case(symbol)
        }) {
            if let Some(open_time) =
                chart
                    .live_fold
                    .apply(timeframe, base, &mut chart.candles, kline.clone())
            {
                chart.live = Some(LiveMark::new(open_time, now));
                chart.last_data_ms = Some(crate::model::watchlist::now_ms());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::chart::Chart;
    use crate::workspace::test_support::{candle, empty_workspace};

    #[test]
    fn every_chart_of_the_symbol_takes_the_live_bar() {
        let mut workspace = empty_workspace(1);
        for id in 1..=2 {
            let mut chart = Chart::blank(id, "BTCUSDT", DataSource::Binance);
            chart.candles = vec![candle(1, 1.0)];
            workspace.charts.push(chart);
        }
        let mut other = Chart::blank(3, "ETHUSDT", DataSource::Binance);
        other.candles = vec![candle(1, 1.0)];
        workspace.charts.push(other);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Binance,
            base,
            "btcusdt",
            candle(2, 5.0),
            Instant::now(),
        );

        let lens: Vec<usize> = workspace.charts.iter().map(|c| c.candles.len()).collect();
        assert_eq!(lens, [2, 2, 1]);
        assert!(workspace.charts[0].live.is_some() && workspace.charts[1].live.is_some());
    }

    #[test]
    fn a_chart_takes_the_live_bar_whatever_way_its_symbol_is_spelled() {
        let mut workspace = empty_workspace(1);
        for (id, symbol) in [(1, "xbt/usd"), (2, "BTC-USD"), (3, "BTC")] {
            let mut chart = Chart::blank(id, symbol, DataSource::Kraken);
            chart.candles = vec![candle(1, 1.0)];
            workspace.charts.push(chart);
        }
        let mut other = Chart::blank(4, "ETH/USD", DataSource::Kraken);
        other.candles = vec![candle(1, 1.0)];
        workspace.charts.push(other);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Kraken,
            base,
            "BTC/USD",
            candle(2, 5.0),
            Instant::now(),
        );

        let lens: Vec<usize> = workspace.charts.iter().map(|c| c.candles.len()).collect();
        assert_eq!(lens, [2, 2, 2, 1]);
    }

    #[test]
    fn a_late_tick_of_an_older_bar_does_not_unsort_the_chart() {
        let mut workspace = empty_workspace(1);
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = vec![candle(1, 1.0), candle(2, 2.0)];
        workspace.charts.push(chart);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Binance,
            base,
            "BTCUSDT",
            candle(1, 9.0),
            Instant::now(),
        );

        assert_eq!(
            workspace.charts[0].candles,
            [candle(1, 1.0), candle(2, 2.0)]
        );
        assert!(workspace.charts[0].live.is_none());
    }

    #[test]
    fn every_chart_of_the_symbol_folds_the_base_klines_into_its_own_bucket() {
        let minute = 60_000;
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::parse("5m").unwrap();
        for id in 1..=2 {
            workspace
                .charts
                .push(Chart::blank(id, "BTCUSDT", DataSource::Binance));
        }
        let live = |open: i64, volume: f64| Kline {
            volume,
            ..candle(open, 5.0)
        };

        for kline in [live(0, 1.0), live(0, 2.0), live(minute, 3.0)] {
            workspace.apply_live_kline(
                DataSource::Binance,
                Timeframe::MINUTE_1,
                "BTCUSDT",
                kline,
                Instant::now(),
            );
        }

        for chart in &workspace.charts {
            assert_eq!(chart.candles.len(), 1);
            assert_eq!(
                chart.candles[0].volume, 5.0,
                "each bar once, each chart its own"
            );
            assert_eq!(
                chart.live.map(|mark| mark.guard_for(Instant::now())),
                Some(None)
            );
        }
    }

    #[test]
    fn a_refresh_that_replaces_the_forming_bucket_makes_the_next_live_kline_start_from_it() {
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::parse("5m").unwrap();
        workspace
            .charts
            .push(Chart::blank(1, "BTCUSDT", DataSource::Binance));
        let mut kline = candle(0, 5.0);
        kline.volume = 4.0;
        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            kline.clone(),
            Instant::now(),
        );

        // A refresh with no live guard replaces the candle: the socket's account is dropped.
        let chart = &mut workspace.charts[0];
        chart.live = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        let fetched = Kline {
            volume: 10.0,
            close_time: 5 * 60_000 - 1,
            ..candle(0, 5.0)
        };
        sender
            .send(Ok(crate::workspace::chart::Fetched {
                candles: vec![fetched],
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch();

        // The bar already counted in the refresh's volume adds only what it trades from here.
        kline.volume = 4.5;
        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            kline.clone(),
            Instant::now(),
        );
        assert_eq!(workspace.charts[0].candles[0].volume, 10.0);
        kline.volume = 5.0;
        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            kline,
            Instant::now(),
        );
        assert_eq!(workspace.charts[0].candles[0].volume, 10.5);
    }
}
