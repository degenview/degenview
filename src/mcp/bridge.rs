//! The one-way door from MCP worker threads to the app.
//!
//! A tool never touches workspaces itself: it queues a [`Request`] and awaits the reply. The
//! app thread drains the queue each frame, executes what is still wanted, and answers. A request
//! that was abandoned, outlived its deadline, was queued before the server stopped, or whose
//! client lost the permission in the meantime is answered with an error and never executed — a
//! timed-out call cannot surprise-mutate later.

use crate::mcp::state::McpShared;
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertChangeResult, AlertsResult, AppliedPineDetail,
    AppliedPineList, ChartsResult, CreateAlertResult, PineAlertsResult, PineInstanceChangeResult,
    PineReportResult, PineStylesResult, UpdateAlertResult, WorkspaceInfo,
};
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
    /// Edit a script's text. `source: Some` writes it as the draft; `None` saves the current
    /// draft over the saved source. Either way `expected_revision` must match the working text.
    EditPineScript {
        script_id: String,
        expected_revision: String,
        source: Option<String>,
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
    ChartPine(AppliedPineList),
    ChartPineDetail(AppliedPineDetail),
    PineStyles(PineStylesResult),
    PineReport(Box<PineReportResult>),
    PineAlerts(PineAlertsResult),
    PineInstanceChanged(Box<PineInstanceChangeResult>),
    PineEdit(Box<crate::mcp::types::PineEditResult>),
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
        }
    }
}

const PENDING: u8 = 0;
const STARTED: u8 = 1;
const CANCELLED: u8 = 2;

type Reply = Result<McpReply, McpError>;

pub struct Request {
    pub client_id: String,
    pub permission: Permission,
    pub command: McpCommand,
    deadline: Instant,
    generation: u64,
    state: Arc<AtomicU8>,
    reply: oneshot::Sender<Reply>,
}

impl Request {
    /// Runs `execute` unless the request is no longer wanted, and answers the caller.
    pub fn run(self, shared: &McpShared, execute: impl FnOnce(McpCommand) -> Reply) {
        let Request {
            client_id,
            permission,
            command,
            deadline,
            generation,
            state,
            reply,
        } = self;
        let refusal = if Instant::now() > deadline {
            Some(McpError::Timeout)
        } else if generation != shared.generation() {
            Some(McpError::AppUnavailable(
                "The MCP server was stopped".into(),
            ))
        } else if !shared.allows(&client_id, permission) {
            Some(McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
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
        let _ = reply.send(execute(command));
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
        client_id: &str,
        permission: Permission,
        command: McpCommand,
        timeout: Duration,
    ) -> Reply {
        let (reply, answer) = oneshot::channel();
        let state = Arc::new(AtomicU8::new(PENDING));
        let request = Request {
            client_id: client_id.to_owned(),
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
                        "c1",
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
        request.run(&shared, move |command| {
            seen.fetch_add(1, Ordering::SeqCst);
            assert_eq!(command, McpCommand::ListWorkspaces);
            ok_reply(command)
        });
        assert_eq!(call.await.unwrap(), Ok(McpReply::Workspaces(Vec::new())));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_timed_out_request_is_never_executed_later() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let result = client
            .call(
                "c1",
                Permission::ChartsRead,
                McpCommand::ListWorkspaces,
                Duration::from_millis(30),
            )
            .await;
        assert_eq!(result, Err(McpError::Timeout));
        // The app only gets to it afterwards: it must not run.
        let request = inbox.take(1).pop().expect("still queued");
        request.run(&shared, |_| panic!("an abandoned request must not execute"));
    }

    #[tokio::test]
    async fn requests_queued_before_a_stop_do_not_run() {
        let (shared, client, inbox, _) = setup(&[Permission::ChartsRead]);
        let call = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .call(
                        "c1",
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
        request.run(&shared, |_| {
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
                        "c1",
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
        request.run(&shared, |_| panic!("revoked client must not execute"));
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
                        "c1",
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
                "c1",
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
