//! Turns a right-click on a chart into the snapshot its menu works from.
//!
//! The click is classified with the same [`navigation::Regions`] that decide which gesture a
//! press starts, and its time and price are read through the chart's own projection and price
//! scale, so the menu agrees with the labels, the crosshair and the drawings. Nothing is
//! recomputed later: the request is a plain value the workspace keeps while the menu is open.

use super::navigation::{Region, Regions};
use super::{ChartCard, PriceScale};
use crate::model::Anchor;
use crate::model::chart_context::Target;
use crate::model::price_scale::PriceScaleMode;
use crate::ui::alert_lines;
use crate::ui::drawing_layer::DrawingFrame;
use crate::ui::drawing_layer::Projection;
use eframe::egui::{self, Pos2, Rect};

/// A right-click that is the chart menu's to answer.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextRequest {
    /// Where the pointer was, for placing the menu.
    pub at: Pos2,
    pub time_ms: Option<i64>,
    pub price: Option<f64>,
    pub target: Target,
}

/// What a classified click is, before time and price are read: which pane, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Spot {
    Plot,
    PriceAxis,
    TimeAxis,
    Pane(usize),
}

/// Where `at` is, with the script pane it is in; `None` in the corner and outside the chart.
pub(super) fn spot(at: Pos2, regions: &Regions, panes: &[Rect]) -> Option<Spot> {
    match regions.of(at) {
        Region::Plot => Some(Spot::Plot),
        Region::PriceAxis => Some(Spot::PriceAxis),
        Region::TimeAxis => Some(Spot::TimeAxis),
        Region::Pane => panes
            .iter()
            .position(|pane| pane.contains(at))
            .map(Spot::Pane),
        Region::Elsewhere => None,
    }
}

/// The request for a click at `at` on `spot`, read through the chart's projection and scale.
/// A price off the chart's scale (not finite) is dropped, never invented.
pub(super) fn request(
    at: Pos2,
    spot: Spot,
    projection: &Projection,
    scale: &PriceScale,
    pane_ids: &[u64],
) -> ContextRequest {
    let finite = |price: f64| price.is_finite().then_some(price);
    let (time_ms, price, target) = match spot {
        Spot::Plot => {
            let Anchor { time_ms, price } = projection.anchor(at);
            (Some(time_ms), finite(price), Target::Background)
        }
        Spot::PriceAxis => (None, finite(scale.price(at.y)), Target::PriceAxis),
        Spot::TimeAxis => (Some(projection.time(at.x)), None, Target::TimeAxis),
        Spot::Pane(index) => match pane_ids.get(index) {
            Some(&instance) => (None, None, Target::Pane { instance }),
            None => (None, None, Target::Background),
        },
    };
    ContextRequest {
        at,
        time_ms,
        price,
        target,
    }
}

impl ChartCard {
    /// The request for this frame's right-click, if the chart's menu is to answer it. A drawing
    /// answers its own clicks; a paper marker is reported by its pill (drawn last frame).
    pub(super) fn context_request(
        &mut self,
        response: &egui::Response,
        regions: &Regions,
        panes: &[Rect],
        projection: &Projection,
        scale: &PriceScale,
        drawing: &mut DrawingFrame,
    ) -> Option<ContextRequest> {
        if let Some(marker) = self.paper.menu.take() {
            return Some(ContextRequest {
                at: marker.at,
                time_ms: None,
                price: None,
                target: marker.target,
            });
        }
        if let Some(menu) = self.alert_menu.take() {
            return Some(ContextRequest {
                at: menu.at,
                time_ms: None,
                price: None,
                target: Target::Alert { id: menu.id },
            });
        }
        // The layer reports a plot click it did not use (no drawing, no draft to finish).
        if let Some(at) = drawing.background_click.take() {
            return Some(request(at, Spot::Plot, projection, scale, &self.pane_ids));
        }
        if !response.secondary_clicked() {
            return None;
        }
        let at = response.interact_pointer_pos()?;
        match spot(at, regions, panes)? {
            // A plot click is the layer's to classify.
            Spot::Plot => None,
            other => Some(request(at, other, projection, scale, &self.pane_ids)),
        }
    }
}

impl ChartCard {
    /// The ids of the alerts drawn as lines on this chart, for tests.
    #[cfg(test)]
    pub fn alert_line_ids(&self) -> Vec<u64> {
        self.alert_lines.iter().map(|line| line.id).collect()
    }

    /// Whether every alert line this chart draws can be dragged, for tests.
    #[cfg(test)]
    pub fn alert_lines_editable(&self) -> bool {
        !self.alert_lines.is_empty() && self.alert_lines.iter().all(|line| line.editable)
    }

    /// A tag dropped on `price`, as the pointer would report it, for tests.
    #[cfg(test)]
    pub fn queue_alert_move(&mut self, id: u64, price: f64) {
        self.alert_moves.push(alert_lines::AlertMove { id, price });
    }

    /// Sets the market the alert tags are dragged on: the last price, which decides the side a
    /// moved alert then watches, and the tick a drop lands on.
    pub fn set_alert_market(&mut self, last: Option<f64>, tick: f64) {
        self.alert_market = alert_lines::Market {
            last,
            tick,
            ..alert_lines::Market::default()
        };
    }

    /// Shows or hides the price-scale menu in the corner of the chart.
    pub fn set_scale_menu(&mut self, shown: bool) {
        self.scale_menu = shown;
    }

    /// The mode picked from the corner menu since the last call.
    pub fn take_scale_request(&mut self) -> Option<PriceScaleMode> {
        self.scale_request.take()
    }

    /// The alert tags dropped on a new level since the last call.
    pub fn take_alert_moves(&mut self) -> Vec<alert_lines::AlertMove> {
        std::mem::take(&mut self.alert_moves)
    }

    /// Sets the price alerts this chart draws as lines.
    pub fn set_alert_lines(&mut self, lines: Vec<crate::model::alert_line::AlertLine>) {
        if self.alert_lines != lines {
            self.alert_lines = lines;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn regions() -> Regions {
        let main = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 320.0));
        Regions {
            plot: Rect::from_min_size(pos2(0.0, 0.0), vec2(440.0, 300.0)),
            main,
            chart: Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 420.0)),
        }
    }

    fn panes() -> Vec<Rect> {
        vec![
            Rect::from_min_size(pos2(0.0, 320.0), vec2(500.0, 50.0)),
            Rect::from_min_size(pos2(0.0, 370.0), vec2(500.0, 50.0)),
        ]
    }

    #[test]
    fn a_click_is_classified_by_where_it_lands() {
        let (regions, panes) = (regions(), panes());
        assert_eq!(spot(pos2(100.0, 100.0), &regions, &panes), Some(Spot::Plot));
        assert_eq!(
            spot(pos2(470.0, 100.0), &regions, &panes),
            Some(Spot::PriceAxis)
        );
        assert_eq!(
            spot(pos2(100.0, 310.0), &regions, &panes),
            Some(Spot::TimeAxis)
        );
        assert_eq!(
            spot(pos2(100.0, 335.0), &regions, &panes),
            Some(Spot::Pane(0))
        );
        assert_eq!(
            spot(pos2(100.0, 400.0), &regions, &panes),
            Some(Spot::Pane(1))
        );
    }

    #[test]
    fn the_corner_and_the_outside_have_no_menu() {
        let (regions, panes) = (regions(), panes());
        assert_eq!(spot(pos2(470.0, 310.0), &regions, &panes), None);
        assert_eq!(spot(pos2(900.0, 100.0), &regions, &panes), None);
    }
}
