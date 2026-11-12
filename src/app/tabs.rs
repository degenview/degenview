use super::AppAction;
use super::DegenViewApp;
use super::window::WindowEnv;
use crate::host::portfolio::PortfolioTab;
use crate::model::InstrumentId;
use crate::model::Timeframe;
use crate::model::tab::windows::WindowId;
use crate::model::watchlist::WatchlistInstrument;
use crate::pine_studio::{ExitReason, PineStudio, PineStudioAction};
use crate::ui::icons::Icon;
use crate::workspace::{ChartWorkspace, WorkspaceAction};
use eframe::egui;
use std::sync::{Arc, Mutex};

pub(super) enum TabContent {
    Chart(Box<ChartWorkspace>),
    Portfolio(Box<PortfolioTab>),
    Pine(Box<PineStudio>),
}

impl TabContent {
    pub(super) fn id(&self) -> u64 {
        match self {
            Self::Chart(workspace) => workspace.id(),
            Self::Portfolio(portfolio) => portfolio.id(),
            Self::Pine(pine) => pine.id(),
        }
    }

    pub(super) fn title(&self) -> String {
        match self {
            Self::Chart(workspace) => workspace.title(),
            Self::Portfolio(portfolio) => portfolio.title().to_owned(),
            Self::Pine(pine) => pine.title(),
        }
    }

    pub(super) fn icon(&self) -> Icon {
        match self {
            Self::Chart(_) => Icon::ChartLine,
            Self::Portfolio(_) => Icon::Briefcase,
            Self::Pine(_) => Icon::Curlybraces,
        }
    }
}

pub(super) struct TabSlot {
    pub(super) content: Arc<Mutex<TabContent>>,
}

impl TabSlot {
    pub(super) fn new(content: TabContent) -> Self {
        Self {
            content: Arc::new(Mutex::new(content)),
        }
    }

    pub(super) fn id(&self) -> u64 {
        self.content.lock().map_or(0, |content| content.id())
    }
}

/// Every open tab's content by id. The window callbacks read it live (they outlive any one
/// frame's snapshot) because a dragged tab changes windows between frames.
pub(super) type TabRegistry = Arc<Mutex<std::collections::HashMap<u64, Arc<Mutex<TabContent>>>>>;

/// The quote-demand owner for the markets the tab strips show: clear of the tab ids the sidebars
/// use, the portfolio cards (`1 << 61`), the alert sheet (`1 << 62`) and the alert runtime.
const TAB_QUOTE_OWNER: u64 = 1 << 60;

/// The tab showing in a window: its content, and the app actions that content asks for.
pub(super) fn show_tab(
    ui: &mut egui::Ui,
    env: &WindowEnv,
    id: u64,
    content: &Arc<Mutex<TabContent>>,
    chart_info: Option<crate::pine_studio::ChartInfo>,
) {
    let Ok(mut content) = content.lock() else {
        return;
    };
    match &mut *content {
        TabContent::Chart(workspace) => {
            if let Ok(host) = env.alert_host.lock() {
                workspace.set_unseen_alerts(host.unseen().count());
                workspace.set_price_alerts(&host.snapshot().alerts);
            }
            if let Ok(rates) = env.alert_fx_shared.lock() {
                workspace.set_alert_fx_rates(&rates);
            }
            workspace.set_card_env(&env.card_env);
            workspace.set_trading_hub(&env.trading_hub);
            workspace.set_snapshots(&env.snapshots);
            workspace.set_link_host(&env.tab_link);
            let actions = workspace.ui(ui, &env.paper_host);
            if workspace.has_link_signals() {
                // The root window collects them; it only draws when asked.
                ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
            }
            if !actions.is_empty()
                && let Ok(mut pending) = env.app_actions.lock()
            {
                pending.extend(actions.into_iter().map(|action| match action {
                    WorkspaceAction::OpenPortfolio => AppAction::OpenPortfolio,
                    WorkspaceAction::OpenPineScripts => AppAction::OpenPine,
                    WorkspaceAction::OpenPineSource(target) => AppAction::OpenPineSource(target),
                    WorkspaceAction::OpenAlerts => AppAction::OpenAlerts,
                    WorkspaceAction::SavePriceAlert { alert, baseline } => {
                        AppAction::SavePriceAlert { alert, baseline }
                    }
                    WorkspaceAction::OpenWebhookSettings => AppAction::OpenWebhookSettings,
                    WorkspaceAction::OpenWatchlistInNewTab(item) => {
                        AppAction::OpenWatchlistInNewTab(item)
                    }
                    WorkspaceAction::OpenAlpacaSettings => AppAction::OpenAlpacaSettings,
                    WorkspaceAction::OpenBrokerSettings => AppAction::OpenBrokerSettings,
                    WorkspaceAction::OpenSnapshotSettings => AppAction::OpenSnapshotSettings,
                    WorkspaceAction::SetAlertState { id, state } => {
                        AppAction::SetAlertState { id, state }
                    }
                    WorkspaceAction::DeleteAlert(id) => AppAction::DeleteAlert(id),
                    WorkspaceAction::MoveAlert { alert, baseline } => {
                        AppAction::MoveAlert { alert, baseline }
                    }
                }));
            }
        }
        TabContent::Portfolio(portfolio) => {
            if let Ok(mut host) = env.portfolio_host.lock() {
                portfolio.ui(ui, &mut host);
            }
        }
        TabContent::Pine(pine) => {
            let pine_actions = pine.ui(ui, chart_info);
            if let Some(decision) = pine.take_exit_decision() {
                match decision.reason {
                    ExitReason::CloseTab if decision.proceed => {
                        env.host.close_tab(id);
                    }
                    ExitReason::CloseTab => {}
                    ExitReason::Quit => {
                        if let Ok(mut pending) = env.app_actions.lock() {
                            pending.push(AppAction::QuitDecided {
                                proceed: decision.proceed,
                            });
                        }
                    }
                }
                ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
            }
            if !pine_actions.is_empty()
                && let Ok(mut pending) = env.app_actions.lock()
            {
                pending.extend(pine_actions.into_iter().map(|action| match action {
                    PineStudioAction::AddToChart {
                        script_id,
                        name,
                        source,
                    } => AppAction::AddPineScript {
                        script_id: Some(script_id),
                        name,
                        source,
                    },
                    PineStudioAction::CreateAlert {
                        chart_id,
                        instance_id,
                        script_id,
                        script_name,
                        source,
                        inputs,
                        symbol_key,
                        timeframe,
                        note,
                        webhook_endpoint_ids,
                    } => AppAction::CreatePineAlert {
                        chart_id,
                        instance_id,
                        script_id,
                        script_name,
                        source,
                        inputs,
                        symbol_key,
                        timeframe,
                        note,
                        webhook_endpoint_ids,
                    },
                    PineStudioAction::OpenAlpacaSettings => AppAction::OpenAlpacaSettings,
                    PineStudioAction::OpenWebhookSettings => AppAction::OpenWebhookSettings,
                    PineStudioAction::OpenAiSettings => AppAction::OpenAiSettings,
                }));
            }
        }
    }
}

impl DegenViewApp {
    /// Opens `slot` in `window`, or in the window in front, and shows it.
    pub(super) fn register(&mut self, slot: TabSlot, window: Option<WindowId>) {
        let id = slot.id();
        if let Ok(mut registry) = self.registry.lock() {
            registry.insert(id, Arc::clone(&slot.content));
        }
        match window {
            Some(window) => self.tab_host.add_tab_to(window, id),
            None => {
                self.tab_host.add_tab(id);
            }
        }
        self.tabs.push(slot);
    }

    /// A tab with no chart: it opens on the empty page, as a new Swift tab does.
    pub(super) fn add_tab(&mut self, window: Option<WindowId>) {
        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Chart(Box::new(ChartWorkspace::empty(
            id,
            Timeframe::Day,
            Arc::clone(&self.watchlist),
            self.quote_book.clone(),
            self.quote_coordinator.clone(),
            self.store.clone(),
        ))));
        self.register(slot, window);
    }

    pub(super) fn add_watchlist_tab(&mut self, item: WatchlistInstrument, ctx: &egui::Context) {
        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Chart(Box::new(
            ChartWorkspace::new_for_watchlist(
                id,
                &item,
                Arc::clone(&self.watchlist),
                self.quote_book.clone(),
                self.quote_coordinator.clone(),
                self.store.clone(),
                ctx,
            ),
        )));
        self.register(slot, None);
    }

    /// Closes `tab`, unless it is a Pine Studio with unsaved edits: that one comes to the front
    /// and asks first (its answer closes it through `TabWindowHost::close_tab`).
    pub(super) fn close_tab_request(&mut self, ctx: &egui::Context, tab: u64) {
        let asking = self.tabs.iter().any(|slot| {
            slot.content
                .lock()
                .is_ok_and(|mut content| match &mut *content {
                    TabContent::Pine(pine) => {
                        pine.id() == tab && pine.begin_exit(ExitReason::CloseTab)
                    }
                    _ => false,
                })
        });
        if asking {
            self.show_tab(ctx, tab);
        } else {
            self.tab_host.close_tab(tab);
        }
    }

    /// Forgets the contents of tabs the window host closed. Without a window left there is
    /// nothing to show, so Windows and Linux quit; macOS stays in the Dock as it always did.
    pub(super) fn drop_closed_tabs(&mut self, ctx: &egui::Context) {
        let closed = self.tab_host.take_closed();
        if closed.is_empty() {
            return;
        }
        self.tabs.retain(|slot| !closed.contains(&slot.id()));
        if let Ok(mut registry) = self.registry.lock() {
            for id in &closed {
                registry.remove(id);
            }
        }
        if self.tab_host.is_empty() && !cfg!(target_os = "macos") {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        }
    }

    /// Brings `tab` to the front of its window and focuses the window.
    pub(super) fn show_tab(&self, ctx: &egui::Context, tab: u64) {
        if let Some(window) = self.tab_host.select_tab(tab) {
            let viewport = Self::viewport_id(window);
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            ctx.request_repaint_of(viewport);
        }
    }

    pub(super) fn open_portfolio(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Portfolio(portfolio) => Some(portfolio.id()),
                TabContent::Chart(_) | TabContent::Pine(_) => None,
            }
        });
        if let Some(id) = existing {
            self.show_tab(ctx, id);
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        let slot = TabSlot::new(TabContent::Portfolio(Box::new(PortfolioTab::new(
            id,
            self.portfolio_quotes.clone(),
        ))));
        self.register(slot, None);
    }

    pub(super) fn tick_tab_quotes(&mut self, ctx: &egui::Context) {
        let mut wanted: Vec<InstrumentId> = Vec::new();
        for slot in &self.tabs {
            let Ok(content) = slot.content.lock() else {
                continue;
            };
            if let TabContent::Chart(workspace) = &*content
                && let Some(instrument) = workspace.focused_instrument()
                && crate::model::tab::title::wants_quote(instrument.source)
            {
                wanted.push(instrument);
            }
        }
        wanted.sort_by_key(InstrumentId::key);
        wanted.dedup_by_key(|instrument| instrument.key());
        let keys: Vec<String> = wanted.iter().map(InstrumentId::key).collect();
        if keys != self.tab_quote_keys {
            if wanted.is_empty() {
                self.quote_coordinator.release(TAB_QUOTE_OWNER);
            } else {
                self.quote_coordinator
                    .set_demand(TAB_QUOTE_OWNER, wanted, true);
            }
            self.tab_quote_keys = keys;
        }
        // The coordinator asks the root window for a frame when quotes move; the tab windows are
        // their own viewports and need asking.
        // The same for a new title style from Settings or another window: each window repaints
        // on its own schedule, and one with nothing animating would show the old titles.
        let revision = self.quote_book.revision();
        let style_revision = self.tab_title_host.revision();
        // A broker's orders and balances change on their own threads; every window shows them.
        let broker_revision = self.broker_host.revision();
        if revision != self.tab_quote_revision
            || style_revision != self.tab_title_revision
            || broker_revision != self.broker_revision
        {
            self.tab_quote_revision = revision;
            self.tab_title_revision = style_revision;
            self.broker_revision = broker_revision;
            for (window, _) in self.tab_host.snapshot() {
                ctx.request_repaint_of(Self::viewport_id(window.id));
            }
        }
    }

    /// Tab Linking, once per frame of the root window: reads who is in which group from the tabs,
    /// collects what the users did since the last frame, and applies what the groups answer.
    ///
    /// Membership comes from the tabs themselves each frame (never from a second list), so a tab
    /// that was closed, moved to another window or recoloured is already right. Tabs are locked
    /// one at a time and never while another is: a delivery is applied after every tab has been
    /// read. Nothing here waits on the network; a delivery starts the same fetch the user's own
    /// choice would have.
    pub(super) fn tick_tab_links(&mut self, ctx: &egui::Context) {
        let mut members = Vec::new();
        let mut signals = Vec::new();
        let mut slots = Vec::new();
        for (index, slot) in self.tabs.iter().enumerate() {
            let Ok(mut content) = slot.content.lock() else {
                continue;
            };
            if let TabContent::Chart(workspace) = &mut *content {
                let member = workspace.link_member();
                let said = workspace.take_link_signals();
                if !said.is_empty() {
                    signals.push((member.tab, said));
                }
                slots.push((member.tab, index));
                members.push(member);
            }
        }
        let table = self.tab_link_host.table();
        let deliveries = self.link_hub.tick(&members, &table, signals);
        let mut moved = false;
        for delivery in deliveries {
            let Some(&(_, index)) = slots.iter().find(|(tab, _)| *tab == delivery.target) else {
                continue;
            };
            let Ok(mut content) = self.tabs[index].content.lock() else {
                continue;
            };
            if let TabContent::Chart(workspace) = &mut *content
                && workspace.apply_link(delivery.sequence, &delivery.signal, ctx)
            {
                moved = true;
            }
        }
        // The tab windows are viewports of their own: a changed chart, a new colour dot or a
        // changed switch is only seen by a window that is asked to draw.
        let revision = self.tab_link_host.revision();
        if moved || revision != self.tab_link_revision {
            self.tab_link_revision = revision;
            for (window, _) in self.tab_host.snapshot() {
                ctx.request_repaint_of(Self::viewport_id(window.id));
            }
        }
    }
}
