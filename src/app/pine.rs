use super::DegenViewApp;
use super::now_ms;
use super::tabs::TabContent;
use super::tabs::TabSlot;
use crate::mcp::bridge::McpError;
use crate::pine_studio::PineStudio;
use eframe::egui;
use std::sync::atomic::Ordering;

impl DegenViewApp {
    /// The focused tab's Pine Studio, if that is what has the keyboard, for the Edit menu.
    pub(super) fn with_focused_pine_studio<R>(
        &self,
        f: impl FnOnce(&mut PineStudio) -> R,
    ) -> Option<R> {
        let focused = self.focused_tab.load(Ordering::Acquire);
        for tab in &self.tabs {
            let Ok(mut content) = tab.content.lock() else {
                continue;
            };
            if let TabContent::Pine(pine) = &mut *content
                && pine.id() == focused
            {
                return Some(f(pine));
            }
        }
        None
    }

    pub(super) fn open_pine(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Pine(pine) => Some(pine.id()),
                TabContent::Chart(_) | TabContent::Portfolio(_) => None,
            }
        });
        if let Some(id) = existing {
            self.show_tab(ctx, id);
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Pine(Box::new(PineStudio::new(
            id,
            self.store.clone(),
        ))));
        self.register(slot, None);
    }

    /// Opens the Pine Studio (the tab there already is, or a new one) on a log message's call.
    pub(super) fn open_pine_source(
        &mut self,
        ctx: &egui::Context,
        target: &crate::model::pine::logs_view::SourceTarget,
    ) {
        self.open_pine(ctx);
        for tab in &self.tabs {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Pine(pine) = &mut *content
            {
                pine.reveal_source(target);
                break;
            }
        }
    }

    /// The first chart tab's focused chart identity, for the Pine integration.
    pub(super) fn focused_pine_target(&self) -> Option<crate::workspace::PineChartTarget> {
        for tab in &self.tabs {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                return workspace.focused_pine_target();
            }
        }
        None
    }

    /// The text of `id` in a Pine Studio editor that has it open, if any: what the person sees.
    fn open_script_text(&self, id: &str) -> Option<String> {
        self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Pine(manager) => manager.open_text(id),
                _ => None,
            }
        })
    }

    /// How the Pine Studio editor that has `id` open stands against the file on disk.
    fn open_script_disk_state(&self, id: &str) -> Option<crate::pine_studio::OpenDiskState> {
        self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Pine(manager) => manager.open_disk_state(id),
                _ => None,
            }
        })
    }

    /// Asks the editor that has `id` open to look at the file now.
    fn look_at_open_script_disk(&self, id: &str) {
        for tab in &self.tabs {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Pine(manager) = &mut *content
            {
                manager.look_at_disk_now(id);
            }
        }
    }

    /// Shows an edit in the Pine Studio editor that has `id` open. Returns whether one did.
    fn show_in_open_editor(&self, id: &str, text: &str, saved: bool) -> bool {
        self.tabs.iter().any(|tab| {
            tab.content
                .lock()
                .is_ok_and(|mut content| match &mut *content {
                    TabContent::Pine(manager) => manager.adopt_external_text(id, text, saved),
                    _ => false,
                })
        })
    }

    /// Writes a script edit through `ScriptStore`, checked against the text the Pine Studio
    /// shows (its open editor if there is one), and updates that editor. `source: Some` is a
    /// draft edit; `None` saves the current draft. Runs on the app thread, so no frame can
    /// type into the editor between the check and the write.
    pub(super) fn edit_pine_script(
        &self,
        id: &str,
        expected_revision: &str,
        source: Option<&str>,
    ) -> Result<crate::mcp::types::PineEditResult, McpError> {
        let shared = self
            .mcp_host
            .lock()
            .map_err(|_| McpError::AppUnavailable("The MCP host is unavailable".into()))?
            .shared();
        let mut store = shared
            .script_store()
            .ok_or_else(|| McpError::AppUnavailable("The script library is unavailable".into()))?;
        let live = self.open_script_text(id);
        let mut result = match source {
            Some(source) => crate::mcp::pine::update_draft(
                &mut store,
                id,
                expected_revision,
                source,
                live.as_deref(),
            )?,
            None => {
                let open = self.open_script_disk_state(id);
                let saved = crate::mcp::pine::save_script(
                    &mut store,
                    id,
                    expected_revision,
                    live.as_deref(),
                    open.as_ref(),
                );
                if matches!(saved, Err(McpError::Conflict(_))) && open.is_some() {
                    // Same as the editor's own refused Save: raise the file/typing question now.
                    self.look_at_open_script_disk(id);
                }
                saved?
            }
        };
        if result.status != "unchanged" {
            let (text, saved) = match source {
                Some(text) => (text.to_owned(), false),
                None => (
                    store
                        .script(id)
                        .ok()
                        .flatten()
                        .map(|script| script.source)
                        .unwrap_or_default(),
                    true,
                ),
            };
            result.applied_to_open_editor = self.show_in_open_editor(id, &text, saved);
        } else {
            result.applied_to_open_editor = live.is_some();
        }
        Ok(result)
    }

    pub(super) fn tick_pine(&mut self) {
        let store = self.store.lock().expect("store lock poisoned");
        let now = now_ms();
        let mut fired = Vec::new();
        let mut datasets = std::collections::HashMap::new();
        for tab in &self.tabs {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                for (chart_id, key, timeframe) in workspace.pine_datasets() {
                    datasets.insert(chart_id, (key, timeframe));
                }
            }
        }
        if let Ok(mut shared) = self.chart_datasets.lock() {
            *shared = datasets;
        }
        if let Ok(mut host) = self.pine_host.lock() {
            for tab in &self.tabs {
                if let Ok(content) = tab.content.lock()
                    && let TabContent::Chart(workspace) = &*content
                {
                    for feed in workspace.charts_candles() {
                        host.reconcile_chart(feed.chart_id, &feed.instances, &store);
                        fired.extend(host.run_active_on(
                            feed.chart_id,
                            &feed.symbol_key,
                            &feed.timeframe,
                            &feed.bars,
                            &feed.run,
                            &feed.instances,
                            now,
                        ));
                    }
                }
            }
        }
        if !fired.is_empty()
            && let Ok(host) = self.alert_host.lock()
        {
            host.dispatch_pine(&fired, &store, &self.store, &*self.notifier, now);
        }
    }
}
