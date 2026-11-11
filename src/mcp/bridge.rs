//! The one-way door from MCP worker threads to the app.
//!
//! A tool never touches workspaces itself: it queues a [`Request`] and awaits the reply. The
//! app thread drains the queue each frame, executes what is still wanted, and answers. A request
//! that was abandoned, outlived its deadline, was queued before the server stopped, or whose
//! client lost the permission in the meantime is answered with an error and never executed — a
//! timed-out call cannot surprise-mutate later.

use crate::mcp::state::{ClientIdentity, McpShared, Origin};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertChangeResult, AlertsResult, AppliedPineDetail,
    AppliedPineList, ChartsResult, CreateAlertResult, PineAlertsResult, PineInstanceChangeResult,
    PineReportResult, PineStylesResult, UpdateAlertResult, WorkspaceInfo,
};
use crate::mcp::types::{CandlesResult, DrawingsChangeResult, DrawingsResult};
use crate::model::alert::{AlertCurrency, AlertFrequency, AlertId};
use crate::model::mcp::Permission;
use crate::model::{BuiltIn, InstrumentId};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// Requests waiting for the app thread. Beyond this the server answers "busy".
const QUEUE_CAPACITY: usize = 32;
/// How long the app thread may take to start a request before it is given up on.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Extra patience for a request the app already started: it has run, so its outcome is real.
const STARTED_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub enum McpCommand {
    ListWorkspaces,
    ListCharts {
        workspace_id: u64,
    },
    AddChart {
        workspace_id: u64,
        instrument: InstrumentId,
    },
    /// One alert by id, or the first page of all of them.
    ListAlerts {
        only: Option<AlertId>,
    },
    CreateAlert(Box<NewAlert>),
    UpdateAlert(Box<AlertUpdate>),
    /// What became of an alert command the app queued earlier (see [`AlertCommandOutcome`]).
    AlertCommandOutcome {
        command_id: u64,
    },
    SetAlertPaused {
        alert_id: AlertId,
        paused: bool,
    },
    DeleteAlert {
        alert_id: AlertId,
    },
    ApplyIndicator {
        workspace_id: u64,
        chart_id: u64,
        indicator: BuiltIn,
        ema_period: Option<usize>,
    },
    ListChartPine {
        workspace_id: u64,
        chart_id: u64,
    },
    GetChartPine {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
    },
    GetPineStyles {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
    },
    GetPineReport {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
        offset: usize,
        limit: usize,
        include_equity: bool,
    },
    ListPineAlerts,
    /// Applies a script (its checked saved text and input overrides) to a chart.
    AddPineInstance {
        workspace_id: u64,
        chart_id: u64,
        script: Box<crate::mcp::pine::ScriptToApply>,
        visible: bool,
    },
    /// Changes an applied script's inputs (`None` clears that input) and/or visibility.
    UpdatePineInstance {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
        inputs: Option<crate::mcp::pine::InputArgs>,
        visible: Option<bool>,
    },
    RemovePineInstance {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
    },
    /// The newest `limit` candles of a chart, or those before `before` (epoch ms).
    GetCandles {
        workspace_id: u64,
        chart_id: u64,
        limit: usize,
        before: Option<i64>,
    },
    ListDrawings {
        workspace_id: u64,
        chart_id: u64,
    },
    AddDrawings {
        workspace_id: u64,
        chart_id: u64,
        drawings: Vec<crate::mcp::drawings::NewDrawing>,
    },
    EditDrawing {
        workspace_id: u64,
        chart_id: u64,
        edit: Box<crate::mcp::drawings::DrawingEdit>,
    },
    RemoveDrawings {
        workspace_id: u64,
        chart_id: u64,
        drawing_ids: Vec<u64>,
    },
    /// Edit a script's text. `source: Some` writes it as the draft; `None` saves the current
    /// draft over the saved source. Either way `expected_revision` must match the working text.
    EditPineScript {
        script_id: String,
        expected_revision: String,
        source: Option<String>,
    },
    /// Takes a built-in indicator off a chart.
    RemoveIndicator {
        workspace_id: u64,
        chart_id: u64,
        indicator: BuiltIn,
    },
    /// Built-in indicator readings over a chart's candles.
    GetIndicatorValues {
        workspace_id: u64,
        chart_id: u64,
        request: crate::mcp::indicator_values::ReadingRequest,
    },
    /// Session statistics over a chart's candles.
    GetSessionStats {
        workspace_id: u64,
        chart_id: u64,
        query: crate::mcp::session_stats::Query,
    },
    /// Moves the whole workspace to another timeframe.
    SetTimeframe {
        workspace_id: u64,
        timeframe: crate::model::Timeframe,
    },
    SetChartType {
        workspace_id: u64,
        chart_id: u64,
        chart_type: crate::model::ChartType,
    },
    RemoveChart {
        workspace_id: u64,
        chart_id: u64,
    },
    ListPaperAccounts,
    GetPaperAccount {
        account_id: u64,
    },
    /// Fills (`closed_trades: false`) or closed trades of an account, newest first.
    ListPaperHistory {
        account_id: u64,
        closed_trades: bool,
        offset: usize,
        limit: usize,
    },
    /// Places a paper order (or only checks it, for a dry run). `request_id` makes a retry
    /// return the order already placed instead of placing another.
    PlacePaperOrder {
        request: Box<crate::model::paper::PaperOrderRequest>,
        dry_run: bool,
        request_id: Option<String>,
    },
    ModifyPaperOrder {
        order_id: u64,
        changes: crate::model::paper::PaperOrderChanges,
    },
    /// Cancels the named working orders, or every working order of the account.
    CancelPaperOrders {
        account_id: u64,
        order_ids: Option<Vec<u64>>,
    },
    /// Closes one position (by instrument key) or every open position of the account.
    ClosePaperPositions {
        account_id: u64,
        instrument_key: Option<String>,
    },
    ListPortfolios,
    /// The ledger side of a holdings read: one portfolio, or every unarchived one.
    GetPortfolioHoldings {
        portfolio_id: Option<u64>,
    },
    ListPortfolioTransactions {
        portfolio_id: Option<u64>,
        asset: Option<String>,
        offset: usize,
        limit: usize,
    },
    /// Starts a picture of a chart; the answer says which job to ask about.
    BeginSnapshot {
        workspace_id: u64,
        chart_id: u64,
        scale: crate::model::chart::snapshot::SnapshotScale,
    },
    /// Whether the picture a job is making is ready, and takes it if so.
    PollSnapshot {
        job_id: u64,
    },
    GetReplay {
        workspace_id: u64,
    },
    ControlReplay {
        workspace_id: u64,
        command: crate::workspace::ReplayCommand,
    },
    ListWatchlists,
    /// One page of a watchlist's entries. Quotes are not read here (the tool waits for them
    /// off the app thread); the reply names the markets on the page.
    GetWatchlist {
        watchlist_id: u64,
        offset: usize,
        limit: usize,
    },
    /// A new list, optionally with markets already in it.
    CreateWatchlist {
        name: String,
        markets: Vec<crate::model::watchlist::WatchlistInstrument>,
    },
    /// Renames a list, edits its sections and adds, removes and moves its markets, all or
    /// nothing.
    UpdateWatchlist {
        watchlist_id: u64,
        changes: Box<crate::store::watchlist::ListChanges>,
    },
    DeleteWatchlist {
        watchlist_id: u64,
    },
}

/// A validated request to create a price alert. The target stays text: the alert editor parses
/// it the same way it parses what a user types.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAlert {
    pub instrument: InstrumentId,
    pub crosses_above: bool,
    pub target_text: String,
    pub currency: AlertCurrency,
    pub frequency: AlertFrequency,
    pub note: String,
}

/// Changes to an existing price alert; `None` leaves that setting as it is. The market cannot
/// be changed — that is a different alert.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertUpdate {
    pub alert_id: AlertId,
    pub crosses_above: Option<bool>,
    pub target_text: Option<String>,
    pub currency: Option<AlertCurrency>,
    pub frequency: Option<AlertFrequency>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpReply {
    Workspaces(Vec<WorkspaceInfo>),
    Charts(ChartsResult),
    ChartAdded(AddChartResult),
    IndicatorApplied(AddIndicatorResult),
    Alerts(AlertsResult),
    AlertCreated(CreateAlertResult),
    AlertUpdated(UpdateAlertResult),
    AlertChanged(AlertChangeResult),
    AlertCommand(AlertCommandOutcome),
    ChartPine(AppliedPineList),
    ChartPineDetail(AppliedPineDetail),
    PineStyles(PineStylesResult),
    PineReport(Box<PineReportResult>),
    PineAlerts(PineAlertsResult),
    PineInstanceChanged(Box<PineInstanceChangeResult>),
    Candles(Box<CandlesResult>),
    Drawings(Box<DrawingsResult>),
    DrawingsChanged(Box<DrawingsChangeResult>),
    PineEdit(Box<crate::mcp::types::PineEditResult>),
    IndicatorValues(Box<crate::mcp::indicator_values::IndicatorValuesResult>),
    SessionStats(Box<crate::mcp::session_stats::SessionStatsResult>),
    Timeframe(crate::mcp::chart_control::TimeframeResult),
    ChartChanged(Box<crate::mcp::chart_control::ChartChangeResult>),
    PaperAccounts(crate::mcp::paper::PaperAccountsResult),
    PaperAccount(Box<crate::mcp::paper::PaperAccountDetail>),
    PaperFills(Box<crate::mcp::paper::PaperFillsResult>),
    PaperTrades(Box<crate::mcp::paper::PaperTradesResult>),
    PaperOrder(Box<crate::mcp::paper::PaperOrderResult>),
    PaperChange(Box<crate::mcp::paper::PaperChangeResult>),
    Portfolios(crate::mcp::portfolio::PortfoliosResult),
    PortfolioHoldings(Box<crate::mcp::portfolio::HoldingsBase>),
    PortfolioTransactions(Box<crate::mcp::portfolio::TransactionsResult>),
    SnapshotStarted(crate::mcp::snapshot_tools::SnapshotStarted),
    SnapshotProgress(crate::mcp::snapshot_tools::SnapshotProgress),
    Replay(crate::mcp::replay_tools::ReplayResult),
    Watchlists(crate::mcp::markets::WatchlistsResult),
    Watchlist(Box<crate::mcp::markets::WatchlistPage>),
    WatchlistChanged(Box<crate::mcp::markets::WatchlistChangeResult>),
}

/// The alert runtime applies queued commands on its own schedule and may refuse one whose
/// precondition no longer holds. This is the answer for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertCommandOutcome {
    /// Not applied yet (or too old to be remembered).
    Pending,
    Applied,
    Rejected(crate::model::alert::AlertRejectReason),
}

impl AlertCommandOutcome {
    /// The error a rejected command is reported as.
    pub fn rejection_error(reason: crate::model::alert::AlertRejectReason) -> McpError {
        use crate::model::alert::AlertRejectReason as Reason;
        match reason {
            Reason::Duplicate => McpError::Conflict(
                "An identical price alert already exists (another request created it first); nothing was changed. List the alerts and retry".into(),
            ),
            Reason::LimitReached => McpError::ResourceLimit(
                "The price alert limit was reached by other requests first; nothing was created".into(),
            ),
            Reason::Missing => McpError::Conflict(
                "The price alert was deleted before this change could be applied; nothing was changed".into(),
            ),
            Reason::Changed => McpError::Conflict(
                "The price alert was changed by another request or in DegenView before this change could be applied; nothing was changed. Read it again and retry".into(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpError {
    NotFound(String),
    InvalidArgument(String),
    Conflict(String),
    Unsupported(String),
    CompilationFailed(String),
    PermissionDenied(String),
    ResourceLimit(String),
    Timeout,
    AppUnavailable(String),
    /// The user has not approved this yet; nothing ran. Repeat the call once they have.
    ConfirmationRequired(crate::mcp::confirm::PendingInfo),
    /// The user declined this exact call; the text says what it was.
    ConfirmationDenied(String),
}

impl McpError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "NOT_FOUND",
            Self::InvalidArgument(_) => "INVALID_ARGUMENT",
            Self::Conflict(_) => "CONFLICT",
            Self::Unsupported(_) => "UNSUPPORTED",
            Self::CompilationFailed(_) => "COMPILATION_FAILED",
            Self::PermissionDenied(_) => "PERMISSION_DENIED",
            Self::ResourceLimit(_) => "RESOURCE_LIMIT",
            Self::Timeout => "TIMEOUT",
            Self::AppUnavailable(_) => "APP_UNAVAILABLE",
            Self::ConfirmationRequired(_) => "CONFIRMATION_REQUIRED",
            Self::ConfirmationDenied(_) => "CONFIRMATION_DENIED",
        }
    }

    /// Fields a client can act on beyond the message, merged into the structured result.
    pub fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::ConfirmationRequired(pending) => Some(serde_json::json!({
                "confirmation_id": pending.id.to_string(),
                "expires_in_seconds": pending.expires_in_secs,
                "operation": pending.operation,
                "next_step": "Ask the user to approve it in DegenView, then repeat this exact call.",
            })),
            Self::ConfirmationDenied(operation) => Some(serde_json::json!({
                "operation": operation,
                "next_step": "Do not repeat it unless the user asks you to.",
            })),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound(message)
            | Self::InvalidArgument(message)
            | Self::Conflict(message)
            | Self::Unsupported(message)
            | Self::CompilationFailed(message)
            | Self::PermissionDenied(message)
            | Self::ResourceLimit(message)
            | Self::AppUnavailable(message) => message.clone(),
            Self::Timeout => {
                "DegenView did not respond in time; the request was not applied".into()
            }
            Self::ConfirmationRequired(pending) => format!(
                "Needs the user's approval in DegenView first, so nothing was changed: {}. \
                 It waits {} seconds for an answer. Ask the user to approve it, then repeat this exact call.",
                pending.operation, pending.expires_in_secs
            ),
            Self::ConfirmationDenied(operation) => format!(
                "The user declined this in DegenView, so nothing was changed: {operation}. \
                 Do not repeat it unless the user asks you to."
            ),
        }
    }
}

const PENDING: u8 = 0;
const STARTED: u8 = 1;
const CANCELLED: u8 = 2;

type Reply = Result<McpReply, McpError>;

/// Cancels a queued request when the call that queued it is dropped. A caller can disappear
/// without reaching its own timeout (the client closed the stream, the request was cancelled,
/// an outer future was aborted); the queued [`Request`] holds its own handle to the state and
/// would otherwise still run. The switch only moves from `PENDING`, so once the app has
/// started a request this does nothing and its real outcome stands.
struct CancelOnDrop(Arc<AtomicU8>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let _ = self
            .0
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire);
    }
}

pub struct Request {
    pub client_id: String,
    /// How the call arrived, judged again when it runs.
    pub origin: Origin,
    pub permission: Permission,
    pub command: McpCommand,
    deadline: Instant,
    generation: u64,
    state: Arc<AtomicU8>,
    reply: oneshot::Sender<Reply>,
}

impl Request {
    /// Runs `execute` unless the request is no longer wanted, and answers the caller. `execute` is
    /// given the command and the origin the request arrived with.
    pub fn run(self, shared: &McpShared, execute: impl FnOnce(McpCommand, Origin) -> Reply) {
        let Request {
            client_id,
            origin,
            permission,
            command,
            deadline,
            generation,
            state,
            reply,
        } = self;
        let refusal = if Instant::now() > deadline || reply.is_closed() {
            // Past its deadline, or the caller dropped its end (see `CancelOnDrop`).
            Some(McpError::Timeout)
        } else if generation != shared.generation() {
            Some(McpError::AppUnavailable(
                "The MCP server was stopped".into(),
            ))
        } else if !shared.allows(&client_id, permission) {
            Some(McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
            ))
        } else if !shared.accepts_origin(&client_id, &origin) {
            Some(McpError::PermissionDenied(
                "This credential can no longer be used from where this request came from".into(),
            ))
        } else {
            None
        };
        if let Some(error) = refusal {
            // Whoever wins this race decides: the caller may already have given up.
            state
                .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                .ok();
            let _ = reply.send(Err(error));
            return;
        }
        // The caller cancels by the same switch; if it got there first, do nothing.
        if state
            .compare_exchange(PENDING, STARTED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            let _ = reply.send(Err(McpError::Timeout));
            return;
        }
        let _ = reply.send(execute(command, origin));
    }
}

/// The worker side: queues requests and awaits their answers.
#[derive(Clone)]
pub struct BridgeClient {
    sender: SyncSender<Request>,
    wake: Arc<dyn Fn() + Send + Sync>,
    shared: Arc<McpShared>,
}

impl BridgeClient {
    pub async fn call(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        command: McpCommand,
        timeout: Duration,
    ) -> Reply {
        let (reply, answer) = oneshot::channel();
        let state = Arc::new(AtomicU8::new(PENDING));
        let _cancel_if_dropped = CancelOnDrop(state.clone());
        let request = Request {
            client_id: client.id.clone(),
            origin: client.origin(),
            permission,
            command,
            deadline: Instant::now() + timeout,
            generation: self.shared.generation(),
            state: state.clone(),
            reply,
        };
        if self.sender.try_send(request).is_err() {
            return Err(McpError::AppUnavailable(
                "DegenView is busy; try again".into(),
            ));
        }
        (self.wake)();
        let mut answer = answer;
        match tokio::time::timeout(timeout, &mut answer).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(McpError::AppUnavailable(
                "DegenView is shutting down".into(),
            )),
            Err(_) => {
                // Cancel if the app has not started it: then it is certain not to run.
                if state
                    .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Err(McpError::Timeout);
                }
                // Already started: it will finish, and its real outcome is what we report.
                match tokio::time::timeout(STARTED_GRACE, answer).await {
                    Ok(Ok(reply)) => reply,
                    _ => Err(McpError::Timeout),
                }
            }
        }
    }
}

/// The app side: drained once per frame.
pub struct BridgeInbox {
    receiver: Receiver<Request>,
}

impl BridgeInbox {
    /// Up to `max` queued requests, oldest first. Never blocks.
    pub fn take(&self, max: usize) -> Vec<Request> {
        let mut taken = Vec::new();
        while taken.len() < max {
            match self.receiver.try_recv() {
                Ok(request) => taken.push(request),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        taken
    }
}

/// A connected pair. `wake` is called after each queued request so the UI runs a frame.
pub fn channel(
    shared: Arc<McpShared>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> (BridgeClient, BridgeInbox) {
    let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
    (
        BridgeClient {
            sender,
            wake,
            shared,
        },
        BridgeInbox { receiver },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::auth::verifier_hex;
    use crate::model::mcp::{McpClient, PermissionSet};
    use std::sync::atomic::AtomicUsize;

    fn shared_with_client(permissions: &[Permission]) -> Arc<McpShared> {
        let shared = McpShared::new();
        shared.replace(
            vec![McpClient {
                id: "c1".into(),
                name: "c".into(),
                verifier: verifier_hex("t"),
                permissions: PermissionSet::of(permissions.iter().copied()),
                created_at_ms: 0,
                last_used_ms: None,
                remote_allowed: false,
                allowed_cidrs: Vec::new(),
            }],
            PermissionSet::of(Permission::ALL),
        );
        shared
    }

    fn setup(
        permissions: &[Permission],
    ) -> (Arc<McpShared>, BridgeClient, BridgeInbox, Arc<AtomicUsize>) {
        let shared = shared_with_client(permissions);
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let (client, inbox) = channel(
            shared.clone(),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        (shared, client, inbox, wakes)
    }

    /// A client on this machine, as the bearer check would have identified it.
    fn identity(id: &str) -> ClientIdentity {
        ClientIdentity {
            id: id.into(),
            name: "c".into(),
            peer: crate::model::mcp::Peer::from_ip(std::net::IpAddr::V4(
                std::net::Ipv4Addr::LOCALHOST,
            )),
            remote: false,
        }
    }

    fn ok_reply(_: McpCommand) -> Reply {
        Ok(McpReply::Workspaces(Vec::new()))
    }

    #[tokio::test]
    async fn a_request_is_executed_once_and_answered() {
        let (shared, client, inbox, wakes) = setup(&[Permission::ChartsRead]);
        let call = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .call(
                        &identity("c1"),
                        Permission::ChartsRead,
                        McpCommand::ListWorkspaces,
                        Duration::from_secs(5),
                    )
                    .await
            }
        });
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        let runs = Arc::new(AtomicUsize::new(0));
        let seen = runs.clone();
        request.run(&shared, move |command, _origin| {
            seen.fetch_add(1, Ordering::SeqCst);
            assert_eq!(command, McpCommand::ListWorkspaces);
            ok_reply(command)
        });
        assert_eq!(call.await.unwrap(), Ok(McpReply::Workspaces(Vec::new())));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    /// A request from the host at `peer` (not this machine) with a credential that may be used
    /// remotely, queued and not yet run; hands back what is needed to run it.
    async fn queued_from_afar(
        permissions: &[Permission],
    ) -> (
        Arc<McpShared>,
        tokio::task::JoinHandle<Reply>,
        Request,
        ClientIdentity,
    ) {
        let (shared, client, inbox, _) = setup(permissions);
        let mut clients = shared.clients();
        clients[0].remote_allowed = true;
        clients[0].allowed_cidrs = vec![crate::model::mcp::Cidr::parse("192.168.1.0/24").unwrap()];
        shared.replace(clients, PermissionSet::of(Permission::ALL));
        let identity = shared
            .authenticate_from("t", "192.168.1.9".parse().ok())
            .expect("the remote credential is let in from its range");
        assert!(identity.remote && !identity.peer.local);
        let call = tokio::spawn({
            let (client, identity) = (client.clone(), identity.clone());
            async move {
                client
                    .call(
                        &identity,
                        Permission::ChartsRead,
                        McpCommand::ListWorkspaces,
                        Duration::from_secs(5),
                    )
                    .await
            }
        });
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        (shared, call, request, identity)
    }

    #[tokio::test]
    async fn a_queued_request_keeps_the_origin_it_arrived_with_when_remote_access_is_switched_off()
    {
        // Remote access is taken away from the credential between the call and the moment it runs.
        // The request still arrived as remote, so the gate must be told so; but its address is no
        // longer admitted, so it does not run at all (see the next test).
        let (shared, _call, request, _identity) = queued_from_afar(&[Permission::ChartsRead]).await;
        assert!(request.origin.remote);
        let mut clients = shared.clients();
        clients[0].remote_allowed = false;
        shared.replace(clients, PermissionSet::of(Permission::ALL));
        assert!(
            !shared.is_remote_client("c1"),
            "as the credential is now: not remote"
        );
        assert!(request.origin.remote, "as the request came in: remote");
    }

    #[tokio::test]
    async fn a_request_from_an_address_the_credential_no_longer_admits_does_not_run() {
        let (shared, call, request, _identity) = queued_from_afar(&[Permission::ChartsRead]).await;
        // Remote access switched off while it waited.
        let mut clients = shared.clients();
        clients[0].remote_allowed = false;
        shared.replace(clients, PermissionSet::of(Permission::ALL));
        request.run(&shared, |_, _| {
            panic!("it must not run from an address no longer admitted")
        });
        match call.await.unwrap() {
            Err(McpError::PermissionDenied(message)) => {
                assert!(message.contains("can no longer be used"), "{message}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_request_whose_range_was_narrowed_away_while_it_waited_does_not_run() {
        let (shared, call, request, _identity) = queued_from_afar(&[Permission::ChartsRead]).await;
        let mut clients = shared.clients();
        clients[0].allowed_cidrs = vec![crate::model::mcp::Cidr::parse("10.0.0.0/8").unwrap()];
        shared.replace(clients, PermissionSet::of(Permission::ALL));
        request.run(&shared, |_, _| panic!("outside the allowed range now"));
        assert!(matches!(
            call.await.unwrap(),
            Err(McpError::PermissionDenied(_))
        ));
    }

    #[tokio::test]
    async fn a_request_still_admitted_runs_and_is_given_its_origin() {
        let (shared, call, request, _identity) = queued_from_afar(&[Permission::ChartsRead]).await;
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        request.run(&shared, move |command, origin| {
            counter.fetch_add(1, Ordering::SeqCst);
            assert!(origin.remote && !origin.peer.local);
            ok_reply(command)
        });
        assert!(call.await.unwrap().is_ok());
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_local_request_is_unaffected_by_the_origin_check() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let local = identity("c1");
        let call = tokio::spawn({
            let client = client.clone();
            let local = local.clone();
            async move {
                client
                    .call(
                        &local,
                        Permission::ChartsRead,
                        McpCommand::ListWorkspaces,
                        Duration::from_secs(5),
                    )
                    .await
            }
        });
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        assert!(!request.origin.remote);
        request.run(&shared, |command, _| ok_reply(command));
        assert!(call.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn a_timed_out_request_is_never_executed_later() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let result = client
            .call(
                &identity("c1"),
                Permission::ChartsRead,
                McpCommand::ListWorkspaces,
                Duration::from_millis(30),
            )
            .await;
        assert_eq!(result, Err(McpError::Timeout));
        // The app only gets to it afterwards: it must not run.
        let request = inbox.take(1).pop().expect("still queued");
        request.run(&shared, |_, _| {
            panic!("an abandoned request must not execute")
        });
    }

    fn spawn_call(client: &BridgeClient) -> tokio::task::JoinHandle<Reply> {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .call(
                    &identity("c1"),
                    Permission::ChartsRead,
                    McpCommand::ListWorkspaces,
                    Duration::from_secs(30),
                )
                .await
        })
    }

    #[tokio::test]
    async fn a_dropped_call_is_never_executed_later() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let call = spawn_call(&client);
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        // The caller goes away long before its own timeout: stream closed, call cancelled.
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        request.run(&shared, |_, _| {
            panic!("a request whose caller was dropped must not execute")
        });
    }

    #[tokio::test]
    async fn a_dropped_call_queued_behind_others_is_skipped_but_the_others_run() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let first = spawn_call(&client);
        let second = spawn_call(&client);
        let third = spawn_call(&client);
        let mut requests = Vec::new();
        while requests.len() < 3 {
            requests.extend(inbox.take(3));
            tokio::task::yield_now().await;
        }
        // Whichever was queued second is abandoned; the abort lands at the next yield.
        second.abort();
        assert!(second.await.unwrap_err().is_cancelled());
        let runs = Arc::new(AtomicUsize::new(0));
        for request in requests {
            let runs = runs.clone();
            request.run(&shared, move |command, _origin| {
                runs.fetch_add(1, Ordering::SeqCst);
                ok_reply(command)
            });
        }
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert!(first.await.unwrap().is_ok());
        assert!(third.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn dropping_a_call_after_the_app_started_it_keeps_its_outcome() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let call = spawn_call(&client);
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        let ran = Arc::new(AtomicUsize::new(0));
        let seen = ran.clone();
        request.run(&shared, move |command, _origin| {
            seen.fetch_add(1, Ordering::SeqCst);
            ok_reply(command)
        });
        call.abort();
        let _ = call.await;
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn requests_queued_before_a_stop_do_not_run() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let call = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .call(
                        &identity("c1"),
                        Permission::ChartsRead,
                        McpCommand::ListWorkspaces,
                        Duration::from_secs(5),
                    )
                    .await
            }
        });
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        shared.bump_generation();
        request.run(&shared, |_, _| {
            panic!("must not execute after the server stopped")
        });
        assert!(matches!(
            call.await.unwrap(),
            Err(McpError::AppUnavailable(_))
        ));
    }

    #[tokio::test]
    async fn permission_revoked_while_queued_is_enforced_at_execution() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsCreate]);
        let call = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .call(
                        &identity("c1"),
                        Permission::ChartsCreate,
                        McpCommand::ListWorkspaces,
                        Duration::from_secs(5),
                    )
                    .await
            }
        });
        let request = loop {
            if let Some(request) = inbox.take(1).pop() {
                break request;
            }
            tokio::task::yield_now().await;
        };
        shared.replace(Vec::new(), PermissionSet::of(Permission::ALL)); // credential revoked
        request.run(&shared, |_, _| panic!("revoked client must not execute"));
        assert!(matches!(
            call.await.unwrap(),
            Err(McpError::PermissionDenied(_))
        ));
    }

    #[tokio::test]
    async fn a_full_queue_answers_busy_instead_of_blocking() {
        let (_shared, client, _inbox, _) = setup(&[Permission::ChartsRead]);
        let mut pending = Vec::new();
        for _ in 0..QUEUE_CAPACITY {
            let client = client.clone();
            pending.push(tokio::spawn(async move {
                client
                    .call(
                        &identity("c1"),
                        Permission::ChartsRead,
                        McpCommand::ListWorkspaces,
                        Duration::from_millis(200),
                    )
                    .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let overflow = client
            .call(
                &identity("c1"),
                Permission::ChartsRead,
                McpCommand::ListWorkspaces,
                Duration::from_millis(200),
            )
            .await;
        assert!(matches!(overflow, Err(McpError::AppUnavailable(_))));
        for task in pending {
            let _ = task.await;
        }
    }
}
