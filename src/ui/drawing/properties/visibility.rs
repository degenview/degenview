//! The Visibility page: the timeframes a drawing is shown on.

use super::DrawingProperties;
use super::rows::{Page, toggle_row};
use crate::i18n::tr;
use crate::model::{Visibility, VisibilityRow};
use crate::ui::icons::Icon;
use eframe::egui::Ui;

impl DrawingProperties {
    pub(super) fn visibility_page(&mut self, ui: &mut Ui) {
        let mut visibility = self.draft.visibility();
        let before = visibility;
        let mut page = Page::new(true);
        page.section(
            ui,
            tr!("Show on"),
            Some(tr!(
                "The drawing is kept when you switch to a timeframe it is not shown on."
            )),
        );
        let mut all = visibility.is_all();
        let was_all = all;
        toggle_row(ui, Icon::Eye, tr!("All timeframes"), "", &mut all);
        if all && !was_all {
            visibility = Visibility::ALL;
        }
        for row in VisibilityRow::ALL {
            let mut shown = visibility.shows_row(row);
            let was = shown;
            toggle_row(ui, Icon::Clock, row.label(), "", &mut shown);
            if shown != was {
                visibility = visibility.with_row(row, shown);
            }
        }
        if visibility != before {
            self.draft.set_visibility(visibility);
        }
    }
}
