//! Saved layouts: the workspace's persisted form, its dirty state, and opening, saving, renaming and deleting layouts.

use super::autosave::AutosaveState;
use super::chart::Chart;
use super::drawings::{is_ruler, merge_remote_drawings, plain_drawings};
use super::undo::{Snapshot, UndoEntry};
use super::{ChartWorkspace, PersistedChart, PersistedWorkspace};
use crate::model::watchlist::{new_id, now_ms};
use crate::model::{ChartColumn, ChartConfig, Drawing, LayoutSnapshot, SavedView};
use crate::ui::layout_prompts::{self, NameOutcome, NamePrompt, UnsavedChoice};
use eframe::egui;

impl ChartWorkspace {
    pub fn persisted(&self) -> PersistedWorkspace {
        PersistedWorkspace {
            id: self.id,
            timeframe: self.timeframe,
            watchlist_visible: self.watchlist_visible,
            charts: self
                .charts
                .iter()
                .map(|chart| PersistedChart {
                    id: chart.id,
                    symbol: chart.symbol.clone(),
                    source: chart.source,
                    log_scale: chart.scale_mode.is_logarithmic(),
                    scale_mode: Some(chart.scale_mode),
                    y_zoom: chart.chart.y_zoom(),
                    indicators: chart.indicators,
                    appearance: chart.chart.appearance(),
                    drawings: chart
                        .drawings
                        .iter()
                        .filter(|drawing| !is_ruler(drawing))
                        .cloned()
                        .collect(),
                    pine_instances: chart.chart.pine_instances().to_vec(),
                    candle_count: chart.chart.visible_candles() as i64,
                    viewport: chart.chart.saved_viewport(),
                    title: chart.chart.title().map(str::to_owned),
                    pm_series: chart.pm_series.clone(),
                    image_url: chart.image_url.clone(),
                    card: chart.card,
                })
                .collect(),
            columns: self.columns.clone(),
            focused: self.focused,
            sync: self.sync,
        }
    }

    pub fn layout_snapshot(&self) -> LayoutSnapshot {
        LayoutSnapshot {
            timeframe: self.timeframe,
            configs: self
                .charts
                .iter()
                .map(|chart| ChartConfig {
                    id: chart.id,
                    symbol: chart.symbol.clone(),
                    source: chart.source,
                    scale_mode: chart.scale_mode,
                    appearance: chart.chart.appearance(),
                    card: chart.card,
                })
                .collect(),
            columns: self
                .columns
                .iter()
                .map(|column| column.chart_ids.clone())
                .collect(),
            sync: self.sync,
        }
    }

    /// Whether the layout differs from the last saved fingerprint. Zoom, focus,
    /// and watchlist edits do not count.
    pub fn is_dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|baseline| *baseline != self.layout_snapshot())
    }

    pub fn mark_clean(&mut self) {
        self.set_baseline();
    }

    /// Records the layout as saved: its fingerprint, and the drawings it holds.
    pub(super) fn set_baseline(&mut self) {
        self.baseline = Some(self.layout_snapshot());
        self.saved_drawings_version = self.drawings_version;
        self.saved_drawings = self
            .charts
            .iter()
            .map(|chart| (chart.id, plain_drawings(&chart.drawings)))
            .collect();
    }

    /// Another Mac changed the layout this tab has open and iCloud Sync stored the merged result.
    /// Brings its drawings into the live charts without losing what was drawn here since the last
    /// save, and without writing anything back: without this the tab keeps showing (and with
    /// Autosave later saves) the old drawings, which would delete the other Mac's from iCloud.
    pub fn adopt_remote_layout(&mut self) {
        let Some(view) = self.active_view() else {
            return;
        };
        let was_dirty = self.drawings_dirty();
        let mut changed = false;
        let mut diverged = false;
        for saved in &view.charts {
            let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == saved.id) else {
                continue;
            };
            let remote = plain_drawings(&saved.drawings);
            let base = self
                .saved_drawings
                .get(&chart.id)
                .cloned()
                .unwrap_or_default();
            if base == remote {
                continue;
            }
            let local = plain_drawings(&chart.drawings);
            let merged = merge_remote_drawings(&base, &local, &remote);
            diverged |= merged != remote;
            if merged != local {
                let rulers: Vec<Drawing> = chart
                    .drawings
                    .iter()
                    .filter(|d| is_ruler(d))
                    .cloned()
                    .collect();
                if let Some(highest) = merged.iter().map(Drawing::stored_id).max() {
                    chart.next_drawing_id = chart.next_drawing_id.max(highest + 1);
                }
                chart.drawings = merged;
                chart.drawings.extend(rulers);
                // An undo step would put back drawings that were since added or removed elsewhere.
                let id = chart.id;
                let drawn = |entry: &UndoEntry| {
                    entry.chart_id == id && matches!(entry.before, Snapshot::Drawings(_))
                };
                self.undo.retain(|entry| !drawn(entry));
                self.redo.retain(|entry| !drawn(entry));
                changed = true;
            }
            self.saved_drawings.insert(chart.id, remote);
        }
        if changed {
            self.drawings_version += 1;
            // Nothing here beyond what is already stored: no save is due.
            if !was_dirty && !diverged {
                self.saved_drawings_version = self.drawings_version;
            }
        }
    }

    /// Drawings changed since the layout was last saved. The layout's fingerprint leaves drawings
    /// out (a drag must not mark a layout as edited), so an Autosave layout watches this too:
    /// without it a drawing would only reach the saved layout, and other Macs, by accident.
    pub(super) fn drawings_dirty(&self) -> bool {
        self.drawings_version != self.saved_drawings_version
    }

    pub(super) fn load_saved_views(&self) -> Vec<SavedView> {
        self.store
            .lock()
            .map(|store| store.saved_views())
            .unwrap_or_default()
    }

    pub(super) fn active_view(&self) -> Option<SavedView> {
        let id = self.active_view_id?;
        self.load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
    }

    /// The toolbar menu's "Save": save in place, or ask for a name the first time.
    pub(super) fn request_save(&mut self, ctx: &egui::Context) {
        if self.active_view_id.is_some() {
            self.save_active(true);
        } else {
            self.open_name_prompt(NamePrompt::Save { then_open: None }, ctx);
        }
    }

    /// The Save shortcut (⌘S / Ctrl+S) and any app-level save.
    pub fn save_layout(&mut self, ctx: &egui::Context) {
        self.request_save(ctx);
    }

    pub(super) fn open_name_prompt(&mut self, prompt: NamePrompt, ctx: &egui::Context) {
        let base = self.active_view_name.clone();
        self.prompt_name = match prompt {
            NamePrompt::Copy => format!("{} copy", base.as_deref().unwrap_or("My Chart")),
            NamePrompt::Rename => base.unwrap_or_default(),
            NamePrompt::Save { .. } => String::new(),
        };
        self.prompt_focus = true;
        self.layout_prompt = Some(prompt);
        ctx.request_repaint();
    }

    fn close_name_prompt(&mut self) {
        self.layout_prompt = None;
        self.prompt_name.clear();
    }

    /// Writes the live layout over the open one, keeping its autosave choice. A failure leaves
    /// the layout dirty; `report` says whether to interrupt with an alert (autosave only flags it).
    pub(super) fn save_active(&mut self, report: bool) -> bool {
        let Some(existing) = self.active_view() else {
            return false;
        };
        let view = self.build_view(
            existing.id,
            existing.name.clone(),
            existing.created_at_ms,
            Some(now_ms()),
            existing.autosave,
        );
        match self.upsert_view(view) {
            Ok(()) => {
                self.mark_clean();
                self.autosave = AutosaveState::default();
                true
            }
            Err(error) => {
                if report {
                    self.layout_error = Some(format!("Couldn’t save “{}”: {error}", existing.name));
                }
                false
            }
        }
    }

    /// First save of an unnamed layout (or a copy): mints a new identity.
    fn save_named(&mut self, name: String, autosave: Option<bool>) -> bool {
        let view = self.build_view(new_id(), name, now_ms(), Some(now_ms()), autosave);
        let (id, name) = (view.id, view.name.clone());
        match self.upsert_view(view) {
            Ok(()) => {
                self.active_view_id = Some(id);
                self.active_view_name = Some(name);
                self.mark_clean();
                self.autosave = AutosaveState::default();
                true
            }
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t save the layout: {error}"));
                false
            }
        }
    }

    /// Captures the live layout into a `SavedView` (without persisting it).
    fn build_view(
        &self,
        id: u64,
        name: String,
        created_at_ms: i64,
        last_opened_at_ms: Option<i64>,
        autosave: Option<bool>,
    ) -> SavedView {
        let persisted = self.persisted();
        SavedView {
            id,
            name,
            timeframe: persisted.timeframe,
            created_at_ms,
            charts: persisted.charts,
            columns: Some(persisted.columns),
            candle_count: self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map_or(0, |chart| chart.chart.visible_candles() as i64),
            last_opened_at_ms,
            autosave,
            sync: persisted.sync,
        }
    }

    fn upsert_view(&self, view: SavedView) -> Result<(), String> {
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let mut views = store.saved_views();
        match views.iter_mut().find(|existing| existing.id == view.id) {
            // Replace in place so the layout keeps its position.
            Some(existing) => *existing = view,
            None => views.push(view),
        }
        store
            .replace_saved_views(&views)
            .map_err(|error| error.to_string())
    }

    /// Edits the open layout's stored row.
    pub(super) fn update_active_view(
        &mut self,
        edit: impl FnOnce(&mut SavedView),
    ) -> Result<(), String> {
        let Some(id) = self.active_view_id else {
            return Ok(());
        };
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let mut views = store.saved_views();
        if let Some(view) = views.iter_mut().find(|view| view.id == id) {
            edit(view);
            store
                .replace_saved_views(&views)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// The menu's "open": write an autosave layout first, ask over unsaved changes, or apply.
    pub(super) fn request_open_saved_view(&mut self, id: u64, ctx: &egui::Context) {
        if self.autosave_enabled() {
            if self.flush_autosave() {
                self.apply_saved_view_by_id(id, ctx);
            }
        } else if self.is_dirty() {
            self.pending_open_id = Some(id);
            ctx.request_repaint();
        } else {
            self.apply_saved_view_by_id(id, ctx);
        }
    }

    fn apply_saved_view_by_id(&mut self, id: u64, ctx: &egui::Context) {
        let Some(view) = self
            .load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
        else {
            return;
        };
        self.apply_saved_view(&view, ctx);
    }

    /// Copies the current layout (unsaved changes included) under a new identity; the original
    /// stays as it was saved.
    fn copy_layout(&mut self, name: String) {
        let autosave = self.active_view().and_then(|view| view.autosave);
        self.save_named(name, autosave);
    }

    fn rename_layout(&mut self, name: String) {
        if self.active_view_id.is_none() {
            self.save_named(name, None);
            return;
        }
        let renamed = name.clone();
        match self.update_active_view(|view| view.name = renamed) {
            Ok(()) => self.active_view_name = Some(name),
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t rename the layout: {error}"));
            }
        }
    }

    pub(super) fn delete_saved_view(&mut self, id: u64) {
        let result = self
            .store
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|store| {
                let mut views = store.saved_views();
                views.retain(|view| view.id != id);
                store
                    .replace_saved_views(&views)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.layout_error = Some(format!("Couldn’t delete the layout: {error}"));
            return;
        }
        // The tab keeps its charts and becomes unnamed.
        if self.active_view_id == Some(id) {
            self.active_view_id = None;
            self.active_view_name = None;
            self.autosave = AutosaveState::default();
        }
    }

    /// Rebuilds the workspace from a saved layout; the one place a layout is applied.
    fn apply_saved_view(&mut self, view: &SavedView, ctx: &egui::Context) {
        self.charts.clear();
        for saved in &view.charts {
            self.charts
                .push(Chart::restored(saved.clone(), view.timeframe, ctx));
        }
        self.timeframe = view.timeframe;
        self.next_id = self
            .charts
            .iter()
            .map(|chart| chart.id)
            .max()
            .map_or(1, |max| max + 1);
        let chart_ids: Vec<u64> = self.charts.iter().map(|chart| chart.id).collect();
        self.columns =
            ChartColumn::resolved(view.columns.as_deref(), &chart_ids, &mut self.next_id);
        self.focused = self.charts.first().map(|chart| chart.id);
        self.set_sync(view.sync);
        self.drop_rulers();
        self.undo.clear();
        self.redo.clear();
        self.active_view_id = Some(view.id);
        self.active_view_name = Some(view.name.clone());
        self.set_baseline();
        self.autosave = AutosaveState::default();
        let _ = self.update_active_view(|opened| opened.last_opened_at_ms = Some(now_ms()));
    }

    /// The name prompt, the "save changes?" question and the error alert, one at a time.
    pub(super) fn show_layout_sheets(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.layout_error.clone() {
            if layout_prompts::show_error(ctx, &message) {
                self.layout_error = None;
            }
            return;
        }
        if let Some(prompt) = self.layout_prompt {
            let mut focus = std::mem::take(&mut self.prompt_focus);
            let mut name = std::mem::take(&mut self.prompt_name);
            let outcome = layout_prompts::show_name(ctx, prompt, &mut name, &mut focus);
            self.prompt_name = name;
            self.prompt_focus = focus;
            match outcome {
                Some(NameOutcome::Confirm(name)) => {
                    self.close_name_prompt();
                    match prompt {
                        NamePrompt::Save { then_open } => {
                            if self.save_named(name, None)
                                && let Some(id) = then_open
                            {
                                self.pending_open_id = None;
                                self.apply_saved_view_by_id(id, ctx);
                            }
                        }
                        NamePrompt::Copy => self.copy_layout(name),
                        NamePrompt::Rename => self.rename_layout(name),
                    }
                }
                Some(NameOutcome::Cancel) => {
                    self.close_name_prompt();
                    self.pending_open_id = None;
                }
                None => {}
            }
            return;
        }
        let Some(id) = self.pending_open_id else {
            return;
        };
        let name = self
            .active_view_name
            .clone()
            .unwrap_or_else(|| "Unnamed".to_owned());
        match layout_prompts::show_unsaved(ctx, &name) {
            Some(UnsavedChoice::Save) => {
                if self.active_view_id.is_none() {
                    // The first save needs a name; opening follows it.
                    self.open_name_prompt(
                        NamePrompt::Save {
                            then_open: Some(id),
                        },
                        ctx,
                    );
                } else if self.save_active(true) {
                    self.pending_open_id = None;
                    self.apply_saved_view_by_id(id, ctx);
                }
            }
            Some(UnsavedChoice::Discard) => {
                self.pending_open_id = None;
                self.apply_saved_view_by_id(id, ctx);
            }
            Some(UnsavedChoice::Cancel) => self.pending_open_id = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DrawingCommand;
    use crate::workspace::test_support::{
        chart_with_drawings, drawing_ids, empty_workspace, rectangle, unreadable,
    };

    /// What another Mac's edit looks like once iCloud Sync has stored it: the saved layout's
    /// drawings for one chart replaced.
    fn store_remote_drawings(workspace: &ChartWorkspace, chart: u64, drawings: Vec<Drawing>) {
        let mut views = workspace.load_saved_views();
        for view in &mut views {
            for saved in view.charts.iter_mut().filter(|saved| saved.id == chart) {
                saved.drawings = drawings.clone();
            }
        }
        workspace
            .store
            .lock()
            .unwrap()
            .replace_saved_views(&views)
            .unwrap();
    }

    fn live_ids(workspace: &ChartWorkspace, chart: u64) -> Vec<u64> {
        drawing_ids(workspace, chart)
    }

    #[test]
    fn unreadable_drawings_arriving_in_an_open_layout_survive_the_next_autosave() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        store_remote_drawings(
            &workspace,
            chart,
            vec![rectangle(1, 100), unreadable(50), unreadable(51)],
        );
        workspace.adopt_remote_layout();
        store_remote_drawings(
            &workspace,
            chart,
            vec![
                rectangle(1, 100),
                unreadable(50),
                unreadable(51),
                unreadable(52),
            ],
        );
        workspace.adopt_remote_layout();

        let stored_ids = |drawings: &[Drawing]| -> Vec<u64> {
            drawings.iter().map(Drawing::stored_id).collect()
        };
        let live = &workspace
            .charts
            .iter()
            .find(|c| c.id == chart)
            .unwrap()
            .drawings;
        assert_eq!(stored_ids(live), [1, 50, 51, 52]);
        // Whatever the next write is, nothing the other Mac drew is dropped.
        workspace.flush_autosave();
        let saved = workspace.load_saved_views();
        assert_eq!(stored_ids(&saved[0].charts[0].drawings), [1, 50, 51, 52]);
        assert!(
            workspace.chart_data(chart).unwrap().next_drawing_id > 52,
            "new drawings never reuse an id a newer build stored"
        );
    }

    #[test]
    fn a_drawing_another_mac_made_appears_in_the_open_autosave_layout_without_a_write_back() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        assert!(!workspace.drawings_dirty());

        store_remote_drawings(
            &workspace,
            chart,
            vec![rectangle(1, 100), rectangle(7, 700)],
        );
        workspace.adopt_remote_layout();

        assert_eq!(live_ids(&workspace, chart), [1, 7]);
        assert!(
            !workspace.drawings_dirty(),
            "it is already stored: nothing to save"
        );
        // The old behaviour: the next autosave wrote the tab's stale list over the other Mac's.
        assert!(workspace.flush_autosave());
        let saved = workspace.load_saved_views();
        assert_eq!(saved[0].charts[0].drawings.len(), 2);
        // The counter moved past the arrived drawing.
        assert!(workspace.chart_data(chart).unwrap().next_drawing_id > 7);
    }

    #[test]
    fn unsaved_local_drawings_survive_an_incoming_layout_and_still_get_saved() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(3, 300))]);
        assert!(workspace.drawings_dirty());

        // Meanwhile the other Mac deleted 2 and drew 7.
        store_remote_drawings(
            &workspace,
            chart,
            vec![rectangle(1, 100), rectangle(7, 700)],
        );
        workspace.adopt_remote_layout();

        assert_eq!(live_ids(&workspace, chart), [1, 3, 7]);
        assert!(
            workspace.drawings_dirty(),
            "the drawing made here is still unsaved"
        );
        assert!(workspace.flush_autosave());
        let ids: Vec<u64> = workspace.load_saved_views()[0].charts[0]
            .drawings
            .iter()
            .map(Drawing::id)
            .collect();
        assert_eq!(ids, [1, 3, 7]);
    }

    #[test]
    fn a_layout_nobody_changed_is_left_alone() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        assert!(workspace.save_named("Nico".to_owned(), Some(true)));
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(1)]);
        workspace.adopt_remote_layout();
        assert_eq!(
            live_ids(&workspace, chart),
            [2],
            "an unsynced local edit stays"
        );
        assert!(workspace.drawings_dirty());
    }

    #[test]
    fn drawing_edits_mark_drawings_dirty_but_not_the_layout_fingerprint() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.mark_clean();
        assert!(!workspace.drawings_dirty());

        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(2, 500))]);
        assert!(workspace.drawings_dirty());
        assert!(
            !workspace.is_dirty(),
            "a drawing never marks the layout as edited"
        );
        workspace.mark_clean();
        assert!(!workspace.drawings_dirty(), "saving takes them in");

        workspace.undo();
        assert!(workspace.drawings_dirty(), "undo changes drawings too");
        workspace.mark_clean();
        workspace.redo();
        assert!(workspace.drawings_dirty());
    }

    #[test]
    fn a_command_that_changes_nothing_leaves_drawings_clean() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.mark_clean();
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Remove(999)]);
        assert!(!workspace.drawings_dirty());
        workspace.apply_drawing_commands(chart, Vec::new());
        assert!(!workspace.drawings_dirty());
    }

    #[test]
    fn an_autosave_layout_writes_a_drawing_only_edit_and_a_manual_one_does_not() {
        let mut workspace = empty_workspace(8);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        assert!(workspace.save_named("Auto".to_owned(), Some(true)));
        assert!(workspace.autosave_enabled());
        workspace.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(1, 100))]);
        assert!(workspace.drawings_dirty());

        assert!(workspace.flush_autosave());
        let saved = workspace.load_saved_views();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0].charts[0].drawings.len(),
            1,
            "the drawing reached the saved layout"
        );
        assert!(!workspace.drawings_dirty());

        // A layout with Autosave off keeps today's behaviour: a drawing alone writes nothing.
        let mut manual = empty_workspace(9);
        let chart = chart_with_drawings(&mut manual, "ETHUSDT", &[]);
        assert!(manual.save_named("Manual".to_owned(), Some(false)));
        manual.apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(1, 100))]);
        assert!(manual.flush_autosave());
        assert!(manual.load_saved_views()[0].charts[0].drawings.is_empty());
    }

    #[test]
    fn the_session_saves_every_drawing_and_restores_the_same_ones() {
        let mut workspace = empty_workspace(14);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2, 3]);
        let mut locked = rectangle(2, 200);
        locked.set_locked(true);
        workspace.apply_drawing_commands(
            chart,
            vec![DrawingCommand::Replace {
                id: 2,
                drawing: locked,
            }],
        );
        let saved = workspace.persisted();
        let json = serde_json::to_string(&saved).unwrap();
        let reopened: PersistedWorkspace = serde_json::from_str(&json).unwrap();
        assert_eq!(reopened, saved);
        assert_eq!(reopened.charts[0].drawings.len(), 3);
        assert!(reopened.charts[0].drawings[1].locked());
    }
}
