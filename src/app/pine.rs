use super::DegenViewApp;
use super::now_ms;
use super::tabs::TabContent;
use super::tabs::TabSlot;
use crate::lock::lock;
use crate::pine_studio::PineStudio;
use eframe::egui;
use std::collections::HashSet;

impl DegenViewApp {
    /// The focused tab's Pine Studio, if that is what has the keyboard, for the Edit menu.
    pub(super) fn with_focused_pine_studio<R>(
        &self,
        f: impl FnOnce(&mut PineStudio) -> R,
    ) -> Option<R> {
        let focused = self.tabs.focused_tab();
        for tab in self.tabs.slots() {
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
        if let Some(id) = self
            .tabs
            .find(|content| matches!(content, TabContent::Pine(_)))
        {
            self.tabs.show(ctx, id);
            return;
        }

        let id = self.tabs.allocate_id();
        let slot = TabSlot::new(TabContent::Pine(Box::new(PineStudio::new(
            id,
            self.store.clone(),
            self.editor_themes.clone(),
        ))));
        self.tabs.register(slot, None);
    }

    /// Opens the Pine Studio (the tab there already is, or a new one) on a log message's call.
    pub(super) fn open_pine_source(
        &mut self,
        ctx: &egui::Context,
        target: &crate::model::pine::logs_view::SourceTarget,
    ) {
        self.open_pine(ctx);
        for tab in self.tabs.slots() {
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
        for tab in self.tabs.slots() {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                return workspace.focused_pine_target();
            }
        }
        None
    }

    /// Runs the script alerts. A tab holding an armed one keeps its data current from here, on
    /// screen or not; other tabs fetch only while they are drawn.
    pub(super) fn tick_pine(&mut self, ctx: &egui::Context) {
        let store = lock(&self.store);
        let now = now_ms();
        let mut fired = Vec::new();
        let mut datasets = std::collections::HashMap::new();
        for tab in self.tabs.slots() {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                for (chart, key, timeframe) in workspace.pine_datasets() {
                    datasets.insert(chart, (key, timeframe));
                }
            }
        }
        let watched: HashSet<u64> = self.pine_host.lock().map_or_else(
            |_| HashSet::new(),
            |host| {
                datasets
                    .iter()
                    .filter(|(chart, (key, timeframe))| host.is_watched(**chart, key, timeframe))
                    .map(|(chart, _)| chart.workspace_id)
                    .collect()
            },
        );
        self.alerts.set_chart_datasets(datasets);
        let mut feeds = Vec::new();
        for tab in self.tabs.slots() {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &mut *content
            {
                if watched.contains(&workspace.id()) {
                    workspace.update_market_data(ctx);
                }
                feeds.extend(workspace.charts_candles());
            }
        }
        if let Ok(mut host) = self.pine_host.lock() {
            host.reconcile(&feeds, &store);
            for feed in &feeds {
                fired.extend(host.run_active_on(feed, now));
            }
        }
        if !fired.is_empty() {
            self.alerts.dispatch_pine(&fired, &store, &self.store, now);
        }
    }
}
