use super::AppAction;
use super::DegenViewApp;
use super::window::WindowEnv;
use crate::host::tab::link::TabLinkHost;
use crate::host::tab::title::TabTitleHost;
use crate::host::tab::window::{Placement, TabWindowHost};
use crate::model::InstrumentId;
use crate::model::Timeframe;
use crate::model::tab::link::LinkHub;
use crate::model::tab::windows::WindowId;
use crate::model::watchlist::WatchlistInstrument;
use crate::pine_studio::{ExitReason, PineStudio, PineStudioAction};
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::ui::icons::Icon;
use crate::ui::portfolio::tab::PortfolioTab;
use crate::ui::window::tab_bar::TabView;
use crate::workspace::{ChartWorkspace, WorkspaceAction};
use eframe::egui;
use std::sync::atomic::{AtomicU64, Ordering};
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
            Self::Pine(_) => Icon::AngleBrackets,
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
type TabRegistry = Arc<Mutex<std::collections::HashMap<u64, Arc<Mutex<TabContent>>>>>;

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
            workspace.set_link_host(env.tabs.links());
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
                        env.tabs.windows().close_tab(id);
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

/// What the tab windows' callbacks share with the app: which tabs share a window, every tab's
/// content by id, the focused tab, and how tabs are titled and linked. Cheap to clone; each
/// deferred viewport holds one.
#[derive(Clone)]
pub(super) struct SharedTabs {
    /// Which tabs share a window; each window shows one tab strip over its active tab.
    windows: TabWindowHost,
    registry: TabRegistry,
    /// The id of the tab whose window last had keyboard focus, for Save and other app-level
    /// commands.
    focused: Arc<AtomicU64>,
    /// How chart tabs name themselves, shared by every window and Settings.
    titles: TabTitleHost,
    /// Tab Linking: the switches of every colour, and who is in which group.
    links: TabLinkHost,
}

impl SharedTabs {
    pub(super) fn windows(&self) -> &TabWindowHost {
        &self.windows
    }

    pub(super) fn titles(&self) -> &TabTitleHost {
        &self.titles
    }

    pub(super) fn links(&self) -> &TabLinkHost {
        &self.links
    }

    /// The tab whose window last had keyboard focus (0 before any window had it).
    pub(super) fn focused_tab(&self) -> u64 {
        self.focused.load(Ordering::Acquire)
    }

    /// Notes `tab` as the focused window's active tab. True when that changed it.
    pub(super) fn note_focused_tab(&self, tab: u64) -> bool {
        self.focused.swap(tab, Ordering::AcqRel) != tab
    }

    pub(super) fn content(&self, tab: u64) -> Option<Arc<Mutex<TabContent>>> {
        self.registry.lock().ok()?.get(&tab).cloned()
    }

    pub(super) fn view(&self, tab: u64) -> Option<TabView> {
        let content = self.content(tab)?;
        let content = content.lock().ok()?;
        let (chart, timeframe, link, linkable) = match &*content {
            TabContent::Chart(workspace) => (
                workspace.tab_chart(),
                workspace.timeframe().label(),
                workspace.link(),
                true,
            ),
            _ => (None, "", None, false),
        };
        Some(TabView {
            id: tab,
            title: content.title(),
            icon: content.icon(),
            chart,
            timeframe,
            link,
            linkable,
            link_toggles: link
                .map(|color| self.links.toggles(color))
                .unwrap_or_default(),
        })
    }

    /// Tags a chart tab with a Tab Linking colour, or takes it out of its group. The chart is left
    /// as it is; the hub aligns a tab that joined a group on the next frame.
    pub(super) fn set_link(&self, tab: u64, color: Option<crate::model::tab::link::LinkColor>) {
        let Some(content) = self.content(tab) else {
            return;
        };
        if let Ok(mut content) = content.lock()
            && let TabContent::Chart(workspace) = &mut *content
            && workspace.set_link(color)
        {
            self.links.touch();
        }
    }

    /// Gives a chart tab its own title (or takes it away). Other tabs have none.
    pub(super) fn set_custom_title(&self, tab: u64, title: Option<String>) {
        let Some(content) = self.content(tab) else {
            return;
        };
        if let Ok(mut content) = content.lock()
            && let TabContent::Chart(workspace) = &mut *content
        {
            workspace.set_custom_title(title);
        }
    }
}

/// The open tabs: their contents, the windows they are in, the next tab id, and what the tab
/// windows were last repainted for. Tab Linking's hub runs here once a frame.
pub(super) struct TabState {
    slots: Vec<TabSlot>,
    shared: SharedTabs,
    next_id: u64,
    link_hub: LinkHub,
    /// The Tab Linking switches' revision the tab windows were last repainted for.
    link_revision: u64,
    /// The markets the tab strips asked quotes for last time (their keys), so the demand is only
    /// re-issued when a tab's focused chart changes.
    quote_keys: Vec<String>,
    /// The quote book's revision the tab windows were last repainted for.
    quote_revision: u64,
    /// The title style's revision they were last repainted for.
    title_revision: u64,
    /// The broker host's revision they were last repainted for.
    broker_revision: u64,
}

impl TabState {
    /// The tabs a session restored (or the first one), laid out in their saved windows. `next_id`
    /// is the id the next new tab gets.
    pub(super) fn restore(
        slots: Vec<TabSlot>,
        next_id: u64,
        first_window: Placement,
        titles: TabTitleHost,
        links: TabLinkHost,
    ) -> Self {
        let windows = TabWindowHost::restore(slots.iter().map(TabSlot::id), first_window);
        let registry: TabRegistry = Arc::new(Mutex::new(
            slots
                .iter()
                .map(|tab| (tab.id(), Arc::clone(&tab.content)))
                .collect(),
        ));
        Self {
            slots,
            shared: SharedTabs {
                windows,
                registry,
                focused: Arc::new(AtomicU64::new(0)),
                titles,
                links,
            },
            next_id,
            link_hub: LinkHub::default(),
            link_revision: 0,
            quote_keys: Vec::new(),
            quote_revision: 0,
            title_revision: 0,
            broker_revision: 0,
        }
    }

    pub(super) fn shared(&self) -> &SharedTabs {
        &self.shared
    }

    pub(super) fn windows(&self) -> &TabWindowHost {
        &self.shared.windows
    }

    pub(super) fn focused_tab(&self) -> u64 {
        self.shared.focused_tab()
    }

    /// Every open tab, in the order they were opened.
    pub(super) fn slots(&self) -> &[TabSlot] {
        &self.slots
    }

    /// Every open tab in window order, then strip order: the order a restore lays them out in.
    pub(super) fn in_window_order(&self) -> Vec<&TabSlot> {
        let ordered = self.shared.windows.ordered_tabs();
        let position = |tab: &TabSlot| {
            ordered
                .iter()
                .position(|id| *id == tab.id())
                .unwrap_or(usize::MAX)
        };
        let mut slots: Vec<&TabSlot> = self.slots.iter().collect();
        slots.sort_by_key(|tab| position(tab));
        slots
    }

    /// The id the next new tab gets; saved with the session.
    pub(super) fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Takes the next tab id for a tab about to be opened.
    pub(super) fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Runs `f` on every chart tab's workspace, one tab locked at a time.
    pub(super) fn each_chart(&self, mut f: impl FnMut(&mut ChartWorkspace)) {
        for slot in &self.slots {
            if let Ok(mut content) = slot.content.lock()
                && let TabContent::Chart(workspace) = &mut *content
            {
                f(workspace);
            }
        }
    }

    /// The id of the open tab `matches` picks, if any.
    pub(super) fn find(&self, matches: impl Fn(&TabContent) -> bool) -> Option<u64> {
        self.slots.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            matches(&content).then(|| content.id())
        })
    }

    /// Opens `slot` in `window`, or in the window in front, and shows it.
    pub(super) fn register(&mut self, slot: TabSlot, window: Option<WindowId>) {
        let id = slot.id();
        if let Ok(mut registry) = self.shared.registry.lock() {
            registry.insert(id, Arc::clone(&slot.content));
        }
        match window {
            Some(window) => self.shared.windows.add_tab_to(window, id),
            None => {
                self.shared.windows.add_tab(id);
            }
        }
        self.slots.push(slot);
    }

    /// Closes `tab`, unless it is a Pine Studio with unsaved edits: that one comes to the front
    /// and asks first (its answer closes it through `TabWindowHost::close_tab`).
    pub(super) fn request_close(&self, ctx: &egui::Context, tab: u64) {
        let asking = self.slots.iter().any(|slot| {
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
            self.show(ctx, tab);
        } else {
            self.shared.windows.close_tab(tab);
        }
    }

    /// Forgets the contents of tabs the window host closed. Without a window left there is
    /// nothing to show, so Windows and Linux quit; macOS stays in the Dock as it always did.
    pub(super) fn drop_closed(&mut self, ctx: &egui::Context) {
        let closed = self.shared.windows.take_closed();
        if closed.is_empty() {
            return;
        }
        self.slots.retain(|slot| !closed.contains(&slot.id()));
        if let Ok(mut registry) = self.shared.registry.lock() {
            for id in &closed {
                registry.remove(id);
            }
        }
        if self.shared.windows.is_empty() && !cfg!(target_os = "macos") {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        }
    }

    /// Brings `tab` to the front of its window and focuses the window.
    pub(super) fn show(&self, ctx: &egui::Context, tab: u64) {
        if let Some(window) = self.shared.windows.select_tab(tab) {
            let viewport = TabWindowHost::viewport_id(window);
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            ctx.request_repaint_of(viewport);
        }
    }

    /// Asks every tab window for a frame: they are viewports of their own, and one with nothing
    /// animating would not show a change made elsewhere.
    pub(super) fn repaint_windows(&self, ctx: &egui::Context) {
        for (window, _) in self.shared.windows.snapshot() {
            ctx.request_repaint_of(TabWindowHost::viewport_id(window.id));
        }
    }

    /// Keeps quotes coming for the market each chart tab shows, and repaints the tab windows
    /// when any quote moved. One owner asks for all of them, however many tabs and windows
    /// there are: the coordinator merges demand by market, so a market a sidebar, an alert or a
    /// portfolio card already follows costs nothing more. Unlike a sidebar's lease it does not
    /// lapse, so a tab in the background keeps its price. `broker_revision` is the broker host's.
    pub(super) fn tick_quotes(
        &mut self,
        ctx: &egui::Context,
        quote_book: &WatchlistQuoteBook,
        quote_coordinator: &WatchlistQuoteCoordinator,
        broker_revision: u64,
    ) {
        let mut wanted: Vec<InstrumentId> = Vec::new();
        for slot in &self.slots {
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
        if keys != self.quote_keys {
            if wanted.is_empty() {
                quote_coordinator.release(TAB_QUOTE_OWNER);
            } else {
                quote_coordinator.set_demand(TAB_QUOTE_OWNER, wanted, true);
            }
            self.quote_keys = keys;
        }
        // The coordinator asks the root window for a frame when quotes move; the tab windows are
        // their own viewports and need asking.
        // The same for a new title style from Settings or another window: each window repaints
        // on its own schedule, and one with nothing animating would show the old titles.
        // A broker's orders and balances change on their own threads; every window shows them.
        let revision = quote_book.revision();
        let style_revision = self.shared.titles.revision();
        if revision != self.quote_revision
            || style_revision != self.title_revision
            || broker_revision != self.broker_revision
        {
            self.quote_revision = revision;
            self.title_revision = style_revision;
            self.broker_revision = broker_revision;
            self.repaint_windows(ctx);
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
    pub(super) fn tick_links(&mut self, ctx: &egui::Context) {
        let mut members = Vec::new();
        let mut signals = Vec::new();
        let mut slots = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
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
        let table = self.shared.links.table();
        let deliveries = self.link_hub.tick(&members, &table, signals);
        let mut moved = false;
        for delivery in deliveries {
            let Some(&(_, index)) = slots.iter().find(|(tab, _)| *tab == delivery.target) else {
                continue;
            };
            let Ok(mut content) = self.slots[index].content.lock() else {
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
        let revision = self.shared.links.revision();
        if moved || revision != self.link_revision {
            self.link_revision = revision;
            self.repaint_windows(ctx);
        }
    }
}

impl DegenViewApp {
    /// A tab with no chart: it opens on the empty page, as a new Swift tab does.
    pub(super) fn add_tab(&mut self, window: Option<WindowId>) {
        let id = self.tabs.allocate_id();
        let slot = TabSlot::new(TabContent::Chart(Box::new(ChartWorkspace::empty(
            id,
            Timeframe::Day,
            Arc::clone(&self.watchlist),
            self.quote_book.clone(),
            self.quote_coordinator.clone(),
            self.store.clone(),
        ))));
        self.tabs.register(slot, window);
    }

    pub(super) fn add_watchlist_tab(&mut self, item: WatchlistInstrument, ctx: &egui::Context) {
        let id = self.tabs.allocate_id();
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
        self.tabs.register(slot, None);
    }

    pub(super) fn open_portfolio(&mut self, ctx: &egui::Context) {
        if let Some(id) = self
            .tabs
            .find(|content| matches!(content, TabContent::Portfolio(_)))
        {
            self.tabs.show(ctx, id);
            return;
        }

        let id = self.tabs.allocate_id();
        let slot = TabSlot::new(TabContent::Portfolio(Box::new(PortfolioTab::new(
            id,
            self.portfolio_quotes.clone(),
        ))));
        self.tabs.register(slot, None);
    }
}
