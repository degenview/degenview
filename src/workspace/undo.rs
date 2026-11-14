//! Undo and redo of drawing, indicator and market changes, and how a burst of restyles becomes one
//! step.

use super::ChartWorkspace;
use super::chart::{Chart, MarketState};
use super::drawing::is_ruler;
use crate::model::overlay::OverlayId;
use crate::model::watchlist::WatchlistInstrument;
use crate::model::{Drawing, DrawingCommand, IndicatorConfig};
use crate::ui::{ViewMove, ViewStep};
use eframe::egui;
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
        scripts: Vec<crate::model::pine::instance::PineInstance>,
    },
    /// The market the chart showed before it was replaced. Putting it back reloads the candles.
    Market(MarketState),
    /// The markets compared on the chart, with their looks: configuration only, never candles.
    /// `scale_mode` is set only by the add that switched the axis to Percent and by the removal of
    /// the last market that switched it back to Regular, so undoing either puts the axis back too
    /// (and redo switches it again); any other step leaves the mode alone.
    Overlays {
        list: crate::model::overlay::OverlayList,
        scale_mode: Option<crate::model::chart::price_scale::PriceScaleMode>,
    },
    /// What part of the chart was on screen before a Zoom In or Zoom out, and which way applying it
    /// takes Zoom out's history (a zoom is undone by going back; its inverse goes forward).
    View { step: ViewStep, direction: ViewMove },
    /// Several of the above restored together: one step for a change that touched them all
    /// (Remove indicators takes the compared markets with it).
    All(Vec<Snapshot>),
}

impl Snapshot {
    /// The applied built-in indicators and the scripts `chart` holds now.
    pub(super) fn indicators_of(chart: &Chart) -> Self {
        Self::Indicators {
            config: chart.indicators,
            scripts: chart.chart.pine_instances().to_vec(),
        }
    }

    /// Whether restoring this swaps the chart's market, which needs its candles loaded again.
    fn changes_market(&self) -> bool {
        match self {
            Self::Market(_) => true,
            Self::All(parts) => parts.iter().any(Self::changes_market),
            _ => false,
        }
    }

    /// Whether restoring this changes what a layout stores. A view does not: it is not saved
    /// with a layout, so undoing a zoom leaves the layout as clean as it found it.
    fn changes_layout(&self) -> bool {
        match self {
            Self::View { .. } => false,
            Self::All(parts) => parts.iter().any(Self::changes_layout),
            _ => true,
        }
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
                let current = Self::indicators_of(chart);
                chart.indicators = config;
                chart.chart.set_pine_instances(scripts);
                current
            }
            Self::Market(state) => Self::Market(chart.swap_market(state)),
            Self::Overlays { list, scale_mode } => Self::Overlays {
                list: std::mem::replace(&mut chart.overlays, list),
                scale_mode: scale_mode.map(|mode| std::mem::replace(&mut chart.scale_mode, mode)),
            },
            Self::View { step, direction } => Self::View {
                step: chart.chart.apply_view(step, direction)?,
                direction: direction.inverse(),
            },
            Self::All(parts) => Self::All(
                parts
                    .into_iter()
                    .map(|part| part.swap_into(chart))
                    .collect::<Option<Vec<_>>>()?,
            ),
        })
    }
}

/// How long after a restyle another of the same drawing still counts as the same edit.
pub(super) const RESTYLE_WINDOW: Duration = Duration::from_millis(800);

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

/// The workspace's undo and redo stacks, and the runs of edits that collapse into one step (a
/// drawing's restyles, an overlay's colour drag). Every step goes through `record`, which ends the
/// redo line and any restyle run.
#[derive(Default)]
pub(super) struct UndoHistory {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    /// The last restyle, so a burst of them (dragging a colour picker) is one undo step.
    last_restyle: Option<(u64, u64, Instant)>,
    /// The overlay colour edit the undo stack last took: chart, overlay, the stack's length right
    /// after, and when. A drag of the colour picker continues it instead of stacking steps.
    last_overlay_style: Option<(u64, OverlayId, usize, Instant)>,
}

impl UndoHistory {
    /// Takes a new step: what `chart_id` held before the change. It ends the redo line and any
    /// run of restyles.
    pub(super) fn record(&mut self, chart_id: u64, before: Snapshot) {
        self.undo.push(UndoEntry { chart_id, before });
        self.redo.clear();
        self.last_restyle = None;
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps Undo can take.
    #[cfg(test)]
    pub(super) fn steps(&self) -> usize {
        self.undo.len()
    }

    /// Forgets every step: they belonged to a layout that is no longer open.
    pub(super) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Forgets the drawing steps of `chart_id`: they would put back drawings that were since
    /// added or removed elsewhere.
    pub(super) fn forget_drawings(&mut self, chart_id: u64) {
        self.forget(|entry| {
            entry.chart_id == chart_id && matches!(entry.before, Snapshot::Drawings(_))
        });
    }

    /// Drops the steps, undoable or redoable, that `gone` matches.
    fn forget(&mut self, gone: impl Fn(&UndoEntry) -> bool) {
        self.undo.retain(|entry| !gone(entry));
        self.redo.retain(|entry| !gone(entry));
    }

    /// Takes a drawing edit: `before` is the step to push, `None` when the edit continues the last
    /// restyle; `restyle` is the drawing a lone restyle changed and when, which starts (or
    /// continues) a run.
    fn record_drawing_edit(
        &mut self,
        chart_id: u64,
        before: Option<Snapshot>,
        restyle: Option<(u64, Instant)>,
    ) {
        if let Some(before) = before {
            self.undo.push(UndoEntry { chart_id, before });
        }
        self.redo.clear();
        self.last_restyle = restyle.map(|(drawing, at)| (chart_id, drawing, at));
    }

    /// Whether restyling `drawing_id` of `chart_id` at `now` continues the last restyle.
    fn continues_restyle(&self, chart_id: u64, drawing_id: u64, now: Instant) -> bool {
        restyle_merges(self.last_restyle, chart_id, drawing_id, now)
    }

    fn end_restyle_run(&mut self) {
        self.last_restyle = None;
    }

    /// Takes the newest step off the undo stack (or the redo stack).
    fn pop(&mut self, undoing: bool) -> Option<UndoEntry> {
        if undoing {
            self.undo.pop()
        } else {
            self.redo.pop()
        }
    }

    /// Puts what a step replaced on the other stack, so the step can be taken back.
    fn push_inverse(&mut self, undoing: bool, entry: UndoEntry) {
        if undoing {
            self.redo.push(entry);
        } else {
            self.undo.push(entry);
        }
    }

    /// Whether a colour edit of `overlay` on `chart_id` at `now` continues the last one: the same
    /// overlay, no other step since, and within the restyle window.
    pub(super) fn continues_overlay_style(
        &self,
        chart_id: u64,
        overlay: OverlayId,
        now: Instant,
    ) -> bool {
        self.last_overlay_style
            .is_some_and(|(chart, last, len, at)| {
                chart == chart_id
                    && last == overlay
                    && len == self.undo.len()
                    && now.saturating_duration_since(at) <= RESTYLE_WINDOW
            })
    }

    /// Notes the overlay edit just made: a colour edit of `(chart, overlay)` at `at` starts or
    /// continues a run, anything else (`None`) ends it.
    pub(super) fn set_overlay_style_run(&mut self, run: Option<(u64, OverlayId, Instant)>) {
        self.last_overlay_style =
            run.map(|(chart, overlay, at)| (chart, overlay, self.undo.len(), at));
    }

    /// Moves the overlay colour run back in time by `age`, as if the user had paused.
    #[cfg(test)]
    pub(super) fn age_overlay_style_run(&mut self, age: Duration) {
        if let Some((_, _, _, at)) = &mut self.last_overlay_style {
            *at -= age;
        }
    }
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
        let merge = restyled.is_some_and(|id| self.history.continues_restyle(chart_id, id, now));
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
            self.layout.drawings_edited();
            // A restyle that follows another of the same drawing moments ago is part of it.
            self.history.record_drawing_edit(
                chart_id,
                (!merge).then_some(Snapshot::Drawings(before)),
                restyled.map(|id| (id, now)),
            );
        } else if restyled.is_none() {
            self.history.end_restyle_run();
        }
    }

    /// Puts the view a zoom left on the undo stack: Zoom In and Zoom out are steps like any edit,
    /// and a new step ends the redo line.
    pub(super) fn remember_view(&mut self, chart_id: u64, step: ViewStep, direction: ViewMove) {
        self.history
            .record(chart_id, Snapshot::View { step, direction });
    }

    /// Forgets the views `chart_id` could be put back to (all charts when `None`): they were
    /// positions in a series that is no longer there.
    pub(super) fn forget_views(&mut self, chart_id: Option<u64>) {
        let gone = |entry: &UndoEntry| {
            matches!(entry.before, Snapshot::View { .. })
                && chart_id.is_none_or(|id| entry.chart_id == id)
        };
        self.history.forget(gone);
    }

    pub(super) fn undo(&mut self, ctx: &egui::Context) {
        self.step_through(true, ctx);
    }

    pub(super) fn redo(&mut self, ctx: &egui::Context) {
        self.step_through(false, ctx);
    }

    /// Shows `item` on `chart_id` as one undo step: the market it had comes back, with the
    /// indicators a market that takes none removes. Returns whether the chart changed.
    pub(super) fn replace_chart_market(
        &mut self,
        chart_id: u64,
        item: &WatchlistInstrument,
        ctx: &egui::Context,
    ) -> bool {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return false;
        };
        let before = Snapshot::All(vec![
            Snapshot::Market(chart.market_state()),
            Snapshot::indicators_of(chart),
        ]);
        if !self.switch_chart(chart_id, item, ctx) {
            return false;
        }
        self.history.record(chart_id, before);
        true
    }

    /// One press of Undo (or Redo): the newest step moves from one stack to the other. A view of
    /// bars the chart no longer shows cannot be put back, so it is dropped and the step beneath it
    /// is the one this press takes: a press that did nothing would be worse than skipping.
    fn step_through(&mut self, undoing: bool, ctx: &egui::Context) {
        self.history.end_restyle_run();
        loop {
            let entry = self.history.pop(undoing);
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
            let reloads = entry.before.changes_market();
            let Some(after) = entry.before.swap_into(chart) else {
                continue;
            };
            if layout {
                self.layout.drawings_edited();
            }
            let moved = UndoEntry {
                chart_id: entry.chart_id,
                before: after,
            };
            self.history.push_inverse(undoing, moved);
            if reloads {
                self.market_changed(entry.chart_id, ctx);
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == entry.chart_id) {
                    let item = self.favorite_item(chart);
                    self.announce_market(&item);
                }
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

        workspace.undo(&egui::Context::default());
        assert_eq!(drawing_ids(&workspace, chart), vec![1, 2]);
        assert_eq!(
            workspace.charts[0].drawings[0].anchors()[0].time_ms,
            900,
            "the move came back as one step"
        );
        workspace.undo(&egui::Context::default());
        assert_eq!(workspace.charts[0].drawings[0].anchors()[0].time_ms, 100);
        workspace.undo(&egui::Context::default());
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);
        workspace.undo(&egui::Context::default());
        assert_eq!(
            drawing_ids(&workspace, chart),
            vec![1],
            "nothing earlier to undo"
        );

        for _ in 0..3 {
            workspace.redo(&egui::Context::default());
        }
        assert_eq!(drawing_ids(&workspace, chart), vec![2]);
        // A new edit clears what could be redone.
        workspace.undo(&egui::Context::default());
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(3, 0))]);
        workspace.redo(&egui::Context::default());
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
        assert!(workspace.history.undo.is_empty(), "no entries");
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
        assert_eq!(workspace.history.undo.len(), 1);
        assert_eq!(
            workspace.charts[0].drawings[0]
                .style()
                .unwrap()
                .stroke
                .width,
            6.0
        );
        workspace.undo(&egui::Context::default());
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
        assert_eq!(workspace.history.undo.len(), 1);
        // Another edit in between ends the burst.
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 300))]);
        workspace.apply_drawing_commands(chart, vec![restyle(10.0)]);
        assert_eq!(workspace.history.undo.len(), 3);
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
        let version = workspace.layout.drawings_version();
        zoomed(&mut workspace, chart, 40, 30.0);

        workspace.undo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 0.0, "back where it was");
        workspace.redo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 30.0, "zoomed again");
        workspace.undo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert_eq!(
            workspace.layout.drawings_version(),
            version,
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
        workspace.undo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(
            drawing_ids(&workspace, chart).is_empty(),
            "the zoom came last"
        );
        workspace.undo(&egui::Context::default());
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);

        // Zooming again after an undo drops what could have been redone.
        zoomed(&mut workspace, chart, 60, 10.0);
        assert!(workspace.history.redo.is_empty());
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
        assert_eq!(workspace.history.undo.len(), 2, "Zoom out is a step too");

        // Undoing it zooms back in, and Zoom out is offered again.
        workspace.undo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 30.0);
        assert!(workspace.charts[0].chart.can_zoom_out());
        // Undoing the zoom itself leaves nothing to zoom out of.
        workspace.undo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(!workspace.charts[0].chart.can_zoom_out());
        // Redo brings both back, history included.
        workspace.redo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 30.0);
        assert!(workspace.charts[0].chart.can_zoom_out());
        workspace.redo(&egui::Context::default());
        assert_eq!(chart_offset(&workspace, chart), 0.0);
        assert!(!workspace.charts[0].chart.can_zoom_out());
    }

    #[test]
    fn undoing_past_the_history_cap_never_offers_a_forward_zoom_out() {
        let mut workspace = empty_workspace(9);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let cap = crate::model::chart::viewport::zoom::MAX_BACK;
        let zooms = cap + 2;
        for i in 1..=zooms {
            zoomed(&mut workspace, chart, 120 - 10 * i, 10.0 * i as f64);
        }
        assert_eq!(history(&workspace), cap, "the history keeps the newest few");

        // Undo every zoom: the history empties as the zooms it lists are undone and stays empty
        // for the ones it had already forgotten.
        for undone in 1..=zooms {
            workspace.undo(&egui::Context::default());
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
            workspace.redo(&egui::Context::default());
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

        workspace.undo(&egui::Context::default());
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
        assert!(workspace.history.undo.is_empty());
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
        assert_eq!(workspace.history.undo.len(), 2);
        assert!(workspace.history.undo.iter().any(|e| e.chart_id == second));
        workspace.forget_views(None);
        assert_eq!(workspace.history.undo.len(), 1);
        assert!(matches!(
            workspace.history.undo[0].before,
            Snapshot::Drawings(_)
        ));
    }

    fn replace(
        workspace: &mut ChartWorkspace,
        chart: u64,
        source: crate::model::DataSource,
        symbol: &str,
        ctx: &egui::Context,
    ) -> bool {
        let item = WatchlistInstrument::from_market(source, symbol, symbol);
        workspace.replace_chart_market(chart, &item, ctx)
    }

    fn market(workspace: &ChartWorkspace) -> (crate::model::DataSource, String) {
        (
            workspace.charts[0].source,
            workspace.charts[0].symbol.clone(),
        )
    }

    #[test]
    fn replacing_a_chart_is_one_undo_step_that_brings_back_its_market() {
        use crate::model::DataSource;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(11);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.charts[0].indicators.show_rsi = true;

        assert!(replace(
            &mut workspace,
            chart,
            DataSource::Coinbase,
            "ETH-USD",
            &ctx
        ));
        assert_eq!(market(&workspace), (DataSource::Coinbase, "ETH-USD".into()));
        assert!(
            workspace.charts[0].indicators.show_rsi,
            "a market keeps them"
        );
        assert_eq!(workspace.history.undo.len(), 1);

        workspace.undo(&ctx);
        assert_eq!(market(&workspace), (DataSource::Binance, "BTCUSDT".into()));
        assert_eq!(drawing_ids(&workspace, chart), vec![1]);
        workspace.redo(&ctx);
        assert_eq!(market(&workspace), (DataSource::Coinbase, "ETH-USD".into()));
        workspace.undo(&ctx);
        assert_eq!(market(&workspace), (DataSource::Binance, "BTCUSDT".into()));
    }

    #[test]
    fn a_market_that_takes_no_indicators_removes_them_and_undo_brings_them_back() {
        use crate::model::DataSource;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(12);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.charts[0].indicators.show_rsi = true;
        workspace.charts[0]
            .chart
            .add_pine_instance(None, "Mine", "plot(close)");

        for (source, symbol) in [
            (DataSource::Polymarket, "some-market"),
            (DataSource::CoinMarketCap, "fear-greed"),
        ] {
            assert!(replace(&mut workspace, chart, source, symbol, &ctx));
            assert_eq!(workspace.charts[0].indicators, IndicatorConfig::default());
            assert!(workspace.charts[0].chart.pine_instances().is_empty());
            assert_eq!(workspace.history.undo.len(), 1, "one step for both");

            workspace.undo(&ctx);
            assert_eq!(market(&workspace), (DataSource::Binance, "BTCUSDT".into()));
            assert!(workspace.charts[0].indicators.show_rsi);
            assert_eq!(workspace.charts[0].chart.pine_instances().len(), 1);
            workspace.redo(&ctx);
            assert!(workspace.charts[0].chart.pine_instances().is_empty());
            workspace.undo(&ctx);
        }
    }

    #[test]
    fn showing_the_same_market_or_following_a_link_leaves_no_step() {
        use crate::model::DataSource;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(13);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        assert!(!replace(
            &mut workspace,
            chart,
            DataSource::Binance,
            "BTCUSDT",
            &ctx
        ));
        assert!(!replace(
            &mut workspace,
            999,
            DataSource::Binance,
            "ETHUSDT",
            &ctx
        ));
        assert!(workspace.history.undo.is_empty());

        // Tab Linking moves the market without it being this tab's own edit.
        let item = WatchlistInstrument::from_market(DataSource::Binance, "ETHUSDT", "ETHUSDT");
        assert!(workspace.switch_chart(chart, &item, &ctx));
        assert!(workspace.history.undo.is_empty());
    }

    #[test]
    fn a_new_replacement_ends_the_redo_line() {
        use crate::model::DataSource;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(14);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        replace(&mut workspace, chart, DataSource::Binance, "ETHUSDT", &ctx);
        workspace.undo(&ctx);
        assert_eq!(workspace.history.redo.len(), 1);
        replace(&mut workspace, chart, DataSource::Binance, "SOLUSDT", &ctx);
        assert!(workspace.history.redo.is_empty());
        workspace.undo(&ctx);
        assert_eq!(market(&workspace), (DataSource::Binance, "BTCUSDT".into()));
    }
}
