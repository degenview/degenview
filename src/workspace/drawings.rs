//! Drawings on a chart: what is saved, the properties dialog, templates and defaults, locking, and merging another Mac's edits.

use super::ChartWorkspace;
use crate::model::{Drawing, DrawingCommand};
use crate::store::drawing_template::{DrawingPrefs, DrawingTemplateStore};
use crate::ui::drawing_properties::{self, DrawingProperties};
use eframe::egui;
use std::time::Instant;

/// Rulers are transient measurements: never persisted, never in undo history.
pub(super) fn is_ruler(drawing: &Drawing) -> bool {
    drawing.is_ruler()
}

/// The drawings that are saved: rulers are never part of a layout.
pub(super) fn plain_drawings(drawings: &[Drawing]) -> Vec<Drawing> {
    drawings.iter().filter(|d| !is_ruler(d)).cloned().collect()
}

/// Three-way merge of one chart's drawings by id. `base` is what this tab last saved or opened,
/// `local` what it shows now and `remote` what the layout holds after another Mac's edit. A change
/// on one side wins over no change on the other; when both changed the same drawing, this tab's
/// version wins and is saved over the other (last writer). Local order is kept, new drawings
/// from the other Mac follow in their order.
///
/// A drawing this version cannot read (a tool from a newer build) is matched by the id it was
/// stored with, since `Drawing::id` is 0 for all of them; one with no readable id is matched by
/// its content.
pub(super) fn merge_remote_drawings(
    base: &[Drawing],
    local: &[Drawing],
    remote: &[Drawing],
) -> Vec<Drawing> {
    let same = |a: &Drawing, b: &Drawing| match (a.stored_id(), b.stored_id()) {
        (0, 0) => a == b,
        (left, right) => left == right,
    };
    let find = |list: &[Drawing], like: &Drawing| list.iter().find(|d| same(d, like)).cloned();
    let mut merged = Vec::new();
    for drawing in local {
        let (old, theirs) = (find(base, drawing), find(remote, drawing));
        let local_unchanged = old.as_ref() == Some(drawing);
        match theirs {
            Some(theirs) if local_unchanged => merged.push(theirs),
            // Removed on the other Mac and untouched here.
            None if local_unchanged => {}
            _ => merged.push(drawing.clone()),
        }
    }
    for drawing in remote {
        if find(local, drawing).is_none() && find(base, drawing).is_none() {
            merged.push(drawing.clone());
        }
    }
    merged
}

/// Drawings the strip's lock and hide controls act on: everything but the transient ruler and
/// rows this version cannot read.
pub(super) fn is_lockable(drawing: &Drawing) -> bool {
    !is_ruler(drawing) && drawing.tool().is_some()
}

impl ChartWorkspace {
    /// Applies drawing commands an MCP tool planned from [`Self::chart_data`] through the same
    /// path as the pointer, so they are one undo step and are saved like any other edit.
    /// `next_free_id` is the first id the commands left unused; the chart's counter moves past it
    /// (0 when the commands mint none).
    pub fn apply_planned_drawings(
        &mut self,
        chart_id: u64,
        commands: Vec<DrawingCommand>,
        next_free_id: u64,
    ) {
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id && chart.is_market())
        {
            chart.next_drawing_id = chart.next_drawing_id.max(next_free_id);
        }
        self.apply_drawing_commands(chart_id, commands);
    }

    /// The saved drawing templates and defaults. The first call opens them and takes the strip's
    /// remembered options (magnet, keep drawing, the grouped tile's tool) from them.
    pub(super) fn drawing_templates(&mut self) -> &mut DrawingTemplateStore {
        if self.drawing_templates.is_none() {
            let templates = DrawingTemplateStore::new(self.store.clone());
            let prefs = templates.prefs();
            self.magnet = prefs.magnet;
            self.keep_drawing = prefs.keep_drawing;
            self.group_tool = prefs.group_tool;
            self.line_tool = prefs.line_tool;
            self.note_tool = prefs.note_tool;
            self.pattern_tool = prefs.pattern_tool;
            self.forecast_tool = prefs.forecast_tool;
            self.fib_tool = prefs.fib_tool;
            self.saved_prefs = prefs;
            self.drawing_templates = Some(templates);
        }
        let templates = self.drawing_templates.as_mut().expect("opened just above");
        templates.refresh(Instant::now());
        templates
    }

    /// Saves the strip's options when they changed.
    pub(super) fn save_drawing_prefs(&mut self) {
        let prefs = DrawingPrefs {
            magnet: self.magnet,
            keep_drawing: self.keep_drawing,
            group_tool: self.group_tool,
            line_tool: self.line_tool,
            note_tool: self.note_tool,
            pattern_tool: self.pattern_tool,
            forecast_tool: self.forecast_tool,
            fib_tool: self.fib_tool,
        };
        if prefs != self.saved_prefs {
            self.saved_prefs = prefs;
            self.drawing_templates().set_prefs(&prefs);
        }
    }

    /// Opens the properties dialog of one drawing (replacing any other that was open).
    pub(super) fn open_drawing_properties(&mut self, chart_id: u64, drawing_id: u64) {
        // One dialog at a time: a request while one is open (a Return meant for it, say) must
        // not replace the edits in it.
        if self.drawing_props.is_some() {
            return;
        }
        self.drawing_props = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| chart.drawings.iter().find(|d| d.id() == drawing_id))
            .filter(|drawing| !drawing.locked())
            .and_then(|drawing| DrawingProperties::new(chart_id, drawing));
    }

    /// The alert behind "Remove all drawings".
    pub(super) fn confirm_drawings_removal(&mut self, ctx: &egui::Context) {
        let Some(chart_id) = self.confirm_remove_drawings else {
            return;
        };
        let count = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .map_or(0, |chart| {
                chart.drawings.iter().filter(|d| is_lockable(d)).count()
            });
        if count == 0 {
            self.confirm_remove_drawings = None;
            return;
        }
        let message = format!(
            "{count} drawing{} will be removed from this chart. You can undo it.",
            if count == 1 { "" } else { "s" }
        );
        let result = crate::ui::kit::confirm(
            ctx,
            egui::Id::new("remove-drawings"),
            &crate::ui::kit::Confirm {
                icon: crate::ui::icons::Icon::Trash,
                title: "Remove all drawings?",
                message: &message,
                destructive: "Remove All",
            },
        );
        match result {
            Some(crate::ui::kit::ConfirmOutcome::Confirmed) => {
                self.confirm_remove_drawings = None;
                let commands: Vec<DrawingCommand> = self
                    .charts
                    .iter()
                    .find(|chart| chart.id == chart_id)
                    .map(|chart| {
                        chart
                            .drawings
                            .iter()
                            .filter(|d| is_lockable(d))
                            .map(|d| DrawingCommand::Remove(d.id()))
                            .collect()
                    })
                    .unwrap_or_default();
                self.apply_drawing_commands(chart_id, commands);
                self.selected_drawing = None;
            }
            Some(crate::ui::kit::ConfirmOutcome::Cancelled) => self.confirm_remove_drawings = None,
            None => {}
        }
    }

    /// Draws the drawing's properties dialog, if one is up, and applies what it returns. The
    /// dialog goes away with its drawing or its chart.
    pub(super) fn show_drawing_properties(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.drawing_props.take() else {
            return;
        };
        let (chart_id, drawing_id) = (dialog.chart_id, dialog.draft().id());
        let exists = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| chart.drawings.iter().any(|d| d.id() == drawing_id));
        if !exists {
            return;
        }
        let templates = self.drawing_templates();
        let outcome = dialog.show(ctx, templates);
        if dialog.take_replace_request() {
            self.replace_picture_request(ctx, chart_id, drawing_id);
        }
        match outcome {
            drawing_properties::Outcome::Open => self.drawing_props = Some(dialog),
            drawing_properties::Outcome::Cancel => {}
            drawing_properties::Outcome::Apply(drawing) => {
                self.apply_drawing_commands(
                    chart_id,
                    vec![DrawingCommand::Replace {
                        id: drawing_id,
                        drawing: *drawing,
                    }],
                );
            }
        }
    }

    /// Whether any chart has a drawing to lock, and whether every one of them is locked.
    pub(super) fn drawing_lock_state(&self) -> (bool, bool) {
        let mut any = false;
        let mut all_locked = true;
        for chart in self.charts.iter().filter(|chart| chart.is_market()) {
            for drawing in chart.drawings.iter().filter(|d| is_lockable(d)) {
                any = true;
                all_locked &= drawing.locked();
            }
        }
        (any, any && all_locked)
    }

    /// Locks or unlocks every drawing on every chart of this tab. One undo step per chart,
    /// since history is kept per chart.
    pub(super) fn set_all_locked(&mut self, locked: bool) {
        let ids: Vec<u64> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| chart.id)
            .collect();
        for id in ids {
            let Some(chart) = self.charts.iter().find(|chart| chart.id == id) else {
                continue;
            };
            let commands: Vec<DrawingCommand> = chart
                .drawings
                .iter()
                .filter(|d| is_lockable(d) && d.locked() != locked)
                .map(|drawing| {
                    let mut changed = drawing.clone();
                    changed.set_locked(locked);
                    DrawingCommand::Replace {
                        id: drawing.id(),
                        drawing: changed,
                    }
                })
                .collect();
            self.apply_drawing_commands(id, commands);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::PersistedWorkspace;
    use crate::workspace::test_support::{
        candle, chart_with_drawings, drawing_ids, empty_workspace, rectangle, unreadable,
    };

    #[test]
    fn drawings_planned_for_mcp_are_one_undo_step_and_keep_their_style_on_save() {
        use crate::mcp::drawings::{AnchorSpec, McpDrawingTool, NewDrawing, PriceField, PriceSpec};
        use crate::model::chart_type::LineDash;
        let mut workspace = empty_workspace(9);
        assert!(workspace.chart_data(404).is_none());
        let chart_id = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        let minute = 60_000;
        workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .unwrap()
            .candles = (0..20)
            .map(|index| candle(1_000_000 + index * minute, 100.0 + index as f64))
            .collect();
        let data = workspace.chart_data(chart_id).expect("market chart");
        assert_eq!(data.candles.len(), 20);
        // The counter lags the stored drawing (the fixture bypassed it); minting still avoids it.
        let first_id = crate::mcp::chart_data::next_free_id(data.drawings, data.next_drawing_id);
        assert_eq!(first_id, 2);
        let anchor = |index: i64, price: PriceSpec| AnchorSpec {
            time_ms: 1_000_000 + index * minute + 5,
            price,
        };
        let mut line = NewDrawing {
            tool: McpDrawingTool::TrendLine,
            a: anchor(2, PriceSpec::Field(PriceField::Low)),
            b: anchor(15, PriceSpec::Field(PriceField::High)),
            patch: Default::default(),
        };
        line.patch.dash = Some(LineDash::Dotted);
        line.patch.extend_right = Some(true);
        line.patch.color = Some([255, 193, 7, 255]);
        let zone = NewDrawing {
            tool: McpDrawingTool::Rectangle,
            a: anchor(4, PriceSpec::Value(103.0)),
            b: anchor(9, PriceSpec::Value(108.0)),
            patch: Default::default(),
        };
        let planned =
            crate::mcp::chart_data::plan_add(data.drawings, data.candles, first_id, &[line, zone])
                .unwrap();
        let commands: Vec<DrawingCommand> =
            planned.iter().cloned().map(DrawingCommand::Add).collect();
        workspace.apply_planned_drawings(chart_id, commands, first_id + 2);

        assert_eq!(drawing_ids(&workspace, chart_id), [1, 2, 3]);
        assert_eq!(workspace.chart_data(chart_id).unwrap().next_drawing_id, 4);
        // Times snapped to the bars' opens, prices read from the bars.
        let anchors = planned[0].anchors();
        assert_eq!(anchors[0].time_ms, 1_000_000 + 2 * minute);
        assert_eq!(anchors[1].price, 115.0);

        // It is saved with the session, style and all, and reads back the same.
        let saved = workspace.persisted();
        let json = serde_json::to_string(&saved).unwrap();
        let reread: PersistedWorkspace = serde_json::from_str(&json).unwrap();
        let restored = reread.charts[0].drawings.clone();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored[1], planned[0]);
        assert_eq!(restored[1].style().unwrap().stroke.dash, LineDash::Dotted);
        assert!(restored[1].style().unwrap().extend_right);

        // One undo takes both away, one redo brings both back.
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, chart_id), [1]);
        workspace.redo();
        assert_eq!(drawing_ids(&workspace, chart_id), [1, 2, 3]);

        // A chart that is not there changes nothing.
        workspace.apply_planned_drawings(404, vec![DrawingCommand::Remove(1)], 50);
        assert_eq!(drawing_ids(&workspace, chart_id), [1, 2, 3]);
    }

    #[test]
    fn merging_remote_drawings_keeps_each_sides_changes_and_local_wins_a_tie() {
        let d = |id: u64, at: i64| rectangle(id, at);
        let base = [d(1, 100), d(2, 200), d(3, 300)];
        // Local moved 1 and added 4. Remote deleted 2, moved 3 and added 5.
        let local = [d(1, 111), d(2, 200), d(3, 300), d(4, 400)];
        let remote = [d(1, 100), d(3, 333), d(5, 500)];
        let merged = merge_remote_drawings(&base, &local, &remote);
        assert_eq!(
            merged,
            [d(1, 111), d(3, 333), d(4, 400), d(5, 500)],
            "local move kept, remote delete and move taken, both additions kept"
        );
        // Both changed the same drawing: this tab's version wins.
        let merged = merge_remote_drawings(&[d(1, 100)], &[d(1, 111)], &[d(1, 122)]);
        assert_eq!(merged, [d(1, 111)]);
        // A drawing deleted here stays deleted even if the other Mac moved it.
        let merged = merge_remote_drawings(&[d(1, 100)], &[], &[d(1, 122)]);
        assert!(merged.is_empty());
    }

    #[test]
    fn unreadable_drawings_merge_by_their_stored_id_not_as_one_drawing() {
        let d = |id: u64, at: i64| rectangle(id, at);
        // The other Mac drew a tool this build cannot read, next to one already held.
        let merged = merge_remote_drawings(
            &[d(1, 100), unreadable(50)],
            &[d(1, 100), unreadable(50)],
            &[d(1, 100), unreadable(50), unreadable(51)],
        );
        assert_eq!(merged, [d(1, 100), unreadable(50), unreadable(51)]);
        // The other Mac deleted one of two: the other must not be duplicated or lost.
        let merged = merge_remote_drawings(
            &[unreadable(50), unreadable(51)],
            &[unreadable(50), unreadable(51)],
            &[unreadable(51)],
        );
        assert_eq!(merged, [unreadable(51)]);
        // Edited here, untouched there: this tab's version stays.
        let merged = merge_remote_drawings(
            &[d(1, 100), unreadable(50)],
            &[d(1, 111), unreadable(50)],
            &[d(1, 100), unreadable(50)],
        );
        assert_eq!(merged, [d(1, 111), unreadable(50)]);
    }

    #[test]
    fn charts_keep_their_own_drawings() {
        let mut workspace = empty_workspace(11);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[7]);
        workspace.apply_drawing_commands(first, vec![DrawingCommand::Remove(1)]);
        assert_eq!(drawing_ids(&workspace, first), vec![2]);
        assert_eq!(drawing_ids(&workspace, second), vec![7]);
        // Undo goes to the chart the edit was made on.
        workspace.apply_drawing_commands(second, vec![DrawingCommand::Add(rectangle(8, 0))]);
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, second), vec![7]);
        assert_eq!(drawing_ids(&workspace, first), vec![2]);
        workspace.undo();
        assert_eq!(drawing_ids(&workspace, first), vec![1, 2]);
    }

    #[test]
    fn lock_all_and_unlock_all_act_on_every_chart_and_report_their_state() {
        let mut workspace = empty_workspace(12);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[3]);
        assert_eq!(workspace.drawing_lock_state(), (true, false));
        workspace.set_all_locked(true);
        assert_eq!(workspace.drawing_lock_state(), (true, true));
        assert!(
            workspace
                .charts
                .iter()
                .all(|c| c.drawings.iter().all(Drawing::locked))
        );
        // One undo step per chart, since history is per chart.
        assert_eq!(workspace.undo.len(), 2);
        workspace.undo();
        workspace.undo();
        assert_eq!(workspace.drawing_lock_state(), (true, false));
        workspace.set_all_locked(true);
        workspace.set_all_locked(false);
        assert_eq!(workspace.drawing_lock_state(), (true, false));
        let _ = (first, second);
        // Locking nothing is not an edit.
        let mut empty = empty_workspace(13);
        chart_with_drawings(&mut empty, "BTCUSDT", &[]);
        empty.set_all_locked(true);
        assert!(empty.undo.is_empty());
        assert_eq!(empty.drawing_lock_state(), (false, false));
    }

    #[test]
    fn one_properties_dialog_at_a_time_and_none_for_a_locked_drawing() {
        let mut workspace = empty_workspace(15);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        let mut locked = rectangle(2, 200);
        locked.set_locked(true);
        workspace.apply_drawing_commands(
            chart,
            vec![DrawingCommand::Replace {
                id: 2,
                drawing: locked,
            }],
        );
        workspace.open_drawing_properties(chart, 2);
        assert!(
            workspace.drawing_props.is_none(),
            "a locked drawing is not edited"
        );
        workspace.open_drawing_properties(chart, 1);
        let opened = workspace
            .drawing_props
            .as_ref()
            .map(|dialog| dialog.draft().id());
        assert_eq!(opened, Some(1));
        // A second request (a Return meant for the dialog) leaves the open one, and its edits.
        workspace
            .drawing_props
            .as_mut()
            .expect("dialog")
            .set_draft_text_for_test("edited");
        workspace.open_drawing_properties(chart, 1);
        assert_eq!(
            workspace
                .drawing_props
                .as_ref()
                .map(|dialog| dialog.draft().text().to_owned()),
            Some("edited".to_owned())
        );
    }
}
