//! Undo and redo of drawing and indicator changes, and how a burst of restyles becomes one step.

use super::ChartWorkspace;
use super::chart::Chart;
use super::drawings::is_ruler;
use crate::model::{Drawing, DrawingCommand, IndicatorConfig};
use crate::ui::{ViewMove, ViewStep};
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
    /// What part of the chart was on screen before a Zoom In or Zoom out, and which way applying it
    /// takes Zoom out's history (a zoom is undone by going back; its inverse goes forward).
    View { step: ViewStep, direction: ViewMove },
}

impl Snapshot {
    /// Whether restoring this changes what a layout stores. A view does not: it is not saved
    /// with a layout, so undoing a zoom leaves the layout as clean as it found it.
    fn changes_layout(&self) -> bool {
        !matches!(self, Self::View { .. })
    }

    /// Puts this snapshot on `chart` and returns what the chart held, to undo the undo. `None` when
    /// it cannot be put back (a view of bars the chart no longer shows): nothing changed.
    fn swap_into(self, chart: &mut Chart) -> Option<Snapshot> {
        Some(match self {
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
            Self::View { step, direction } => Self::View {
                step: chart.chart.apply_view(step, direction)?,
                direction: direction.inverse(),
            },
        })
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
    pub(super) fn remember_view(&mut self, chart_id: u64, step: ViewStep, direction: ViewMove) {
        self.undo.push(UndoEntry {
            chart_id,
            before: Snapshot::View { step, direction },
        });
        self.redo.clear();
        self.last_restyle = None;
    }

    /// Forgets the views `chart_id` could be put back to (all charts when `None`): they were
    /// positions in a series that is no longer there.
    pub(super) fn forget_views(&mut self, chart_id: Option<u64>) {
        let gone = |entry: &UndoEntry| {
            matches!(entry.before, Snapshot::View { .. })
                && chart_id.is_none_or(|id| entry.chart_id == id)
        };
        self.undo.retain(|entry| !gone(entry));
        self.redo.retain(|entry| !gone(entry));
    }

    pub(super) fn undo(&mut self) {
        self.step_through(true);
    }

    pub(super) fn redo(&mut self) {
        self.step_through(false);
    }

    /// One press of Undo (or Redo): the newest step moves from one stack to the other. A view of
    /// bars the chart no longer shows cannot be put back, so it is dropped and the step beneath it
    /// is the one this press takes: a press that did nothing would be worse than skipping.
    fn step_through(&mut self, undoing: bool) {
        self.last_restyle = None;
        loop {
            let entry = if undoing {
                self.undo.pop()
            } else {
                self.redo.pop()
            };
            let Some(entry) = entry else {
                return;
            };
            let Some(chart) = self
                .charts
                .iter_mut()
                .find(|chart| chart.id == entry.chart_id)
            else {
                return;
            };
            let layout = entry.before.changes_layout();
            let Some(after) = entry.before.swap_into(chart) else {
                continue;
            };
            self.drawings_version += u64::from(layout);
            let moved = UndoEntry {
                chart_id: entry.chart_id,
                before: after,
            };
            if undoing {
                self.redo.push(moved);
            } else {
                self.undo.push(moved);
            }
            return;
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

    /// What the grid does when a card reports a zoom: record the view it left.
    fn zoomed(workspace: &mut ChartWorkspace, chart_id: u64, visible: usize, offset: f64) {
        let card = &mut workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .expect("chart")
            .chart;
        card.zoom_to_for_test(visible, offset);
        let before = card.take_zoomed().expect("zoomed");
        workspace.remember_view(chart_id, before, ViewMove::Back);
    }

    fn chart_offset(workspace: &ChartWorkspace, chart_id: u64) -> f64 {
        workspace
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .expect("chart")
            .chart
            .saved_viewport()
            .right_offset
    }

    fn history(workspace: &ChartWorkspace) -> usize {
        workspace.charts[0].chart.zoom_history_len()
    }

    #[test]
    fn a_zoom_is_an_undo_step_and_redo_replays_it() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let version = workspace.drawings_version;
        zoomed(&mut workspace, chart, 40, 30.0);

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
        zoomed(&mut workspace, chart, 40, 30.0);

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
        zoomed(&mut workspace, chart, 60, 10.0);
        assert!(workspace.redo.is_empty());
    }

    #[test]
    fn zoom_out_is_an_undo_step_and_the_history_follows_undo_and_redo() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.focused = Some(chart);
        zoomed(&mut workspace, chart, 40, 30.0);
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
    fn undoing_past_the_history_cap_never_offers_a_forward_zoom_out() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let cap = crate::model::viewport::zoom::MAX_BACK;
        let zooms = cap + 2;
        for i in 1..=zooms {
            zoomed(&mut workspace, chart, 120 - 10 * i, 10.0 * i as f64);
        }
        assert_eq!(history(&workspace), cap, "the history keeps the newest few");

        // Undo every zoom: the history empties as the zooms it lists are undone and stays empty
        // for the ones it had already forgotten.
        for undone in 1..=zooms {
            workspace.undo();
            assert_eq!(
                history(&workspace),
                cap.saturating_sub(undone),
                "after undoing {undone}"
            );
            assert_eq!(
                workspace.charts[0].chart.can_zoom_out(),
                undone < cap,
                "Zoom out is only offered for a zoom that can still be undone, {undone}"
            );
        }
        assert_eq!(chart_offset(&workspace, chart), 0.0);

        // Redo them all: each one offers the view it came from, up to the cap.
        for redone in 1..=zooms {
            workspace.redo();
            assert_eq!(
                history(&workspace),
                redone.min(cap),
                "after redoing {redone}"
            );
            assert!(workspace.charts[0].chart.can_zoom_out());
        }
        assert_eq!(chart_offset(&workspace, chart), 10.0 * zooms as f64);
    }

    #[test]
    fn a_view_of_another_series_is_dropped_and_the_step_beneath_it_is_undone() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(1)]);
        zoomed(&mut workspace, chart, 40, 30.0);
        // The bars the offset counted are gone.
        workspace.charts[0]
            .chart
            .set_chart_type(crate::model::ChartType::Renko);
        let viewport = workspace.charts[0].chart.saved_viewport();

        workspace.undo();
        assert_eq!(
            workspace.charts[0].chart.saved_viewport(),
            viewport,
            "the stale view was not put on the new series"
        );
        assert!(
            !workspace.charts[0].chart.can_zoom_out(),
            "and made no Zoom out"
        );
        assert_eq!(
            drawing_ids(&workspace, chart),
            vec![1],
            "the press undid the step beneath it instead of doing nothing"
        );
        assert!(workspace.undo.is_empty());
    }

    #[test]
    fn views_are_forgotten_with_the_series_they_belonged_to() {
        let mut workspace = empty_workspace(9);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[2]);
        zoomed(&mut workspace, first, 40, 30.0);
        zoomed(&mut workspace, second, 40, 30.0);
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
