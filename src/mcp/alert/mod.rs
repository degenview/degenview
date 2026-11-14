//! The price alert tools (`tools`) and what they hand the app thread ([`AlertCommand`], run in
//! `host::mcp::alert` with the alert sheet's own rules).

pub mod tools;

use crate::mcp::bridge::McpError;
use crate::mcp::confirm::{Confirm, Gate};
use crate::mcp::tools::{CREATE_ALERT, DELETE_ALERT, PAUSE_ALERT, RESUME_ALERT, UPDATE_ALERT};
use crate::model::InstrumentId;
use crate::model::alert::{AlertCurrency, AlertFrequency, AlertId, AlertRejectReason};

#[derive(Debug, Clone, PartialEq)]
pub enum AlertCommand {
    /// One alert by id, or the first page of all of them.
    List {
        only: Option<AlertId>,
    },
    Create(Box<NewAlert>),
    Update(Box<AlertUpdate>),
    /// What became of an alert command the app queued earlier (see [`AlertCommandOutcome`]).
    CommandOutcome {
        command_id: u64,
    },
    SetPaused {
        alert_id: AlertId,
        paused: bool,
    },
    Delete {
        alert_id: AlertId,
    },
}

impl Confirm for AlertCommand {
    fn gate(&self) -> Gate {
        match self {
            // Cannot be undone from here.
            Self::Delete { .. } => Gate::Ask(DELETE_ALERT),
            // Changes, but easy to see and to undo in the app.
            Self::Create(_) => Gate::AskRemote(CREATE_ALERT),
            Self::Update(_) => Gate::AskRemote(UPDATE_ALERT),
            Self::SetPaused { paused: true, .. } => Gate::AskRemote(PAUSE_ALERT),
            Self::SetPaused { paused: false, .. } => Gate::AskRemote(RESUME_ALERT),
            // Reads.
            Self::List { .. } | Self::CommandOutcome { .. } => Gate::Free,
        }
    }

    fn plain_summary(&self) -> Option<(String, String)> {
        Some(match self {
            Self::Create(alert) => (
                "Create a price alert".to_owned(),
                format!(
                    "{} crosses {} {}",
                    alert.instrument.symbol,
                    if alert.crosses_above {
                        "above"
                    } else {
                        "below"
                    },
                    alert.target_text
                ),
            ),
            Self::Update(update) => (
                "Change a price alert".to_owned(),
                format!("alert {}", update.alert_id),
            ),
            Self::SetPaused { alert_id, paused } => (
                if *paused {
                    "Pause a price alert"
                } else {
                    "Resume a price alert"
                }
                .to_owned(),
                format!("alert {alert_id}"),
            ),
            _ => return None,
        })
    }
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

/// The alert runtime applies queued commands on its own schedule and may refuse one whose
/// precondition no longer holds. This is the answer for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertCommandOutcome {
    /// Not applied yet (or too old to be remembered).
    Pending,
    Applied,
    Rejected(AlertRejectReason),
}

impl AlertCommandOutcome {
    /// The error a rejected command is reported as.
    pub fn rejection_error(reason: AlertRejectReason) -> McpError {
        use AlertRejectReason as Reason;
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
