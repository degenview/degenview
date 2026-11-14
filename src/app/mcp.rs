use super::DegenViewApp;
use super::tabs::{TabContent, TabState};
use super::window::settings_viewport_id;
use crate::host::mcp;
use crate::host::mcp::alert::AlertAccess;
use crate::host::mcp::confirmation::{self, ConfirmationHost};
use crate::host::mcp::paper::OrderMemory;
use crate::host::mcp::snapshot::SnapshotJobs;
use crate::host::mcp::{McpHost, McpTabs};
use crate::mcp::bridge::{McpCommand, McpError, McpReply, Request};
use crate::mcp::state::McpShared;
use crate::pine_studio::PineStudio;
use crate::service::alert::macos_notifications;
use crate::store::SharedStore;
use crate::workspace::ChartWorkspace;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The questions MCP clients' high-impact requests raise, waiting for the user. Cheap to clone;
/// the window callbacks each hold one to draw the open question.
#[derive(Clone)]
pub(super) struct McpQuestions(Arc<Mutex<ConfirmationHost>>);

impl McpQuestions {
    /// The question an MCP client's request raises, if one is open. Drawn by the focused window
    /// only, so one question shows however many windows are open; the answer goes to the host.
    pub(super) fn show(&self, ctx: &egui::Context) {
        let now = Instant::now();
        let pending = self
            .0
            .lock()
            .map(|host| host.pending(now))
            .unwrap_or_default();
        let answer = crate::ui::mcp::confirmation::show(ctx, &pending);
        let Some(answer) = answer else {
            return;
        };
        if let Ok(mut host) = self.0.lock() {
            match answer {
                crate::ui::mcp::confirmation::Answer::Allow(id) => host.approve(now, id),
                crate::ui::mcp::confirmation::Answer::Deny(id) => host.deny(now, id),
            }
        }
    }
}

/// The app's side of the MCP server: the server's host, the questions its clients raise, and what
/// the app remembers between requests (placed orders, pictures being made).
pub(super) struct McpState {
    /// The MCP server's lifecycle, credentials and request queue.
    host: Arc<Mutex<McpHost>>,
    questions: McpQuestions,
    /// Orders placed over MCP by `request_id`, so a retried call places nothing twice.
    orders: Mutex<OrderMemory>,
    /// Chart pictures being made for MCP clients.
    snapshots: Mutex<SnapshotJobs>,
}

impl McpState {
    /// Loads the server's settings and credentials (starting it if it is on) and gives it the
    /// watchlists' quote feed to read from. `wake` asks for a frame when a request arrives.
    pub(super) fn load(
        store: SharedStore,
        wake: Arc<dyn Fn() + Send + Sync>,
        quotes: crate::mcp::state::QuoteAccess,
    ) -> Self {
        let host = McpHost::load(store, wake);
        let questions = McpQuestions(Arc::new(Mutex::new(ConfirmationHost::new(host.shared()))));
        // The server reads quotes from the feed the watchlists already run, on its own threads.
        host.shared().attach_quotes(quotes);
        Self {
            host: Arc::new(Mutex::new(host)),
            questions,
            orders: Mutex::new(OrderMemory::default()),
            snapshots: Mutex::new(SnapshotJobs::default()),
        }
    }

    /// The server's host, for the Settings window's MCP page.
    pub(super) fn host(&self) -> &Arc<Mutex<McpHost>> {
        &self.host
    }

    pub(super) fn questions(&self) -> &McpQuestions {
        &self.questions
    }

    /// Stops the server and writes the activity log's last rows (they are otherwise written about
    /// once a second, so a quit would lose the final moments).
    pub(super) fn shutdown(&self) {
        if let Ok(mut host) = self.host.lock() {
            host.shutdown();
        }
    }

    /// Applies queued MCP Settings changes and takes up to `max` of the requests the server queued.
    /// Pictures an agent never collected are dropped on time, not at its next snapshot call.
    fn begin_frame(&self, max: usize) -> Option<(Vec<Request>, Arc<McpShared>)> {
        let taken = {
            let mut host = self.host.lock().ok()?;
            host.tick();
            (host.take_requests(max), host.shared())
        };
        if let Ok(mut snapshots) = self.snapshots.lock() {
            snapshots.expire();
        }
        Some(taken)
    }

    /// Questions nobody answered run out; returns the ones raised since the last frame.
    fn take_fresh_questions(&self) -> Vec<crate::mcp::confirm::PendingView> {
        match self.questions.0.lock() {
            Ok(mut confirmations) => {
                confirmations.tick(Instant::now());
                confirmations.take_fresh()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Lets `command` run only once the user approved this exact request (see `mcp::confirm`);
    /// until then the caller is told the question to put to the user.
    fn gate(
        &self,
        client_id: &str,
        client_name: &str,
        remote_origin: bool,
        command: &McpCommand,
        describe: impl FnOnce() -> Result<crate::mcp::confirm::Summary, McpError>,
    ) -> Result<(), McpError> {
        confirmation::gate_command(
            &self.questions.0,
            Instant::now(),
            client_id,
            client_name,
            remote_origin,
            command,
            describe,
        )
    }
}

impl DegenViewApp {
    /// Applies queued MCP Settings changes and runs the tool requests the server queued. Every
    /// request is checked again just before it runs (see `mcp::bridge::Request::run`).
    pub(super) fn tick_mcp(&mut self, ctx: &egui::Context) {
        /// Requests run per frame; the rest wait for the next one.
        const REQUESTS_PER_FRAME: usize = 8;
        let Some((requests, shared)) = self.mcp.begin_frame(REQUESTS_PER_FRAME) else {
            return;
        };
        if requests.len() == REQUESTS_PER_FRAME {
            ctx.request_repaint();
        }
        for request in requests {
            let client_id = request.client_id.clone();
            request.run(&shared, |command, origin| {
                self.run_gated(ctx, &shared, &client_id, origin.remote, command)
            });
        }
        // Questions nobody answered run out, and a new one is announced where the user can see it.
        let fresh = self.mcp.take_fresh_questions();
        if !fresh.is_empty() {
            self.announce_questions(ctx, &fresh);
        }
    }

    /// Runs `command`, but a high-impact one only once the user has approved this exact request
    /// (see `mcp::confirm`). Until then the caller is told the question to put to the user.
    fn run_gated(
        &self,
        ctx: &egui::Context,
        shared: &McpShared,
        client_id: &str,
        remote_origin: bool,
        command: McpCommand,
    ) -> Result<McpReply, McpError> {
        if crate::mcp::confirm::needs_approval(shared, client_id, remote_origin, &command) {
            let name = shared
                .clients()
                .into_iter()
                .find(|client| client.id == client_id)
                .map(|client| client.name)
                .unwrap_or_default();
            self.mcp
                .gate(client_id, &name, remote_origin, &command, || {
                    self.confirmation_summary(shared, &command)
                })?;
        }
        self.run_mcp_command(ctx, shared, command)
    }

    /// What `command` would do, in words, from what the app knows about its target: its domain
    /// describes it. A target that does not exist is refused with the error the command itself
    /// would give, unasked. A gated command with no description is refused rather than run unasked.
    fn confirmation_summary(
        &self,
        shared: &McpShared,
        command: &McpCommand,
    ) -> Result<crate::mcp::confirm::Summary, McpError> {
        match command {
            McpCommand::Chart(command) => mcp::chart::describe(command, &self.tabs),
            McpCommand::Alert(command) => mcp::alert::describe(command, &self.alert_access()),
            McpCommand::Pine(command) => {
                mcp::pine::describe(command, &self.tabs, &self.store, shared)
            }
            McpCommand::Paper(command) => mcp::paper::describe(command, &self.paper),
            McpCommand::Portfolio(command) => mcp::portfolio::describe(command),
            McpCommand::Watchlist(command) => mcp::watchlist::describe(command, &self.watchlist),
            McpCommand::Replay(command) => mcp::replay::describe(command),
            McpCommand::Snapshot(command) => mcp::snapshot::describe(command),
        }
    }

    /// A new question is up: wake every window so the focused one draws it, and tell the user
    /// outside the app too, if they have notifications on.
    fn announce_questions(&self, ctx: &egui::Context, fresh: &[crate::mcp::confirm::PendingView]) {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        ctx.request_repaint_of(settings_viewport_id());
        self.tabs.repaint_windows(ctx);
        let enabled = self.alerts.host().lock().is_ok_and(|host| {
            crate::model::alert::notification::notifications_enabled(host.settings())
        });
        if !enabled {
            return;
        }
        for view in fresh {
            macos_notifications::notify_async(
                Arc::clone(&self.notifier),
                crate::model::alert::notification::NotificationContent {
                    title: format!("“{}” is asking permission", view.client_name),
                    body: view.summary.line(),
                    identifier: format!("mcp-confirmation-{}", view.id),
                    sound: false,
                    user_info: Vec::new(),
                },
                |_, _| {},
            );
        }
    }

    /// Hands `command` to its domain with the parts of the app it reaches.
    fn run_mcp_command(
        &self,
        ctx: &egui::Context,
        shared: &McpShared,
        command: McpCommand,
    ) -> Result<McpReply, McpError> {
        match command {
            McpCommand::Chart(command) => mcp::chart::run(command, &self.tabs, ctx),
            McpCommand::Alert(command) => mcp::alert::run(command, &self.alert_access(), ctx),
            McpCommand::Pine(command) => mcp::pine::run(command, &self.tabs, &self.store, shared),
            McpCommand::Paper(command) => {
                mcp::paper::run(command, &self.paper, &self.mcp.orders, ctx)
            }
            McpCommand::Portfolio(command) => mcp::portfolio::run(command, &self.portfolio),
            McpCommand::Watchlist(command) => mcp::watchlist::run(command, &self.watchlist, ctx),
            McpCommand::Replay(command) => mcp::replay::run(command, &self.tabs),
            McpCommand::Snapshot(command) => {
                mcp::snapshot::run(command, &self.mcp.snapshots, &self.tabs, ctx)
            }
        }
    }

    /// What the alert commands reach: the alert coordinator's host, prices and Save.
    fn alert_access(&self) -> AlertAccess<'_> {
        AlertAccess {
            host: self.alerts.host(),
            quotes: &self.quote_book,
            fx_rates: self.alerts.fx_rates(),
            store: &self.store,
            save: self.alerts.registration(),
        }
    }
}

impl McpTabs for TabState {
    fn with_workspace<R>(
        &self,
        workspace_id: u64,
        action: impl FnOnce(&mut ChartWorkspace) -> Result<R, McpError>,
    ) -> Result<R, McpError> {
        let not_found = || McpError::NotFound(format!("No workspace {workspace_id}"));
        for tab in self.slots() {
            let mut content = tab
                .content
                .lock()
                .map_err(|_| McpError::AppUnavailable("The workspace is unavailable".into()))?;
            match &mut *content {
                TabContent::Chart(workspace) if workspace.id() == workspace_id => {
                    return action(workspace);
                }
                TabContent::Portfolio(tab) if tab.id() == workspace_id => {
                    return Err(McpError::InvalidArgument(format!(
                        "{workspace_id} is a portfolio tab, not a chart workspace"
                    )));
                }
                TabContent::Pine(manager) if manager.id() == workspace_id => {
                    return Err(McpError::InvalidArgument(format!(
                        "{workspace_id} is the Pine Studio, not a chart workspace"
                    )));
                }
                _ => {}
            }
        }
        Err(not_found())
    }

    fn each_workspace(&self, mut visit: impl FnMut(&ChartWorkspace, bool)) {
        let focused = self.focused_tab();
        for tab in self.slots() {
            let Ok(content) = tab.content.lock() else {
                continue;
            };
            if let TabContent::Chart(workspace) = &*content {
                visit(workspace, workspace.id() == focused);
            }
        }
    }

    fn find_in_pine_studios<R>(
        &self,
        mut find: impl FnMut(&mut PineStudio) -> Option<R>,
    ) -> Option<R> {
        self.slots().iter().find_map(|tab| {
            let mut content = tab.content.lock().ok()?;
            match &mut *content {
                TabContent::Pine(studio) => find(studio),
                _ => None,
            }
        })
    }
}
