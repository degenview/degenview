//! The one-way door from MCP worker threads to the app.
//!
//! A tool never touches workspaces itself: it queues a [`Request`] and awaits the reply. The
//! app thread drains the queue each frame, executes what is still wanted, and answers. A request
//! that was abandoned, outlived its deadline, was queued before the server stopped, or whose
//! client lost the permission in the meantime is answered with an error and never executed — a
//! timed-out call cannot surprise-mutate later.

use crate::mcp::alert::AlertCommand;
use crate::mcp::chart::ChartCommand;
use crate::mcp::confirm::{Confirm, Gate};
use crate::mcp::market::WatchlistCommand;
use crate::mcp::paper::PaperCommand;
use crate::mcp::pine::PineCommand;
use crate::mcp::portfolio::PortfolioCommand;
use crate::mcp::replay::ReplayRequest;
use crate::mcp::snapshot::SnapshotCommand;
use crate::mcp::state::{ClientIdentity, McpShared, Origin};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertChangeResult, AlertsResult, AppliedPineDetail,
    AppliedPineList, CandlesResult, ChartsResult, CreateAlertResult, DrawingsChangeResult,
    DrawingsResult, PineAlertsResult, PineInstanceChangeResult, PineReportResult, PineStylesResult,
    UpdateAlertResult, WorkspaceInfo,
};
use crate::model::mcp::Permission;
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

/// What a tool asks the app thread to do, by domain. Each domain's commands, and what each says
/// about the confirmation step ([`Confirm`]), live with that domain's tools in `mcp::<domain>`;
/// the app runs them in `host::mcp::<domain>`. A new tool in an existing domain adds a variant
/// there, never here.
#[derive(Debug, Clone, PartialEq)]
pub enum McpCommand {
    /// Workspaces, charts, built-in indicators, candles, drawings and chart control.
    Chart(ChartCommand),
    Alert(AlertCommand),
    /// Scripts on charts and script edits.
    Pine(PineCommand),
    Paper(PaperCommand),
    Portfolio(PortfolioCommand),
    Watchlist(WatchlistCommand),
    Replay(ReplayRequest),
    Snapshot(SnapshotCommand),
}

impl McpCommand {
    /// The domain's own command, as the confirmation step reads it.
    fn domain(&self) -> &dyn Confirm {
        match self {
            Self::Chart(command) => command,
            Self::Alert(command) => command,
            Self::Pine(command) => command,
            Self::Paper(command) => command,
            Self::Portfolio(command) => command,
            Self::Watchlist(command) => command,
            Self::Replay(command) => command,
            Self::Snapshot(command) => command,
        }
    }
}

impl Confirm for McpCommand {
    fn gate(&self) -> Gate {
        self.domain().gate()
    }

    fn plain_summary(&self) -> Option<(String, String)> {
        self.domain().plain_summary()
    }
}

/// So a tool can hand [`BridgeClient::call`] its domain's command as it is.
macro_rules! from_domain {
    ($($variant:ident($command:ty)),* $(,)?) => {
        $(impl From<$command> for McpCommand {
            fn from(command: $command) -> Self {
                Self::$variant(command)
            }
        })*
    };
}

from_domain!(
    Chart(ChartCommand),
    Alert(AlertCommand),
    Pine(PineCommand),
    Paper(PaperCommand),
    Portfolio(PortfolioCommand),
    Watchlist(WatchlistCommand),
    Replay(ReplayRequest),
    Snapshot(SnapshotCommand),
);

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
    AlertCommand(crate::mcp::alert::AlertCommandOutcome),
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
    IndicatorValues(Box<crate::mcp::indicator::IndicatorValuesResult>),
    SessionStats(Box<crate::mcp::session_stats::SessionStatsResult>),
    Timeframe(crate::mcp::chart::control::TimeframeResult),
    ChartChanged(Box<crate::mcp::chart::control::ChartChangeResult>),
    PaperAccounts(crate::mcp::paper::PaperAccountsResult),
    PaperAccount(Box<crate::mcp::paper::PaperAccountDetail>),
    PaperFills(Box<crate::mcp::paper::PaperFillsResult>),
    PaperTrades(Box<crate::mcp::paper::PaperTradesResult>),
    PaperOrder(Box<crate::mcp::paper::PaperOrderResult>),
    PaperChange(Box<crate::mcp::paper::PaperChangeResult>),
    Portfolios(crate::mcp::portfolio::PortfoliosResult),
    PortfolioHoldings(Box<crate::mcp::portfolio::HoldingsBase>),
    PortfolioTransactions(Box<crate::mcp::portfolio::TransactionsResult>),
    SnapshotStarted(crate::mcp::snapshot::SnapshotStarted),
    SnapshotProgress(crate::mcp::snapshot::SnapshotProgress),
    Replay(crate::mcp::replay::ReplayResult),
    Watchlists(crate::mcp::market::WatchlistsResult),
    Watchlist(Box<crate::mcp::market::WatchlistPage>),
    WatchlistChanged(Box<crate::mcp::market::WatchlistChangeResult>),
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
        command: impl Into<McpCommand>,
        timeout: Duration,
    ) -> Reply {
        let (reply, answer) = oneshot::channel();
        let state = Arc::new(AtomicU8::new(PENDING));
        let _cancel_if_dropped = CancelOnDrop(state.clone());
        let request = Request {
            client_id: client.id.clone(),
            origin: client.origin(),
            permission,
            command: command.into(),
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
    use crate::mcp::transport::auth::verifier_hex;
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
                        McpCommand::Chart(ChartCommand::ListWorkspaces),
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
            assert_eq!(command, McpCommand::Chart(ChartCommand::ListWorkspaces));
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
                        McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                        McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                    McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                        McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                        McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                        McpCommand::Chart(ChartCommand::ListWorkspaces),
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
                McpCommand::Chart(ChartCommand::ListWorkspaces),
                Duration::from_millis(200),
            )
            .await;
        assert!(matches!(overflow, Err(McpError::AppUnavailable(_))));
        for task in pending {
            let _ = task.await;
        }
    }
}
