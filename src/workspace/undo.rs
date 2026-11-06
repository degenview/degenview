//! Undo and redo of drawing and indicator changes, and how a burst of restyles becomes one step.

use super::ChartWorkspace;
use super::chart::Chart;
use super::drawings::is_ruler;
use crate::model::viewport::ChartViewport;
use crate::model::{Drawing, DrawingCommand, IndicatorConfig};
use std::time::{Duration, Instant};

/// A reversible change to one chart: what it held before, so undo can restore it. The stack is
/// the workspace's, in the order things happened, whatever they changed.
pub(super) struct UndoEntry {
    pub(super) chart_id: u64,
    pub(super) before: Snapshot,
}

/// The part of a chart an undoable change touched.
pub(super) enum Snapshot {
    /// The chart's drawings before a batch of commands (rulers excluded).
    Drawings(Vec<Drawing>),
    /// The applied built-in indicators and the scripts on the chart.
    Indicators {
        config: IndicatorConfig,
        scripts: Vec<crate::model::pine_instance::PineInstance>,
    },
    /// What part of the chart was on screen before a Zoom In or Zoom out.
    View(ChartViewport),
}

impl Snapshot {
    /// Whether restoring this changes what a layout stores. A view does not: it is not saved
    /// with a layout, so undoing a zoom leaves the layout as clean as it found it.
    fn changes_layout(&self) -> bool {
        !matches!(self, Self::View(_))
    }

    /// Puts this snapshot on `chart` and returns what the chart held, to undo the undo.
    fn swap_into(self, chart: &mut Chart) -> Snapshot {
        match self {
            Self::Drawings(drawings) => {
                let current = std::mem::replace(&mut chart.drawings, drawings);
                Self::Drawings(current.into_iter().filter(|d| !is_ruler(d)).collect())
            }
            Self::Indicators { config, scripts } => {
                let current = Self::Indicators {
                    config: chart.indicators,
                    scripts: chart.chart.pine_instances().to_vec(),
                };
                chart.indicators = config;
                chart.chart.set_pine_instances(scripts);
                current
            }
            Self::View(view) => Self::View(chart.chart.swap_viewport(view)),
        }
    }
}

/// How long after a restyle another of the same drawing still counts as the same edit.
const RESTYLE_WINDOW: Duration = Duration::from_millis(800);

/// The drawing a batch restyles, when that is all the batch does.
fn restyled_drawing(commands: &[DrawingCommand]) -> Option<u64> {
    match commands {
        [DrawingCommand::Style { id, .. }] => Some(*id),
        _ => None,
    }
}

/// Whether restyling `drawing_id` of `chart_id` at `now` continues `last`.
fn restyle_merges(
    last: Option<(u64, u64, Instant)>,
    chart_id: u64,
    drawing_id: u64,
    now: Instant,
) -> bool {
    last.is_some_and(|(chart, drawing, at)| {
        chart == chart_id
            && drawing == drawing_id
            && now.saturating_duration_since(at) <= RESTYLE_WINDOW
    })
}

impl ChartWorkspace {
    pub(super) fn apply_drawing_commands(&mut self, chart_id: u64, commands: Vec<DrawingCommand>) {
        if commands.is_empty() {
            return;
        }
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        let restyled = restyled_drawing(&commands);
        let now = Instant::now();
        let merge = restyled.is_some_and(|id| restyle_merges(self.last_restyle, chart_id, id, now));
        // A restyle that merges into the last step needs no copy of the drawings to go back to.
        let before: Vec<Drawing> = if merge {
            Vec::new()
        } else {
            chart
                .drawings
                .iter()
                .filter(|drawing| !is_ruler(drawing))
                .cloned()
                .collect()
        };
        // One batch is one undo step, and a batch that changes nothing (or only a ruler) none.
        if crate::model::drawings::apply_commands(&mut chart.drawings, commands) {
            self.drawings_version += 1;
            // A restyle that follows another of the same drawing moments ago is part of it.
            if !merge {
                self.undo.push(UndoEntry {
                    chart_id,
                    before: Snapshot::Drawings(before),
                });
            }
            self.redo.clear();
            self.last_restyle = restyled.map(|id| (chart_id, id, now));
        } else if restyled.is_none() {
            self.last_restyle = None;
        }
    }

    /// Puts the view a zoom left on the undo stack: Zoom In and Zoom out are steps like any edit,
    /// and a new step ends the redo line.
    pub(super) fn remember_view(&mut self, chart_id: u64, before: ChartViewport) {
        self.undo.push(UndoEntry {
            chart_id,
            before: Snapshot::View(before),
        });
        self.redo.clear();
        self.last_restyle = None;
    }

    /// Forgets the views `chart_id` could be put back to (all charts when `None`): they were
    /// positions in a series that is no longer there.
    pub(super) fn forget_views(&mut self, chart_id: Option<u64>) {
        let gone = |entry: &UndoEntry| {
            matches!(entry.before, Snapshot::View(_))
                && chart_id.is_none_or(|id| entry.chart_id == id)
        };
        self.undo.retain(|entry| !gone(entry));
        self.redo.retain(|entry| !gone(entry));
    }

    pub(super) fn undo(&mut self) {
        self.last_restyle = None;
        let Some(entry) = self.undo.pop() else {
            return;
        };
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == entry.chart_id)
        {
            let layout = entry.before.changes_layout();
            let after = entry.before.swap_into(chart);
            self.drawings_version += u64::from(layout);
            self.redo.push(UndoEntry {
                chart_id: entry.chart_id,
                before: after,
            });
        }
    }

    pub(super) fn redo(&mut self) {
        self.last_restyle = None;
        let Some(entry) = self.redo.pop() else {
            return;
        };
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == entry.chart_id)
        {
            let layout = entry.before.changes_layout();
            let after = entry.before.swap_into(chart);
            self.drawings_version += u64::from(layout);
            self.undo.push(UndoEntry {
                chart_id: entry.chart_id,
                before: after,
            });
        }
    }

    pub(super) fn drop_rulers(&mut self) {
        for chart in &mut self.charts {
            chart.drawings.retain(|drawing| !is_ruler(drawing));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tool;
    use crate::workspace::test_support::{
        chart_with_drawings, drawing_ids, empty_workspace, rectangle,
    };

    #[test]
    fn restyles_of_one_drawing_in_a_burst_are_one_edit() {
        let now = Instant::now();
        let soon = now + Duration::from_millis(300);
        let late = now + Duration::from_secs(2);
        let last = Some((1, 7, now));
        assert!(restyle_merges(last, 1, 7, soon));
        assert!(
            !restyle_merges(last, 1, 7, late),
            "a pause starts a new edit"
        );
        assert!(!restyle_merges(last, 1, 8, soon), "another drawing");
        assert!(!restyle_merges(last, 2, 7, soon), "another chart");
        assert!(!restyle_merges(None, 1, 7, soon));
    }

    #[test]
    fn only_a_lone_restyle_command_counts_as_one() {
        let drawing = Drawing::Ruler {
            id: 3,
            a: crate::model::Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: crate::model::Anchor {
                time_ms: 1,
                price: 2.0,
            },
        };
        let style = DrawingCommand::Style {
            id: 3,
            drawing: drawing.clone(),
        };
        assert_eq!(restyled_drawing(std::slice::from_ref(&style)), Some(3));
        assert_eq!(restyled_drawing(&[style.clone(), style]), None);
        assert_eq!(restyled_drawing(&[DrawingCommand::Remove(3)]), None);
        assert_eq!(restyled_drawing(&[]), None);
    }

    #[test]
    fn one_batch_of_drawing_commands_is_one_undo_step_and_redo_replays_it() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        // Draw, move (two commands at once), delete: three steps.
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 500))]);
        workspace.apply_drawing_commands(
            chart,
            vec![
                DrawingCommand::Replace {
                    id: 1,
                    drawing: rectangle(1, 900),
                },
                DrawingCommand::Replace {
                    id: 2,
                    drawing: rectangle(2, 950),
                },
            ],
        );
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(1)]);
        assert_eq!(drawing_ids(&workspace, chart), vec![2]);

        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1, 2]);
        assert_eq!(
            workspace.charts[0].drawings[0].anchors()[0].time_ms,
            900,
            "the move came back as one step"
        );
        workspace.undo();
        assert_eq!(workspace.charts[0].drawings[0].anchors()[0].time_ms, 100);
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);
        workspace.undo();
        assert_eq!(
            drawing_ids(&workspace, chart),
            vec![1],
            "nothing earlier to undo"
        );

        for _ in 0..3 {
            workspace.redo();
        }
        assert_eq!(drawing_ids(&workspace, chart), vec![2]);
        // A new edit clears what could be redone.
        workspace.undo();
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(3, 0))]);
        workspace.redo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1, 2, 3]);
    }

    #[test]
    fn a_ruler_and_a_batch_that_changes_nothing_leave_no_undo_entry() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let ruler = Drawing::Ruler {
            id: 50,
            a: crate::model::Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: crate::model::Anchor {
                time_ms: 9,
                price: 2.0,
            },
        };
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(ruler)]);
        workspace.apply_drawing_commands(
            chart,
            vec![DrawingCommand::Replace {
                id: 1,
                drawing: rectangle(1, 100),
            }],
        );
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(404)]);
        assert!(workspace.undo.is_empty(), "no entries");
        assert_eq!(workspace.charts[0].drawings.len(), 2, "the ruler is there");
        // A persisted chart never carries it.
        assert!(
            workspace.persisted().charts[0]
                .drawings
                .iter()
                .all(|d| !d.is_ruler())
        );
    }

    #[test]
    fn a_burst_of_restyles_is_one_undo_step() {
        let mut workspace = empty_workspace(10);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let restyle = |width: f32| {
            let mut drawing = rectangle(1, 100);
            let mut style = *drawing.style().expect("style");
            style.stroke.width = width;
            drawing.set_style(style);
            DrawingCommand::Style { id: 1, drawing }
        };
        // Dragging a colour picker sends one restyle per frame.
        for width in [3.0, 4.0, 5.0, 6.0] {
            workspace.apply_drawing_commands(chart, vec![restyle(width)]);
        }
        assert_eq!(workspace.undo.len(), 1);
        assert_eq!(
            workspace.charts[0].drawings[0]
                .style()
                .unwrap()
                .stroke
                .width,
            6.0
        );
        workspace.undo();
        assert_eq!(
            workspace.charts[0].drawings[0]
                .style()
                .unwrap()
                .stroke
                .width,
            Tool::Rectangle.default_style().stroke.width,
            "back to how it was before the burst"
        );
        // After an undo the next restyle starts a new step.
        workspace.apply_drawing_commands(chart, vec![restyle(8.0)]);
        workspace.apply_drawing_commands(chart, vec![restyle(9.0)]);
        assert_eq!(workspace.undo.len(), 1);
        // Another edit in between ends the burst.
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 300))]);
        workspace.apply_drawing_commands(chart, vec![restyle(10.0)]);
        assert_eq!(workspace.undo.len(), 3);
    }

    // MARK: Zoom steps

    fn view(visible: usize, right_offset: f64) -> ChartViewport {
        let mut view = ChartViewport::default();
        view.horizontal.visible = visible;
        view.horizontal.right_offset = right_offset;
        view
    }

    fn chart_offset(workspace: &ChartWorkspace, chart_id: u64) -> f64 {
        let saved = workspace
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .expect("chart")
            .chart
            .saved_viewport();
        saved.right_offset
    }

    #[test]
    fn a_zoom_is_an_undo_step_and_redo_replays_it() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let version = workspace.drawings_version;
        // The zoom has been applied by the card; the workspace is handed the view it left.
        let before = ChartViewport::default();
        let card = &mut workspace.charts[0].chart;
        card.swap_viewport(view(40, 30.0));
        workspace.remember_view(chart, before);

        workspace.undo();
        assert_eq!(chart_offset(&workspace, chart), 0.0, "back where it was");
        workspace.redo();
        assert_eq!(chart_offset(&workspace, chart), 30.0, "zoomed again");
        workspace.undo();
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert_eq!(
            workspace.drawings_version, version,
            "a view is not part of what a layout stores"
        );
    }

    #[test]
    fn a_new_step_ends_the_redo_line_and_steps_run_in_the_order_they_happened() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(1)]);
        workspace.charts[0].chart.swap_viewport(view(40, 30.0));
        workspace.remember_view(chart, ChartViewport::default());

        // Undo takes the zoom first, then the removal.
        workspace.undo();
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(
            drawing_ids(&workspace, chart).is_empty(),
            "the zoom came last"
        );
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);

        // Zooming again after an undo drops what could have been redone.
        workspace.charts[0].chart.swap_viewport(view(60, 10.0));
        workspace.remember_view(chart, ChartViewport::default());
        assert!(workspace.redo.is_empty());
    }

    #[test]
    fn zoom_out_is_an_undo_step_and_the_history_follows_undo_and_redo() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.focused = Some(chart);
        // A zoom: the card holds the zoomed view and one view to go back to.
        workspace.charts[0].chart.swap_viewport(view(40, 30.0));
        workspace.remember_view(chart, ChartViewport::default());
        assert!(workspace.charts[0].chart.can_zoom_out());

        workspace.zoom_out_focused();
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(!workspace.charts[0].chart.can_zoom_out());
        assert_eq!(workspace.undo.len(), 2, "Zoom out is a step too");

        // Undoing it zooms back in, and Zoom out is offered again.
        workspace.undo();
        assert_eq!(chart_offset(&workspace, chart), 30.0);
        assert!(workspace.charts[0].chart.can_zoom_out());
        // Undoing the zoom itself leaves nothing to zoom out of.
        workspace.undo();
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(!workspace.charts[0].chart.can_zoom_out());
        // Redo brings both back, history included.
        workspace.redo();
        assert_eq!(chart_offset(&workspace, chart), 30.0);
        assert!(workspace.charts[0].chart.can_zoom_out());
        workspace.redo();
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(!workspace.charts[0].chart.can_zoom_out());
    }

    #[test]
    fn views_are_forgotten_with_the_series_they_belonged_to() {
        let mut workspace = empty_workspace(9);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[2]);
        workspace.remember_view(first, ChartViewport::default());
        workspace.remember_view(second, ChartViewport::default());
        workspace.apply_drawing_commands(first, vec![DrawingCommand::Remove(1)]);
        workspace.forget_views(Some(first));
        // The other chart's view and the drawing step stay.
        assert_eq!(workspace.undo.len(), 2);
        assert!(workspace.undo.iter().any(|e| e.chart_id == second));
        workspace.forget_views(None);
        assert_eq!(workspace.undo.len(), 1);
        assert!(matches!(workspace.undo[0].before, Snapshot::Drawings(_)));
    }
}
