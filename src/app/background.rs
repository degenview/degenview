use super::DegenViewApp;
use super::tabs::TabContent;
use crate::service::alert::macos_notifications;
use eframe::egui;
use std::sync::Arc;
use std::time::Instant;

impl DegenViewApp {
    /// Keeps quotes coming for the market each chart tab shows, and repaints the tab windows
    /// when any quote moved. One owner asks for all of them, however many tabs and windows
    /// there are: the coordinator merges demand by market, so a market a sidebar, an alert or a
    /// portfolio card already follows costs nothing more. Unlike a sidebar's lease it does not
    /// lapse, so a tab in the background keeps its price.
    /// Tells the user what happened to their live orders: a short message at the foot of the
    /// focused tab and a system notification. Each notice arrives once (the broker crate's book
    /// deduplicates replayed messages) and none for the orders already open when a connection
    /// starts.
    pub(super) fn tick_broker_notices(&mut self) {
        let notices = self.broker_host.drain_notices();
        if notices.is_empty() {
            return;
        }
        let focused = self.tabs.focused_tab();
        for (broker, notice) in notices {
            let prefs = self.broker_host.prefs(broker);
            if !prefs.notify {
                continue;
            }
            let name = degenview_brokers::registry::by_id(broker)
                .map_or(broker.as_str(), |b| b.descriptor().display_name);
            let spec = self
                .broker_host
                .snapshot(broker)
                .and_then(|snapshot| snapshot.product(&notice.order.product).cloned());
            let text = crate::model::broker::ticket::notice_text(&notice, name, spec.as_ref());
            for slot in self.tabs.slots() {
                if slot.id() != focused {
                    continue;
                }
                if let Ok(mut content) = slot.content.lock()
                    && let TabContent::Chart(workspace) = &mut *content
                {
                    workspace.announce(format!("{}: {}", text.title, text.body), text.error);
                }
            }
            macos_notifications::notify_async(
                Arc::clone(&self.notifier),
                crate::model::alert::notification::NotificationContent {
                    title: text.title,
                    body: text.body,
                    identifier: text.identifier,
                    sound: false,
                    user_info: Vec::new(),
                },
                |_, _| {},
            );
        }
    }

    /// Quits, but first lets each Pine Studio window ask about its unsaved scripts, one window
    /// at a time. Closing the hidden root window ends the app through the same exit path ⌘Q
    /// always took, so the session is saved first.
    /// Saves the editor's themes a moment after they last changed, and picks up a font that has
    /// been read. The store is held only for the write.
    pub(super) fn tick_editor_themes(&self) {
        if let Ok(store) = self.store.lock() {
            self.editor_themes.tick(&store, Instant::now());
        }
    }

    /// Runs active Pine alert subscriptions over every open chart's candles and dispatches any that
    /// fire (webhooks + macOS notification via `dispatch_pine`). Each subscription keeps a
    /// persistent execution controller, so alerts follow the real tick/close lifecycle.
    /// Applies what iCloud Sync fetched and reloads the stores this app holds in memory.
    pub(super) fn tick_sync(&mut self, ctx: &egui::Context) {
        // Script files change outside the database: tell sync when the library or the text being
        // typed changes.
        let mut unsaved = std::collections::HashSet::new();
        let mut signature = 0u64;
        for slot in self.tabs.slots() {
            if let Ok(content) = slot.content.try_lock()
                && let TabContent::Pine(manager) = &*content
            {
                signature ^= manager.library_signature();
                unsaved.extend(manager.open_unsaved_id());
            }
        }
        if signature != self.script_signature {
            self.script_signature = signature;
            self.sync.note_scripts_changed();
        }
        self.sync.set_unsaved_scripts(unsaved);
        for report in self.sync.tick() {
            if (report.tables.contains("watchlist") || report.settings.contains("watchlist.flags"))
                && let Ok(mut watchlist) = self.watchlist.lock()
            {
                watchlist.reload();
            }
            if report.tables.contains("saved_view") {
                self.tabs
                    .each_chart(|workspace| workspace.adopt_remote_layout());
            }
            if report.tables.contains("paper_account")
                && let Ok(mut paper) = self.paper.lock()
            {
                paper.reload();
            }
            if (report.tables.contains("portfolio")
                || report.tables.contains("portfolio_transaction"))
                && let Ok(mut portfolio) = self.portfolio.lock()
            {
                portfolio.reload();
            }
            if (report.tables.contains("editor_theme")
                || report
                    .settings
                    .contains(crate::model::editor_theme::prefs::KEY))
                && let Ok(store) = self.store.lock()
            {
                self.editor_themes.reload(&store);
            }
            if report.settings.contains(crate::ui::theme::THEME_KEY) {
                let theme = self
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| store.setting(crate::ui::theme::THEME_KEY))
                    .unwrap_or_default();
                crate::ui::theme::apply(ctx, theme);
            }
            ctx.request_repaint();
        }
    }
}
