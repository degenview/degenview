//! The drawings a chart keeps for each market it has shown.
//!
//! `Chart::drawings` is the set of the market on screen, so everything that draws, selects,
//! edits or lists drawings sees that market's alone. The sets of the markets the chart showed
//! before wait in `parked_drawings` and come back when it shows them again.

use super::Chart;
use crate::model::InstrumentId;
use crate::model::drawings::market::MarketDrawings;
use crate::workspace::drawing::is_ruler;

impl Chart {
    /// What this chart's drawings are filed under: the market it shows now.
    pub(in crate::workspace) fn drawing_market(&self) -> String {
        InstrumentId::new(self.source, &self.symbol).key()
    }

    /// Everything drawn on the chart, market by market, rulers left out: what is saved, and what
    /// a layout is compared with.
    pub(in crate::workspace) fn stored_drawings(&self) -> MarketDrawings {
        let mut all = self.parked_drawings.clone();
        all.put(
            &self.drawing_market(),
            self.drawings
                .iter()
                .filter(|drawing| !is_ruler(drawing))
                .cloned()
                .collect(),
        );
        all
    }

    /// Puts `stored` on the chart: the set of the market it shows becomes the drawings, the rest
    /// wait. A ruler on screen stays, and ids already used stay taken.
    pub(in crate::workspace) fn set_stored_drawings(&mut self, mut stored: MarketDrawings) {
        if let Some(highest) = stored.highest_id() {
            self.next_drawing_id = self.next_drawing_id.max(highest + 1);
        }
        let rulers: Vec<_> = self
            .drawings
            .iter()
            .filter(|drawing| is_ruler(drawing))
            .cloned()
            .collect();
        self.drawings = stored.take(&self.drawing_market());
        self.drawings.extend(rulers);
        self.parked_drawings = stored;
    }

    /// The chart shows another market than `from`, the key it showed: keeps what is drawn under
    /// that key and brings back what was drawn on the market it shows now. A ruler measures the
    /// market it was drawn on, so it goes.
    pub(super) fn file_drawings(&mut self, from: &str) {
        let now = self.drawing_market();
        if now == from {
            return;
        }
        self.drawings.retain(|drawing| !is_ruler(drawing));
        let shown = std::mem::replace(&mut self.drawings, self.parked_drawings.take(&now));
        self.parked_drawings.put(from, shown);
    }
}

#[cfg(test)]
mod tests {
    use crate::model::watchlist::WatchlistInstrument;
    use crate::model::{DataSource, Drawing};
    use crate::workspace::test_support::{
        chart_with_drawings, drawing_ids, empty_workspace, rectangle,
    };
    use crate::workspace::{Chart, ChartWorkspace};
    use eframe::egui;

    fn show(workspace: &mut ChartWorkspace, chart: u64, source: DataSource, symbol: &str) {
        let item = WatchlistInstrument::from_market(source, symbol, symbol);
        workspace.switch_chart(chart, &item, &egui::Context::default());
    }

    fn draw(workspace: &mut ChartWorkspace, chart: u64, id: u64) {
        let chart = workspace.charts.iter_mut().find(|c| c.id == chart).unwrap();
        chart.drawings.push(rectangle(id, id as i64 * 100));
        chart.next_drawing_id = chart.next_drawing_id.max(id + 1);
    }

    #[test]
    fn a_chart_keeps_a_set_of_drawings_for_each_market_it_shows() {
        let mut workspace = empty_workspace(1);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);

        show(&mut workspace, chart, DataSource::Binance, "ETHUSDT");
        assert!(
            drawing_ids(&workspace, chart).is_empty(),
            "nothing drawn here yet"
        );
        draw(&mut workspace, chart, 3);

        show(&mut workspace, chart, DataSource::Binance, "BTCUSDT");
        assert_eq!(drawing_ids(&workspace, chart), [1, 2]);
        show(&mut workspace, chart, DataSource::Binance, "ETHUSDT");
        assert_eq!(drawing_ids(&workspace, chart), [3]);
    }

    #[test]
    fn the_same_symbol_on_another_source_is_another_market() {
        let mut workspace = empty_workspace(2);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        show(&mut workspace, chart, DataSource::Okx, "BTC-USDT");
        assert!(drawing_ids(&workspace, chart).is_empty());
        show(&mut workspace, chart, DataSource::Binance, "btcusdt");
        assert_eq!(
            drawing_ids(&workspace, chart),
            [1],
            "the same market, spelled otherwise"
        );
    }

    #[test]
    fn a_drawing_never_takes_an_id_a_parked_set_holds() {
        let mut workspace = empty_workspace(3);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 7]);
        workspace.charts[0].next_drawing_id = 8;
        show(&mut workspace, chart, DataSource::Binance, "ETHUSDT");
        assert!(workspace.charts[0].next_drawing_id >= 8);
        let stored = workspace.charts[0].stored_drawings();
        assert_eq!(stored.highest_id(), Some(7));
    }

    #[test]
    fn showing_the_market_it_already_shows_changes_no_drawing() {
        let mut workspace = empty_workspace(4);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        show(&mut workspace, chart, DataSource::Binance, "BTCUSDT");
        assert_eq!(drawing_ids(&workspace, chart), [1, 2]);
        assert_eq!(workspace.charts[0].parked_drawings.count(), 0);
    }

    #[test]
    fn a_ruler_stays_on_the_market_it_measured() {
        let mut workspace = empty_workspace(5);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.charts[0].drawings.push(Drawing::Ruler {
            id: 9,
            a: crate::model::Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: crate::model::Anchor {
                time_ms: 10,
                price: 2.0,
            },
        });
        show(&mut workspace, chart, DataSource::Binance, "ETHUSDT");
        show(&mut workspace, chart, DataSource::Binance, "BTCUSDT");
        assert_eq!(drawing_ids(&workspace, chart), [1]);
    }

    #[test]
    fn the_drawings_of_every_market_are_saved_and_come_back_with_the_chart() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        show(&mut workspace, chart, DataSource::Binance, "ETHUSDT");
        draw(&mut workspace, chart, 3);

        let saved = workspace.persisted().charts.remove(0);
        assert_eq!(saved.drawings.count(), 3);
        let reopened = Chart::restored(saved, workspace.timeframe, &ctx);
        assert_eq!(
            reopened
                .drawings
                .iter()
                .map(Drawing::id)
                .collect::<Vec<_>>(),
            [3]
        );
        assert!(reopened.next_drawing_id >= 4);

        let mut workspace = empty_workspace(7);
        workspace.charts.push(reopened);
        let chart = workspace.charts[0].id;
        show(&mut workspace, chart, DataSource::Binance, "BTCUSDT");
        assert_eq!(drawing_ids(&workspace, chart), [1, 2]);
    }

    #[test]
    fn what_refers_to_the_old_market_s_drawings_is_let_go_when_the_market_changes() {
        let mut workspace = empty_workspace(9);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let second = chart_with_drawings(&mut workspace, "SOLUSDT", &[1]);
        workspace.tools.select(first, vec![1, 2]);
        workspace.tools.ask_to_remove_all(first);

        show(&mut workspace, second, DataSource::Binance, "ADAUSDT");
        assert_eq!(
            workspace.tools.selected_on(first),
            [1, 2],
            "another chart's is kept"
        );
        assert_eq!(workspace.tools.removal_asked(), Some(first));

        show(&mut workspace, first, DataSource::Binance, "ETHUSDT");
        assert!(workspace.tools.selected_on(first).is_empty());
        assert_eq!(workspace.tools.removal_asked(), None);
    }

    #[test]
    fn two_charts_of_one_market_keep_their_own_drawings() {
        let mut workspace = empty_workspace(8);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[1, 2]);
        show(&mut workspace, second, DataSource::Binance, "BTCUSDT");
        assert!(drawing_ids(&workspace, second).is_empty());
        assert_eq!(drawing_ids(&workspace, first), [1]);
    }
}
