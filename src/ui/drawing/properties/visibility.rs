//! The Visibility page: the timeframes a drawing is shown on.

use super::DrawingProperties;
use super::rows::{Page, toggle_row};
use crate::model::Timeframe;
use crate::model::Visibility;
use crate::ui::icons::Icon;
use eframe::egui::Ui;

impl DrawingProperties {
    pub(super) fn visibility_page(&mut self, ui: &mut Ui) {
        let mut visibility = self.draft.visibility();
        let before = visibility;
        let mut page = Page::new(true);
        page.section(
            ui,
            "Show on",
            Some("The drawing is kept when you switch to a timeframe it is not shown on."),
        );
        let mut all = visibility.is_all();
        let was_all = all;
        toggle_row(ui, Icon::Eye, "All timeframes", "", &mut all);
        if all && !was_all {
            visibility = Visibility::ALL;
        }
        for timeframe in Timeframe::ALL {
            let mut shown = visibility.shows(timeframe);
            let was = shown;
            toggle_row(ui, Icon::Clock, timeframe.description(), "", &mut shown);
            if shown != was {
                visibility = visibility.with(timeframe, shown);
            }
        }
        if visibility != before {
            self.draft.set_visibility(visibility);
        }
    }
}
