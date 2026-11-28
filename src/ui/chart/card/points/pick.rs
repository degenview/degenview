//! "Click the chart" mode for a script's point: the points clicked so far, the prompt, and the
//! click. A drag still pans and the wheel still zooms; only a click answers.

use super::super::crosshair::paint_value_tag;
use super::super::scripts;
use super::super::*;
use super::SetupEvent;
use super::markers::Surface;
use crate::model::MagnetMode;
use crate::ui::pine::setup::prompt;

impl ChartCard {
    /// Whether a click on the plot answers the point being asked: a pick of bars, Select portion,
    /// Zoom In and a drawing tool taken up meanwhile have the pointer first.
    pub(in crate::ui::chart::card) fn picking_point(
        &self,
        tool: crate::model::Tool,
        pointer_taken: bool,
    ) -> bool {
        self.setup.pick.is_some() && tool == crate::model::Tool::Cursor && !pointer_taken
    }

    /// Where the asked point is read: a script pane's own scale, else the price chart's.
    fn pick_surface(
        &self,
        layout: &frame::FrameLayout,
        view: &View,
        shown: usize,
    ) -> Option<Surface> {
        let pick = self.setup.pick.as_ref()?;
        let pane = pick.pane.and_then(|id| {
            let index = layout
                .pane_outputs
                .iter()
                .position(|(pane, _)| *pane == id)?;
            let rect = *layout.pane_rects.get(index)?;
            let plot = scripts::pane_plot(layout.plot, rect);
            let scale = scripts::pane_scale(layout.pane_outputs[index].1, shown, plot);
            Some(Surface { rect: plot, scale })
        });
        pane.or_else(|| {
            layout.main_visible.then(|| Surface {
                rect: self.below_header(layout.plot),
                scale: view.scale.clone(),
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui::chart::card) fn handle_point_pick(
        &mut self,
        ui: &mut egui::Ui,
        response: &Response,
        painter: &egui::Painter,
        layout: &frame::FrameLayout,
        view: &View,
        projection: &Projection,
        magnet: MagnetMode,
        chart_id: u64,
    ) {
        let shown = view.end.saturating_sub(view.start);
        let Some(surface) = self.pick_surface(layout, view, shown) else {
            return;
        };
        let Some(pick) = self.setup.pick.clone() else {
            return;
        };
        for point in &pick.picked {
            // A point is drawn where it was read: a price from the price chart stays there.
            self.paint_point(painter, &surface, projection, layout, *point);
        }
        if let Some(text) = &pick.prompt
            && prompt::show(ui, chart_id, layout.plot, text)
        {
            self.setup_events.push(SetupEvent::PromptClosed);
        }
        let Some(at) = response
            .hover_pos()
            .filter(|position| surface.rect.contains(*position))
        else {
            return;
        };
        if pick.pane.is_some() && pick.axes.price().is_some() {
            canvas::horizontal(
                painter,
                (surface.rect.left(), surface.rect.right()),
                at.y,
                layout.style.crosshair,
            );
            let label = surface
                .scale
                .tag_label(self.appearance.price_format(), surface.scale.price(at.y));
            let axis = (layout.plot.right(), layout.rect.right());
            paint_value_tag(painter, axis, at.y, &label, &layout.style);
        }
        if !response.clicked() {
            return;
        }
        // The magnet reads the price chart's bars, so it only pulls a click on the price chart.
        let magnet = match pick.pane {
            None => magnet.inverted(ui.input(|input| input.modifiers.command)),
            Some(_) => MagnetMode::Off,
        };
        let (anchor, snapped) = projection.bar_anchor(at, magnet);
        let price = if snapped {
            anchor.price
        } else {
            surface.scale.price(at.y)
        };
        if price.is_finite() {
            self.setup_events.push(SetupEvent::Picked {
                time_ms: anchor.time_ms,
                price,
            });
        }
    }
}
