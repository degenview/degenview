use super::DegenViewApp;
use super::now_ms;
use super::tabs::TabContent;
use super::tabs::TabSlot;
use crate::lock::lock;
use crate::pine_studio::PineStudio;
use eframe::egui;

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

    pub(super) fn tick_pine(&mut self) {
        let store = lock(&self.store);
        let now = now_ms();
        let mut fired = Vec::new();
        let mut datasets = std::collections::HashMap::new();
        for tab in self.tabs.slots() {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                for (chart_id, key, timeframe) in workspace.pine_datasets() {
                    datasets.insert(chart_id, (key, timeframe));
                }
            }
        }
        self.alerts.set_chart_datasets(datasets);
        if let Ok(mut host) = self.pine_host.lock() {
            for tab in self.tabs.slots() {
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
        if !fired.is_empty() {
            self.alerts.dispatch_pine(&fired, &store, &self.store, now);
        }
    }
}
