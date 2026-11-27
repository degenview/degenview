//! Keeping the charts' candles current: fetches, and the live candle feeds, one per streamed
//! source on the workspace, folding its candles into the charts.

use crate::model::market::live::LiveMark;
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::live::LiveFeed;
use crate::workspace::ChartWorkspace;
use eframe::egui;
use std::time::Instant;

impl ChartWorkspace {
    /// Brings the charts' data up to date: takes finished fetches, starts the ones due and reads
    /// the live feeds. Needs no window, so it serves a tab that is drawn and one that is only
    /// watched (a script alert on a tab that is not on screen).
    pub fn update_market_data(&mut self, ctx: &egui::Context) {
        let timeframe = self.timeframe;
        for chart in &mut self.charts {
            chart.receive_fetch(timeframe);
            chart.receive_older(timeframe);
            chart.request_older(timeframe, ctx);
            if chart.refresh_due() {
                chart.fetch(timeframe, ctx);
            }
        }
        self.tick_link_probation(ctx);
        self.tick_overlays(ctx);
        self.update_live_feeds(ctx);
    }

    fn update_live_feeds(&mut self, ctx: &egui::Context) {
        for listing in Listing::candle_streams() {
            self.update_live_feed(listing, ctx);
        }
    }

    fn update_live_feed(&mut self, listing: Listing, ctx: &egui::Context) {
        let source = listing.source;
        let mut symbols: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market() && chart.source == source)
            .map(|chart| InstrumentId::new(source, chart.symbol.as_str()))
            .filter(|instrument| instrument.listing() == listing)
            .map(|instrument| instrument.canonical_symbol())
            .collect();
        // The markets compared on a chart stream too, once however many overlays name them.
        for symbol in self.overlay_hub.streamed_symbols(listing) {
            if !symbols.contains(&symbol) {
                symbols.push(symbol);
            }
        }
        // The stream is of the candle length the source serves the timeframe from (one-minute
        // klines for a 22-minute chart, monthly ones for a quarter); a timeframe it cannot reach
        // has none.
        let base = listing
            .plan(self.timeframe)
            .map(|plan| plan.base())
            .filter(|_| !symbols.is_empty());
        let wanted = base.map(|base| (symbols, base));
        let changed = match (self.live_feeds.get(&listing), &wanted) {
            (Some(feed), Some(wanted)) => feed.signature() != wanted,
            (None, None) => false,
            _ => true,
        };
        if changed {
            let feed = wanted
                .and_then(|(symbols, base)| LiveFeed::connect(listing, symbols, base, ctx.clone()));
            match feed {
                Some(feed) => {
                    self.live_feeds.insert(listing, feed);
                }
                None => {
                    self.live_feeds.remove(&listing);
                }
            }
        }

        let updates = self
            .live_feeds
            .get(&listing)
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
    /// shows. A chart's symbol may be spelled any way the source takes (`BTC`, `btc/usdt`): the
    /// feed names the market as the source does, so they are compared by `canonical_symbol`.
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
                    .canonical_symbol()
                    .eq_ignore_ascii_case(symbol)
        }) {
            if let Some(open_time) =
                chart
                    .live_fold
                    .apply(timeframe, base, &mut chart.candles, kline.clone())
            {
                chart.live = Some(LiveMark::new(open_time, now));
                chart.note_arrival(timeframe, crate::clock::now_ms());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::chart::Chart;
    use crate::workspace::test_support::{candle, empty_workspace};

    /// A tab that is not on screen is never drawn; its data must arrive all the same.
    #[test]
    fn a_fetch_reaches_a_chart_that_is_never_drawn() {
        let mut workspace = empty_workspace(1);
        // A source without a live stream or older pages, so nothing here reaches the network.
        let mut chart = Chart::blank(1, "pair", DataSource::DexScreener);
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(Ok(crate::workspace::chart::Fetched {
                candles: vec![candle(0, 5.0)],
                bars: Vec::new(),
                series: Vec::new(),
            }))
            .expect("send");
        chart.inbox = Some(receiver);
        workspace.charts.push(chart);

        workspace.update_market_data(&egui::Context::default());

        assert_eq!(workspace.charts[0].candles.len(), 1);
    }

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

    /// A 1-minute kline of the bar that holds the clock now.
    fn minute_bar_now() -> Kline {
        let now = crate::clock::now_ms();
        let open = crate::model::market::bucket::bucket_start_ms(Timeframe::MINUTE_1, now);
        let mut bar = candle(open, 1.0);
        bar.close_time = open + 60_000 - 1;
        bar
    }

    #[test]
    fn a_live_bar_alone_gives_a_chart_its_countdown_while_the_history_is_still_coming() {
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::MINUTE_1;
        workspace
            .charts
            .push(Chart::blank(1, "BTCUSDT", DataSource::Binance));
        let now = crate::clock::now_ms();
        assert!(
            workspace.charts[0]
                .bar_countdown(Timeframe::MINUTE_1, false, now)
                .is_none(),
            "nothing has arrived"
        );

        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            minute_bar_now(),
            Instant::now(),
        );
        assert!(
            workspace.charts[0]
                .bar_countdown(Timeframe::MINUTE_1, false, crate::clock::now_ms())
                .is_some(),
            "the socket's bar is a bar of this timeframe, REST or no REST"
        );
    }

    #[test]
    fn a_live_bar_of_a_new_timeframe_counts_down_where_old_candles_do_not() {
        let mut workspace = empty_workspace(1);
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        // The chart was fetched on 5 minutes; the workspace has moved on to 1 minute.
        chart.candles = vec![candle(1, 1.0)];
        chart.note_arrival(Timeframe::MINUTE_5, crate::clock::now_ms());
        workspace.charts.push(chart);
        workspace.timeframe = Timeframe::MINUTE_1;
        let at = |workspace: &ChartWorkspace| {
            workspace.charts[0].bar_countdown(Timeframe::MINUTE_1, false, crate::clock::now_ms())
        };
        assert!(
            at(&workspace).is_none(),
            "the old candles are not a bar of it"
        );

        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            minute_bar_now(),
            Instant::now(),
        );
        assert!(at(&workspace).is_some());
    }

    #[test]
    fn a_live_bar_the_chart_drops_records_no_arrival() {
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::MINUTE_1;
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = vec![candle(1_000_000_000_000, 1.0)];
        workspace.charts.push(chart);
        // Older than the newest candle: dropped.
        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            candle(1, 1.0),
            Instant::now(),
        );
        assert_eq!(workspace.charts[0].last_data_ms, None);
        assert_eq!(workspace.charts[0].data_timeframe, None);
    }

    #[test]
    fn a_kraken_chart_takes_the_live_bar_whatever_way_its_symbol_is_spelled() {
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
    fn an_okx_chart_takes_the_live_bar_whatever_way_its_symbol_is_spelled() {
        let mut workspace = empty_workspace(1);
        for (id, symbol) in [(1, "btc/usdt"), (2, "BTC-USDT"), (3, "BTC")] {
            let mut chart = Chart::blank(id, symbol, DataSource::Okx);
            chart.candles = vec![candle(1, 1.0)];
            workspace.charts.push(chart);
        }
        let mut other = Chart::blank(4, "ETH-USDT", DataSource::Okx);
        other.candles = vec![candle(1, 1.0)];
        workspace.charts.push(other);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Okx,
            base,
            "BTC-USDT",
            candle(2, 5.0),
            Instant::now(),
        );

        let lens: Vec<usize> = workspace.charts.iter().map(|c| c.candles.len()).collect();
        assert_eq!(lens, [2, 2, 2, 1]);
    }

    #[test]
    fn a_bybit_chart_takes_the_live_bar_whatever_way_its_symbol_is_spelled() {
        let mut workspace = empty_workspace(1);
        for (id, symbol) in [(1, "btc/usdt"), (2, "BTCUSDT"), (3, "BTC")] {
            let mut chart = Chart::blank(id, symbol, DataSource::Bybit);
            chart.candles = vec![candle(1, 1.0)];
            workspace.charts.push(chart);
        }
        let mut other = Chart::blank(4, "ETHUSDT", DataSource::Bybit);
        other.candles = vec![candle(1, 1.0)];
        workspace.charts.push(other);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Bybit,
            base,
            "BTCUSDT",
            candle(2, 5.0),
            Instant::now(),
        );

        let lens: Vec<usize> = workspace.charts.iter().map(|c| c.candles.len()).collect();
        assert_eq!(lens, [2, 2, 2, 1]);
    }

    #[test]
    fn a_gate_chart_takes_the_live_bar_whatever_way_its_symbol_is_spelled() {
        let mut workspace = empty_workspace(1);
        for (id, symbol) in [(1, "btc/usdt"), (2, "BTC_USDT"), (3, "BTC")] {
            let mut chart = Chart::blank(id, symbol, DataSource::Gate);
            chart.candles = vec![candle(1, 1.0)];
            workspace.charts.push(chart);
        }
        let mut other = Chart::blank(4, "ETH_USDT", DataSource::Gate);
        other.candles = vec![candle(1, 1.0)];
        workspace.charts.push(other);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Gate,
            base,
            "BTC_USDT",
            candle(2, 5.0),
            Instant::now(),
        );

        let lens: Vec<usize> = workspace.charts.iter().map(|c| c.candles.len()).collect();
        assert_eq!(lens, [2, 2, 2, 1]);
    }

    #[test]
    fn a_mexc_chart_takes_the_live_bar_whatever_way_its_symbol_is_spelled() {
        let mut workspace = empty_workspace(1);
        for (id, symbol) in [(1, "btc/usdt"), (2, "BTCUSDT"), (3, "BTC")] {
            let mut chart = Chart::blank(id, symbol, DataSource::Mexc);
            chart.candles = vec![candle(1, 1.0)];
            workspace.charts.push(chart);
        }
        let mut other = Chart::blank(4, "ETHUSDT", DataSource::Mexc);
        other.candles = vec![candle(1, 1.0)];
        workspace.charts.push(other);

        let base = workspace.timeframe;
        workspace.apply_live_kline(
            DataSource::Mexc,
            base,
            "BTCUSDT",
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
                bars: Vec::new(),
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch(workspace.timeframe);

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

    #[test]
    fn an_answer_with_the_buckets_base_bars_gives_the_forming_candle_what_the_socket_missed() {
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::parse("5m").unwrap();
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        // The socket was first: it saw only the bar at minute 3, 8 traded.
        chart.last_fetch = Instant::now();
        workspace.charts.push(chart);
        let mut kline = candle(3 * 60_000, 5.0);
        kline.volume = 8.0;
        workspace.apply_live_kline(
            DataSource::Binance,
            Timeframe::MINUTE_1,
            "BTCUSDT",
            kline,
            Instant::now(),
        );

        // REST answers later with the bucket's first four minutes, the last one cut at 3.
        let bars: Vec<Kline> = (0..4)
            .map(|minute| {
                let mut bar = candle(minute * 60_000, 4.0);
                bar.volume = if minute == 3 { 3.0 } else { 2.0 };
                bar
            })
            .collect();
        let chart = &mut workspace.charts[0];
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(Ok(crate::workspace::chart::Fetched {
                candles: vec![Kline {
                    volume: 9.0,
                    close_time: 5 * 60_000 - 1,
                    ..candle(0, 4.0)
                }],
                bars,
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.live = Some(LiveMark::new(0, Instant::now()));
        chart.receive_fetch(workspace.timeframe);

        assert_eq!(workspace.charts[0].candles.len(), 1);
        assert_eq!(
            workspace.charts[0].candles[0].volume, 14.0,
            "three minutes of 2 and the socket's 8"
        );
    }
}
