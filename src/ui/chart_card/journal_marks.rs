//! The Trade Journal's trades on the chart: painted in the shared frame (so a snapshot shows them
//! when it includes trading), hovered and clicked on the live chart.
//!
//! The marks are input the workspace fills each frame from the journal (the market's trades, for
//! the account the journal is showing); the card never reads the journal itself. Like the paper
//! markers they are view state: not drawings, never saved, in no template, and they never feed the
//! price scale. A click reports the trade's key and the workspace asks the app to open it.

use std::sync::Arc;

use super::{ChartCard, Projection};
use crate::ui::journal_markers::{self, Palette};
use degenview_journal::marks::TradeMark;
use eframe::egui::{Painter, Pos2, Rect};

#[derive(Default)]
pub struct JournalCardState {
    /// In: this market's trades.
    pub marks: Option<Arc<Vec<TradeMark>>>,
    /// Out: the trade the user clicked, to open in the journal.
    pub open: Option<String>,
}

impl ChartCard {
    /// Whether this chart has journal trades to show and shows them.
    pub(super) fn shows_journal(&self) -> bool {
        self.appearance.canvas.show_journal_trades
            && self
                .journal
                .marks
                .as_ref()
                .is_some_and(|marks| !marks.is_empty())
    }

    /// Lays out and paints the trades in view and returns the one under `pointer`.
    pub(super) fn paint_journal_marks(
        &self,
        painter: &Painter,
        projection: &Projection,
        plot: Rect,
        pointer: Option<Pos2>,
    ) -> Option<TradeMark> {
        let marks = self.journal.marks.as_ref()?;
        // In a replay the chart ends before the present: a trade that has not happened yet at the
        // cursor is not shown.
        let horizon = projection
            .times
            .bars()
            .last()
            .map_or(i64::MAX, |bar| bar.close_time.max(bar.open_time));
        let cut;
        let shown: &[TradeMark] = if marks.last().is_none_or(|mark| mark.open_ts_ms <= horizon) {
            marks
        } else {
            cut = marks
                .iter()
                .filter(|mark| mark.open_ts_ms <= horizon)
                .cloned()
                .collect::<Vec<_>>();
            &cut
        };
        if shown.is_empty() {
            return None;
        }
        let items = journal_markers::layout(
            shown,
            plot,
            &|time| projection.x(time),
            &|price| projection.scale.y(price),
            &|text| {
                painter
                    .layout_no_wrap(
                        text.to_owned(),
                        eframe::egui::FontId::monospace(10.5),
                        eframe::egui::Color32::WHITE,
                    )
                    .size()
            },
        );
        let hovered = pointer
            .filter(|pointer| plot.contains(*pointer))
            .and_then(|pointer| journal_markers::hit(&items, pointer));
        let (up, down) = self.up_down_colors();
        journal_markers::paint(painter, &items, shown, &Palette::new(up, down), hovered);
        hovered.map(|index| shown[index].clone())
    }
}
