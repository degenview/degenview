//! The Trade Journal on this workspace's charts: each market chart is handed the journal's trades
//! for its market, and a click on one is passed up as a request to open it.
//!
//! The workspace never reads the journal; the app gives it the marks (the account the journal is
//! showing), and the chart paints them (`ui::chart_card::journal_marks`).

use eframe::egui;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{ChartWorkspace, WorkspaceAction, displayed_candles};
use crate::model::InstrumentId;
use crate::model::journal::marks::{Marks, timeframe_for};
use crate::ui::Scrolled;

/// How long a chart is given to find the candles a trade needs before the view settles for what
/// it has.
const PENDING_LIMIT: Duration = Duration::from_secs(30);

/// A trade the user asked to see on a chart, waiting for the candles to be there.
pub(super) struct PendingRange {
    pub chart_id: u64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub started: Instant,
}

impl ChartWorkspace {
    /// Whether any chart here draws journal trades. The app reads the journal only when this is
    /// true.
    pub fn wants_journal_marks(&self) -> bool {
        self.charts
            .iter()
            .any(|chart| chart.is_market() && chart.chart.appearance().canvas.show_journal_trades)
    }

    /// The journal's marks, handed in by the app each frame (a cheap pointer copy).
    pub fn set_journal_marks(&mut self, marks: Option<Arc<Marks>>) {
        self.journal_marks = marks;
    }

    /// Whether a chart here is of `instrument`'s market.
    pub fn has_market(&self, instrument: &InstrumentId) -> bool {
        self.charts.iter().any(|chart| {
            chart.is_market()
                && InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument)
        })
    }

    /// Takes the chart of `instrument` (added if the workspace has none) to the trade held from
    /// `from_ms` to `to_ms`: the timeframe is changed when this one cannot show the hold, older
    /// candles are paged in until `from_ms` is reached, then the view frames the trade. If the
    /// provider cannot reach back that far the view shows what there is and says so.
    pub fn show_trade_range(
        &mut self,
        instrument: &InstrumentId,
        from_ms: i64,
        to_ms: i64,
        ctx: &egui::Context,
    ) {
        let chart_id = self.add_chart_for(instrument, ctx).chart_id;
        self.focus_chart(chart_id);
        let wanted = timeframe_for(to_ms - from_ms);
        let span_bars = (to_ms - from_ms) / self.timeframe.nominal_ms();
        // Keep the user's timeframe when it frames the trade in a sensible number of bars.
        if !(2..=150).contains(&span_bars) && self.timeframe != wanted {
            self.reload_for_timeframe(wanted, ctx);
            self.say(
                format!("Timeframe set to {} to frame the trade.", wanted.label()),
                false,
            );
        }
        self.pending_range = Some(PendingRange {
            chart_id,
            from_ms,
            to_ms,
            started: Instant::now(),
        });
    }

    /// Advances a pending "view on chart": waits for candles, pages older ones in, then frames.
    pub(super) fn tick_pending_range(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_range else {
            return;
        };
        let (chart_id, from_ms, to_ms, started) = (
            pending.chart_id,
            pending.from_ms,
            pending.to_ms,
            pending.started,
        );
        let Some(index) = self.chart_index(chart_id) else {
            self.pending_range = None;
            return;
        };
        let replay_ms = self.replay_cut();
        let timeframe = self.timeframe;
        let chart = &mut self.charts[index];
        let waited = started.elapsed();
        let candles = displayed_candles(&chart.candles, replay_ms);
        if candles.is_empty() {
            if waited > PENDING_LIMIT || chart.error.is_some() {
                self.pending_range = None;
                self.say("The chart has no candles to show this trade on.", true);
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            return;
        }
        match chart.chart.show_time_range(from_ms, to_ms, candles) {
            Scrolled::Done => self.pending_range = None,
            Scrolled::NoBars => self.pending_range = None,
            Scrolled::BeforeHistory => {
                let can_page = chart.older.can_progress(
                    chart.source.supports_older_history(timeframe),
                    candles.len(),
                );
                if can_page && waited < PENDING_LIMIT {
                    // The view sits at the oldest candle, which is what makes the chart ask for
                    // the page before it; look again when it arrives.
                    ctx.request_repaint_after(Duration::from_millis(200));
                } else {
                    self.pending_range = None;
                    let name = chart.source.display_name();
                    self.say(
                        format!(
                            "{name} has no older candles, so the view shows the oldest it has \
                             and the start of this trade is off the left edge."
                        ),
                        false,
                    );
                }
            }
        }
    }

    pub(super) fn configure_journal_cards(&mut self) {
        let marks = self.journal_marks.clone();
        for chart in &mut self.charts {
            let market = chart
                .is_market()
                .then(|| InstrumentId::new(chart.source, &chart.symbol).key());
            let mine = match (&marks, market) {
                (Some(marks), Some(key)) => marks.market(&key),
                _ => None,
            };
            // Only replace when it changed, so an unchanged chart is left alone.
            let same = match (&chart.chart.journal.marks, &mine) {
                (Some(old), Some(new)) => Arc::ptr_eq(old, new),
                (None, None) => true,
                _ => false,
            };
            if !same {
                chart.chart.journal.marks = mine;
            }
        }
    }

    /// The trades clicked on a chart since the last frame, as requests to open them.
    pub(super) fn apply_journal_opens(&mut self) -> Vec<WorkspaceAction> {
        self.charts
            .iter_mut()
            .filter_map(|chart| chart.chart.journal.open.take())
            .map(WorkspaceAction::OpenJournalTrade)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{chart_with_drawings, empty_workspace};
    use super::*;
    use crate::model::journal::filters::OutcomeFilter;
    use crate::model::journal::marks::TradeMark;
    use crate::model::journal::model::{Direction, Trade, TradeStatus};
    use crate::model::journal::testing::{d, instrument};
    use rust_decimal::Decimal;

    fn trade(symbol: &str) -> Trade {
        let mut market = instrument(symbol);
        market.market_key = InstrumentId::new(crate::model::DataSource::Binance, symbol).key();
        Trade {
            key: format!("k-{symbol}"),
            account_uid: "a".into(),
            instrument: market,
            direction: Direction::Long,
            status: TradeStatus::Open,
            open_ts_ms: 1,
            close_ts_ms: None,
            legs: vec![crate::model::journal::model::TradeLeg {
                exec_uid: "e".into(),
                ts_ms: 1,
                side: crate::model::journal::model::Side::Buy,
                quantity: d("1"),
                price: d("100"),
                fee: Some(Decimal::ZERO),
                is_entry: true,
                realized_gross: Decimal::ZERO,
                position_after: d("1"),
            }],
            entry_quantity: d("1"),
            exit_quantity: Decimal::ZERO,
            open_quantity: d("1"),
            max_quantity: d("1"),
            avg_entry: d("100"),
            avg_exit: None,
            multiplier: Decimal::ONE,
            gross_pnl: Decimal::ZERO,
            fees: Decimal::ZERO,
            fees_known: true,
            open_fees: Decimal::ZERO,
            currency: Some("USD".into()),
            initial_stop: None,
        }
    }

    #[test]
    fn each_chart_gets_only_its_own_markets_trades() {
        let mut workspace = empty_workspace(1);
        let btc = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let eth = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
        assert!(workspace.wants_journal_marks());
        let marks = Arc::new(Marks::build(&[trade("BTCUSDT")], None, Decimal::ZERO));
        workspace.set_journal_marks(Some(marks.clone()));
        workspace.configure_journal_cards();
        let of = |workspace: &ChartWorkspace, id: u64| -> Option<Vec<String>> {
            workspace
                .charts
                .iter()
                .find(|chart| chart.id == id)?
                .chart
                .journal
                .marks
                .as_ref()
                .map(|marks| {
                    marks
                        .iter()
                        .map(|mark: &TradeMark| mark.key.clone())
                        .collect()
                })
        };
        assert_eq!(of(&workspace, btc), Some(vec!["k-BTCUSDT".to_owned()]));
        assert_eq!(of(&workspace, eth), None);
        assert_eq!(
            workspace.charts[0]
                .chart
                .journal
                .marks
                .as_ref()
                .map(|m| m[0].outcome),
            Some(OutcomeFilter::Open)
        );
        // Taking the journal away clears the charts.
        workspace.set_journal_marks(None);
        workspace.configure_journal_cards();
        assert_eq!(of(&workspace, btc), None);
    }

    #[test]
    fn a_clicked_trade_becomes_a_request_to_open_it_and_only_once() {
        let mut workspace = empty_workspace(1);
        chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.charts[0].chart.journal.open = Some("k-BTCUSDT".into());
        let actions = workspace.apply_journal_opens();
        assert!(matches!(
            actions.as_slice(),
            [WorkspaceAction::OpenJournalTrade(key)] if key == "k-BTCUSDT"
        ));
        assert!(workspace.apply_journal_opens().is_empty());
    }

    #[test]
    fn a_workspace_with_every_chart_switched_off_does_not_ask_for_the_journal() {
        let mut workspace = empty_workspace(1);
        chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.charts[0]
            .chart
            .appearance_mut()
            .canvas
            .show_journal_trades = false;
        assert!(!workspace.wants_journal_marks());
    }

    const HOUR: i64 = 3_600_000;

    fn hourly(workspace: &mut ChartWorkspace, chart_id: u64, from_hour: i64, to_hour: i64) {
        use super::super::test_support::candle;
        let chart = workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .unwrap();
        chart.candles = (from_hour..to_hour)
            .map(|hour| candle(hour * HOUR, 100.0 + hour as f64))
            .collect();
    }

    #[test]
    fn a_trade_is_framed_once_the_candles_reach_it() {
        let mut workspace = empty_workspace(1);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.timeframe = crate::model::Timeframe::Hour;
        hourly(&mut workspace, chart, 0, 400);
        let ctx = egui::Context::default();
        let market = InstrumentId::new(crate::model::DataSource::Binance, "BTCUSDT");
        assert!(workspace.has_market(&market));
        workspace.show_trade_range(&market, 150 * HOUR, 160 * HOUR, &ctx);
        assert!(workspace.pending_range.is_some());
        workspace.tick_pending_range(&ctx);
        assert!(workspace.pending_range.is_none(), "framed at once");
        let card = &workspace.charts[0].chart;
        // 400 bars, the trade around bar 155: scrolled back about 244 bars from the newest.
        let back = card.scrolled_back();
        assert!(
            (back - (399.0 - 155.0 - card.visible_bars() as f64 / 2.0)).abs() < 3.0,
            "{back}"
        );
        assert!(card.visible_bars() < 40, "zoomed in on a ten-bar trade");
    }

    #[test]
    fn a_trade_older_than_the_candles_waits_for_older_history_then_says_when_there_is_none() {
        let mut workspace = empty_workspace(1);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.timeframe = crate::model::Timeframe::Hour;
        hourly(&mut workspace, chart, 100, 300);
        let ctx = egui::Context::default();
        let market = InstrumentId::new(crate::model::DataSource::Binance, "BTCUSDT");
        workspace.show_trade_range(&market, 20 * HOUR, 30 * HOUR, &ctx);
        workspace.tick_pending_range(&ctx);
        assert!(
            workspace.pending_range.is_some(),
            "still waiting for older candles"
        );
        // The provider has nothing older.
        workspace.charts[0].older.arrived(0);
        workspace.tick_pending_range(&ctx);
        assert!(workspace.pending_range.is_none());
        let notice = workspace.context.notice.as_ref().expect("a notice");
        assert!(notice.text.contains("no older candles"), "{}", notice.text);
    }

    #[test]
    fn a_trade_too_long_for_the_timeframe_changes_it() {
        let mut workspace = empty_workspace(1);
        chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.timeframe = crate::model::Timeframe::Hour;
        let ctx = egui::Context::default();
        let market = InstrumentId::new(crate::model::DataSource::Binance, "BTCUSDT");
        // Two hundred days on hourly bars is thousands of bars.
        workspace.show_trade_range(&market, 0, 200 * 24 * HOUR, &ctx);
        assert_eq!(workspace.timeframe, crate::model::Timeframe::Week);
        assert!(
            workspace
                .context
                .notice
                .as_ref()
                .is_some_and(|n| n.text.contains("1W"))
        );
    }
}
