//! The confirmation step for high-impact tool calls.
//!
//! Some commands cannot be undone from here (a delete, a saved script) or move money, even
//! simulated money (a paper order). With "Require confirmation" on, the app asks the user before
//! it runs one. The step never parks a worker waiting for a person: the first call answers
//! `CONFIRMATION_REQUIRED` straight away with an id and an expiry, the user approves or declines
//! in the app, and the agent repeats the exact same call, which then runs once.
//!
//! This module is the egui-free half: which commands are gated ([`gate`], an exhaustive match so a
//! new command cannot be added without deciding), and the [`ConfirmationBook`] of what is waiting
//! for an answer. An approval is tied to the client and to the exact command, not to a tool name:
//! a repeat with any argument changed is a different operation and asks again. An approval is
//! used up by the call that runs it, and only if what it would do is still what the user was
//! shown: a summary carries the resolved target (the orders an "all" reaches), and a repeat whose
//! summary has changed asks again.
//!
//! The user reads the whole summary. The client is told only what it asked for ("Delete a price
//! alert"), never what the app found out about the target, because a credential that may delete
//! alerts may not be allowed to read them (see [`Summary::client_line`]).
//!
//! All times are passed in, so tests never sleep.

use crate::mcp::bridge::McpCommand;
use crate::mcp::state::McpShared;
use std::time::{Duration, Instant};

/// How long a question waits for an answer.
pub const PENDING_TTL: Duration = Duration::from_secs(120);
/// How long an approval waits for the agent to repeat its call.
pub const APPROVED_TTL: Duration = Duration::from_secs(120);
/// How long a "no" is remembered, so an agent that repeats the call is told, not asked again.
pub const DENIED_TTL: Duration = Duration::from_secs(300);
/// Questions one client may have open at once.
pub const MAX_PENDING_PER_CLIENT: usize = 8;
/// Questions open at once, across clients.
pub const MAX_PENDING_TOTAL: usize = 32;
/// Longest a summary line is kept, so a long name cannot make a dialog unreadable.
pub const SUMMARY_LIMIT: usize = 140;

/// Whether a command needs the user's approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Runs as soon as the permission allows it.
    Free,
    /// Asks the user first (unless the user turned confirmation off).
    Ask,
    /// Changes something, but is easy to see and to undo in the app: runs unasked for a local
    /// credential, and asks for one with remote access (see [`needs_approval`]).
    AskRemote,
}

/// Whether `client_id`'s `command` has to be approved by the user first.
///
/// A credential with remote access asks for everything that changes anything, and the "ask before
/// high-impact requests" setting does not apply to it: someone who can reach the server from
/// another machine is not given a way to switch the question off from here. A local credential
/// asks for the high-impact commands only, and not at all when the setting is off.
///
/// `remote_origin` is how the request arrived (`ClientIdentity::remote`), fixed when it was
/// authenticated: a call that came in as remote keeps asking even if remote access is switched off
/// while it waits to run. The credential's present state counts too, so neither can lower it.
pub fn needs_approval(
    shared: &McpShared,
    client_id: &str,
    remote_origin: bool,
    command: &McpCommand,
) -> bool {
    let remote = remote_origin || shared.is_remote_client(client_id);
    match gate(command) {
        Gate::Free => false,
        Gate::Ask => remote || shared.require_confirmation(),
        Gate::AskRemote => remote,
    }
}

/// The decision for every command. An exhaustive match on purpose: adding a command to
/// `McpCommand` fails to compile here until someone decides whether it needs approval.
pub fn gate(command: &McpCommand) -> Gate {
    use McpCommand as C;
    match command {
        // Cannot be undone from here.
        C::DeleteAlert { .. }
        | C::DeleteWatchlist { .. }
        | C::RemoveDrawings { .. }
        | C::RemoveChart { .. }
        | C::RemovePineInstance { .. }
        // Overwrites the saved source. Saving is the step that counts for a local credential.
        | C::EditPineScript { source: None, .. }
        // Simulated money, but the orders are real to the paper account. A dry run only checks.
        | C::PlacePaperOrder { dry_run: false, .. }
        | C::ModifyPaperOrder { .. }
        | C::CancelPaperOrders { .. }
        | C::ClosePaperPositions { .. } => Gate::Ask,

        // Writing a draft changes what is in the editor, and is free for a local credential.
        C::EditPineScript { source: Some(_), .. } => Gate::AskRemote,
        // Only checks; nothing is placed.
        C::PlacePaperOrder { dry_run: true, .. } => Gate::Free,

        // Reads.
        C::ListWorkspaces
        | C::ListCharts { .. }
        | C::ListAlerts { .. }
        | C::AlertCommandOutcome { .. }
        | C::ListChartPine { .. }
        | C::GetChartPine { .. }
        | C::GetPineStyles { .. }
        | C::GetPineReport { .. }
        | C::ListPineAlerts
        | C::GetCandles { .. }
        | C::ListDrawings { .. }
        | C::GetIndicatorValues { .. }
        | C::GetSessionStats { .. }
        | C::ListPaperAccounts
        | C::GetPaperAccount { .. }
        | C::ListPaperHistory { .. }
        | C::ListPortfolios
        | C::GetPortfolioHoldings { .. }
        | C::ListPortfolioTransactions { .. }
        | C::BeginSnapshot { .. }
        | C::PollSnapshot { .. }
        | C::GetReplay { .. }
        | C::ListWatchlists
        | C::GetWatchlist { .. } => Gate::Free,

        // Changes, but easy to see and to undo in the app.
        C::AddChart { .. }
        | C::CreateAlert(_)
        | C::UpdateAlert(_)
        | C::SetAlertPaused { .. }
        | C::ApplyIndicator { .. }
        | C::RemoveIndicator { .. }
        | C::AddPineInstance { .. }
        | C::UpdatePineInstance { .. }
        | C::AddDrawings { .. }
        | C::EditDrawing { .. }
        | C::SetTimeframe { .. }
        | C::SetChartType { .. }
        | C::ControlReplay { .. }
        | C::CreateWatchlist { .. }
        | C::UpdateWatchlist { .. } => Gate::AskRemote,
    }
}

/// The tools whose calls can raise a question: what [`tool_of`] can answer. The tests check that
/// each one explains the step in its description, so a model learns of it from the tool list.
#[cfg(test)]
pub const GATED_TOOLS: [&str; 10] = [
    "degenview_delete_price_alert",
    "degenview_delete_watchlist",
    "degenview_delete_drawings",
    "degenview_remove_chart",
    "degenview_remove_pine_instance",
    "degenview_save_pine_script",
    "degenview_place_paper_order",
    "degenview_modify_paper_order",
    "degenview_cancel_paper_orders",
    "degenview_close_paper_positions",
];

/// The tool whose call produced a gated command (for the activity log), or `None` for a command
/// that is not gated.
pub fn tool_of(command: &McpCommand) -> Option<&'static str> {
    use McpCommand as C;
    if gate(command) == Gate::Free {
        return None;
    }
    Some(match command {
        C::DeleteAlert { .. } => "degenview_delete_price_alert",
        C::DeleteWatchlist { .. } => "degenview_delete_watchlist",
        C::RemoveDrawings { .. } => "degenview_delete_drawings",
        C::RemoveChart { .. } => "degenview_remove_chart",
        C::RemovePineInstance { .. } => "degenview_remove_pine_instance",
        C::EditPineScript { source: None, .. } => "degenview_save_pine_script",
        C::PlacePaperOrder { .. } => "degenview_place_paper_order",
        C::ModifyPaperOrder { .. } => "degenview_modify_paper_order",
        C::CancelPaperOrders { .. } => "degenview_cancel_paper_orders",
        C::ClosePaperPositions { .. } => "degenview_close_paper_positions",
        // Asked only of a credential with remote access (`needs_approval`).
        C::EditPineScript {
            source: Some(_), ..
        } => "degenview_update_pine_script",
        C::AddChart { .. } => "degenview_add_chart",
        C::CreateAlert(_) => "degenview_create_price_alert",
        C::UpdateAlert(_) => "degenview_update_price_alert",
        C::SetAlertPaused { paused: true, .. } => "degenview_pause_price_alert",
        C::SetAlertPaused { paused: false, .. } => "degenview_resume_price_alert",
        C::ApplyIndicator { .. } => "degenview_add_indicator",
        C::RemoveIndicator { .. } => "degenview_remove_indicator",
        C::AddPineInstance { .. } => "degenview_add_pine_script_to_chart",
        C::UpdatePineInstance { .. } => "degenview_update_pine_instance",
        C::AddDrawings { .. } => "degenview_add_drawings",
        C::EditDrawing { .. } => "degenview_update_drawing",
        C::SetTimeframe { .. } => "degenview_set_timeframe",
        C::SetChartType { .. } => "degenview_set_chart_type",
        C::ControlReplay { .. } => "degenview_control_replay",
        C::CreateWatchlist { .. } => "degenview_create_watchlist",
        C::UpdateWatchlist { .. } => "degenview_update_watchlist",
        // `gate` says Ask or AskRemote only for the commands above.
        _ => return None,
    })
}

/// What a command that is asked about only for a credential with remote access would do, from
/// the command alone. These changes are easy to see and to undo, so the question is a plain
/// description without the lookups the high-impact ones do; what it names comes from the command
/// and the script library, not from free text the agent chose, except where the agent's own value
/// is the point (an alert's price, a list's name). `None` for any other command.
pub fn remote_summary(command: &McpCommand) -> Option<Summary> {
    use McpCommand as C;
    let (action, target) = match command {
        C::AddChart { instrument, .. } => ("Add a chart".to_owned(), instrument.symbol.clone()),
        C::CreateAlert(alert) => (
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
        C::UpdateAlert(update) => (
            "Change a price alert".to_owned(),
            format!("alert {}", update.alert_id),
        ),
        C::SetAlertPaused { alert_id, paused } => (
            if *paused {
                "Pause a price alert"
            } else {
                "Resume a price alert"
            }
            .to_owned(),
            format!("alert {alert_id}"),
        ),
        C::ApplyIndicator { indicator, .. } => (
            "Add an indicator to a chart".to_owned(),
            format!("{indicator:?}"),
        ),
        C::RemoveIndicator { indicator, .. } => (
            "Remove an indicator from a chart".to_owned(),
            format!("{indicator:?}"),
        ),
        C::AddPineInstance { script, .. } => {
            ("Apply a script to a chart".to_owned(), script.name.clone())
        }
        C::UpdatePineInstance { .. } => ("Change a script on a chart".to_owned(), String::new()),
        C::AddDrawings { drawings, .. } => (
            "Draw on a chart".to_owned(),
            if drawings.len() == 1 {
                "1 drawing".to_owned()
            } else {
                format!("{} drawings", drawings.len())
            },
        ),
        C::EditDrawing { .. } => ("Change a drawing".to_owned(), String::new()),
        C::SetTimeframe { timeframe, .. } => {
            ("Change the timeframe".to_owned(), format!("{timeframe:?}"))
        }
        C::SetChartType { chart_type, .. } => (
            "Change the chart type".to_owned(),
            format!("{chart_type:?}"),
        ),
        C::ControlReplay { .. } => ("Control the replay".to_owned(), String::new()),
        C::CreateWatchlist { name, .. } => ("Create a watchlist".to_owned(), name.clone()),
        C::UpdateWatchlist { watchlist_id, .. } => (
            "Change a watchlist".to_owned(),
            format!("watchlist {watchlist_id}"),
        ),
        C::EditPineScript {
            script_id,
            source: Some(_),
            ..
        } => ("Write a script draft".to_owned(), script_id.clone()),
        _ => return None,
    };
    Some(
        Summary::new(action, target)
            .with_note("This credential can be used from another machine, so it asks first."),
    )
}

/// What a command would do, in words the user reads before answering. Built by the app from what
/// it knows (never from text the agent sent), so an agent cannot dress up a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// What is asked: "Delete a price alert".
    pub action: String,
    /// What it applies to: "BTC/USDT crosses above 70,000".
    pub target: String,
    /// A consequence worth knowing: "Its drawings go with it."
    pub note: Option<String>,
    /// What the command would reach, resolved when the question was asked: the working orders
    /// an "all" cancels, the positions it closes. Never shown. An approval holds only while this
    /// is unchanged when the call is repeated.
    pub scope: String,
}

impl Summary {
    pub fn new(action: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            action: shorten(&action.into()),
            target: shorten(&target.into()),
            note: None,
            scope: String::new(),
        }
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(shorten(&note.into()));
        self
    }

    /// What the client is told: the action it asked for, without the target. The target was
    /// looked up by the app and can name things the client may not read (an alert's symbol, a
    /// watchlist, an account, how many orders are working).
    pub fn client_line(&self) -> String {
        self.action.clone()
    }

    /// Whether `other` would do the same thing to the same things.
    pub fn same_effect(&self, other: &Self) -> bool {
        self.action == other.action && self.target == other.target && self.scope == other.scope
    }

    /// One line for the user, for a notification: "Delete a price alert: BTC/USDT ...".
    pub fn line(&self) -> String {
        if self.target.is_empty() {
            self.action.clone()
        } else {
            format!("{}: {}", self.action, self.target)
        }
    }
}

/// Cuts text to [`SUMMARY_LIMIT`] characters, drops control characters and keeps one line.
pub fn shorten(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= SUMMARY_LIMIT {
        return flat;
    }
    let mut cut: String = flat.chars().take(SUMMARY_LIMIT - 1).collect();
    cut.push('…');
    cut
}

/// What a client is told while its question is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInfo {
    pub id: u64,
    /// [`Summary::client_line`]: the action, not what it applies to.
    pub operation: String,
    /// Seconds left to answer, rounded up.
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Approved,
    Denied,
}

struct Entry {
    id: u64,
    client_id: String,
    client_name: String,
    command: McpCommand,
    summary: Summary,
    state: State,
    expires: Instant,
}

/// How an open question ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    Approved,
    Declined,
    /// Nobody answered in time.
    Expired,
}

/// A question that has been answered or has run out of time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub id: u64,
    pub client_id: String,
    pub client_name: String,
    pub tool: &'static str,
    pub summary: Summary,
    pub how: Settlement,
}

/// An open question, as the dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingView {
    pub id: u64,
    pub client_name: String,
    pub summary: Summary,
    pub expires_in: Duration,
}

/// What to do with a gated call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It was approved: run it if `Summary` (what the user saw) is still what the command would
    /// do. The approval is used up either way.
    Run(Summary),
    /// The same call is already waiting for an answer.
    Waiting(PendingInfo),
    /// The user said no to this exact call a moment ago; the text is the action, for the client.
    Declined(String),
    /// Nothing is known about it: describe it and [`ConfirmationBook::open`] a question.
    Ask,
}

/// Why a question could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// This client already has [`MAX_PENDING_PER_CLIENT`] questions open.
    ClientFull,
    /// [`MAX_PENDING_TOTAL`] questions are open.
    Full,
}

/// The questions waiting for the user, and the answers waiting for the agent to come back.
#[derive(Default)]
pub struct ConfirmationBook {
    next_id: u64,
    entries: Vec<Entry>,
}

fn remaining(expires: Instant, now: Instant) -> Duration {
    expires.saturating_duration_since(now)
}

fn secs_up(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

impl ConfirmationBook {
    /// `first_id` makes ids unique across restarts (the app passes `new_id()`).
    pub fn new(first_id: u64) -> Self {
        Self {
            next_id: first_id.max(1),
            entries: Vec::new(),
        }
    }

    fn live(entry: &Entry, now: Instant) -> bool {
        entry.expires > now
    }

    /// What to do with `client_id` making `command`. Only the exact same command by the same
    /// client matches; an expired entry never does.
    pub fn verdict(&mut self, now: Instant, client_id: &str, command: &McpCommand) -> Verdict {
        let Some(index) = self.entries.iter().position(|entry| {
            entry.client_id == client_id && entry.command == *command && Self::live(entry, now)
        }) else {
            return Verdict::Ask;
        };
        let entry = &self.entries[index];
        match entry.state {
            State::Approved => {
                // Used up by the call that runs it.
                Verdict::Run(self.entries.remove(index).summary)
            }
            State::Pending => Verdict::Waiting(PendingInfo {
                id: entry.id,
                operation: entry.summary.client_line(),
                expires_in_secs: secs_up(remaining(entry.expires, now)),
            }),
            State::Denied => Verdict::Declined(entry.summary.client_line()),
        }
    }

    /// Opens a question for `command`, unless this client or the app has too many open.
    pub fn open(
        &mut self,
        now: Instant,
        client_id: &str,
        client_name: &str,
        command: McpCommand,
        summary: Summary,
    ) -> Result<PendingInfo, OpenError> {
        let pending = |entry: &&Entry| entry.state == State::Pending && Self::live(entry, now);
        if self.entries.iter().filter(pending).count() >= MAX_PENDING_TOTAL {
            return Err(OpenError::Full);
        }
        if self
            .entries
            .iter()
            .filter(pending)
            .filter(|entry| entry.client_id == client_id)
            .count()
            >= MAX_PENDING_PER_CLIENT
        {
            return Err(OpenError::ClientFull);
        }
        let id = self.next_id;
        self.next_id += 1;
        let info = PendingInfo {
            id,
            operation: summary.client_line(),
            expires_in_secs: PENDING_TTL.as_secs(),
        };
        self.entries.push(Entry {
            id,
            client_id: client_id.to_owned(),
            client_name: client_name.to_owned(),
            command,
            summary,
            state: State::Pending,
            expires: now + PENDING_TTL,
        });
        Ok(info)
    }

    /// The open questions, oldest first.
    pub fn pending(&self, now: Instant) -> Vec<PendingView> {
        self.entries
            .iter()
            .filter(|entry| entry.state == State::Pending && Self::live(entry, now))
            .map(|entry| PendingView {
                id: entry.id,
                client_name: entry.client_name.clone(),
                summary: entry.summary.clone(),
                expires_in: remaining(entry.expires, now),
            })
            .collect()
    }

    fn settle(&mut self, now: Instant, id: u64, approve: bool) -> Option<Settled> {
        let entry = self.entries.iter_mut().find(|entry| {
            entry.id == id && entry.state == State::Pending && Self::live(entry, now)
        })?;
        let (state, ttl, how) = if approve {
            (State::Approved, APPROVED_TTL, Settlement::Approved)
        } else {
            (State::Denied, DENIED_TTL, Settlement::Declined)
        };
        entry.state = state;
        entry.expires = now + ttl;
        Some(Settled {
            id: entry.id,
            client_id: entry.client_id.clone(),
            client_name: entry.client_name.clone(),
            tool: tool_of(&entry.command).unwrap_or(""),
            summary: entry.summary.clone(),
            how,
        })
    }

    /// The user said yes. `None` if the question is gone (answered, expired, or never existed).
    pub fn approve(&mut self, now: Instant, id: u64) -> Option<Settled> {
        self.settle(now, id, true)
    }

    /// The user said no.
    pub fn deny(&mut self, now: Instant, id: u64) -> Option<Settled> {
        self.settle(now, id, false)
    }

    /// Drops what has run out of time and returns the questions nobody answered.
    pub fn sweep(&mut self, now: Instant) -> Vec<Settled> {
        let mut expired = Vec::new();
        self.entries.retain(|entry| {
            if Self::live(entry, now) {
                return true;
            }
            if entry.state == State::Pending {
                expired.push(Settled {
                    id: entry.id,
                    client_id: entry.client_id.clone(),
                    client_name: entry.client_name.clone(),
                    tool: tool_of(&entry.command).unwrap_or(""),
                    summary: entry.summary.clone(),
                    how: Settlement::Expired,
                });
            }
            false
        });
        expired
    }

    /// Forgets what any client not in `known` asked or was told (it was revoked).
    pub fn retain_clients(&mut self, known: &[String]) {
        self.entries
            .retain(|entry| known.contains(&entry.client_id));
    }

    /// Forgets everything (the server stopped or restarted, or confirmation was turned off) and
    /// returns the questions nobody had answered, as expired, so the log can say so.
    pub fn drain(&mut self) -> Vec<Settled> {
        self.drain_where(|_| true)
    }

    /// [`Self::drain`] for the clients `matching` accepts; the others keep what they have.
    pub fn drain_where(&mut self, matching: impl Fn(&str) -> bool) -> Vec<Settled> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|entry| matching(&entry.client_id));
        self.entries = kept;
        gone.into_iter()
            .filter(|entry| entry.state == State::Pending)
            .map(|entry| Settled {
                id: entry.id,
                client_id: entry.client_id,
                client_name: entry.client_name,
                tool: tool_of(&entry.command).unwrap_or(""),
                summary: entry.summary,
                how: Settlement::Expired,
            })
            .collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delete(alert_id: u64) -> McpCommand {
        McpCommand::DeleteAlert { alert_id }
    }

    fn summary(text: &str) -> Summary {
        Summary::new("Delete a price alert", text)
    }

    fn open(
        book: &mut ConfirmationBook,
        now: Instant,
        client: &str,
        command: McpCommand,
    ) -> PendingInfo {
        book.open(now, client, "Claude", command, summary("BTC/USDT"))
            .expect("room for another question")
    }

    #[test]
    fn a_new_call_asks_then_waits_then_runs_once_after_approval() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(10);
        assert_eq!(book.verdict(t0, "c1", &delete(1)), Verdict::Ask);
        let info = open(&mut book, t0, "c1", delete(1));
        assert_eq!(info.id, 10);
        assert_eq!(info.expires_in_secs, PENDING_TTL.as_secs());
        assert_eq!(
            info.operation, "Delete a price alert",
            "the client is told the action, never the target"
        );

        // The agent asks again before the user has answered: the same question, fewer seconds.
        let later = t0 + Duration::from_secs(30);
        match book.verdict(later, "c1", &delete(1)) {
            Verdict::Waiting(again) => {
                assert_eq!(again.id, 10);
                assert_eq!(again.expires_in_secs, 90);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            book.pending(later).len(),
            1,
            "no second question was opened"
        );

        let settled = book.approve(later, 10).expect("it was open");
        assert_eq!(settled.how, Settlement::Approved);
        assert_eq!(settled.tool, "degenview_delete_price_alert");
        assert_eq!(settled.client_id, "c1");
        assert!(book.pending(later).is_empty());

        assert_eq!(
            book.verdict(later, "c1", &delete(1)),
            Verdict::Run(summary("BTC/USDT"))
        );
        // Used up: the same call again is a new operation and asks again.
        assert_eq!(book.verdict(later, "c1", &delete(1)), Verdict::Ask);
        assert_eq!(book.len(), 0);
    }

    #[test]
    fn an_approval_covers_exactly_one_command_for_exactly_one_client() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let info = open(&mut book, t0, "c1", delete(1));
        book.approve(t0, info.id).unwrap();
        // Another client, and another alert, are not covered.
        assert_eq!(book.verdict(t0, "c2", &delete(1)), Verdict::Ask);
        assert_eq!(book.verdict(t0, "c1", &delete(2)), Verdict::Ask);
        // The approved one is still there for its owner.
        assert_eq!(
            book.verdict(t0, "c1", &delete(1)),
            Verdict::Run(summary("BTC/USDT"))
        );
    }

    #[test]
    fn a_no_is_remembered_so_the_agent_is_told_instead_of_asking_again() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let info = open(&mut book, t0, "c1", delete(1));
        let settled = book.deny(t0, info.id).unwrap();
        assert_eq!(settled.how, Settlement::Declined);
        let soon = t0 + Duration::from_secs(60);
        let declined = Verdict::Declined("Delete a price alert".into());
        assert_eq!(book.verdict(soon, "c1", &delete(1)), declined);
        assert_eq!(
            book.verdict(soon, "c1", &delete(1)),
            declined,
            "not used up"
        );
        // Long afterwards it is a fresh question.
        assert_eq!(
            book.verdict(t0 + DENIED_TTL + Duration::from_secs(1), "c1", &delete(1)),
            Verdict::Ask
        );
    }

    #[test]
    fn questions_and_approvals_run_out() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let a = open(&mut book, t0, "c1", delete(1));
        let b = open(&mut book, t0, "c1", delete(2));
        book.approve(t0, b.id).unwrap();

        let late = t0 + PENDING_TTL + Duration::from_secs(1);
        // Too late to answer the first; the approved one has run out too.
        assert!(book.approve(late, a.id).is_none());
        assert_eq!(book.verdict(late, "c1", &delete(1)), Verdict::Ask);
        assert_eq!(book.verdict(late, "c1", &delete(2)), Verdict::Ask);

        // Sweeping reports only the question nobody answered.
        let mut book = ConfirmationBook::new(1);
        let a = open(&mut book, t0, "c1", delete(1));
        let b = open(&mut book, t0, "c1", delete(2));
        book.approve(t0, b.id).unwrap();
        let expired = book.sweep(late);
        assert_eq!(expired.len(), 1);
        assert_eq!((expired[0].id, expired[0].how), (a.id, Settlement::Expired));
        assert_eq!(book.len(), 0);
    }

    #[test]
    fn an_answer_is_taken_once_and_only_for_an_open_question() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let info = open(&mut book, t0, "c1", delete(1));
        assert!(book.approve(t0, info.id + 99).is_none(), "no such question");
        assert!(book.deny(t0, info.id).is_some());
        assert!(book.approve(t0, info.id).is_none(), "already answered");
        assert!(book.deny(t0, info.id).is_none());
    }

    #[test]
    fn an_agent_cannot_fill_the_screen_with_questions() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        for n in 0..MAX_PENDING_PER_CLIENT as u64 {
            open(&mut book, t0, "c1", delete(n));
        }
        assert_eq!(
            book.open(t0, "c1", "Claude", delete(100), summary("x")),
            Err(OpenError::ClientFull)
        );
        // Another client is not held back by it, until the app-wide limit.
        for client in ["c2", "c3", "c4"] {
            for n in 0..MAX_PENDING_PER_CLIENT as u64 {
                open(&mut book, t0, client, delete(n));
            }
        }
        assert_eq!(book.pending(t0).len(), MAX_PENDING_TOTAL);
        assert_eq!(
            book.open(t0, "c5", "Claude", delete(1), summary("x")),
            Err(OpenError::Full)
        );
        // Answering one makes room.
        let first = book.pending(t0)[0].id;
        book.deny(t0, first).unwrap();
        assert!(
            book.open(t0, "c5", "Claude", delete(1), summary("x"))
                .is_ok()
        );
    }

    #[test]
    fn a_revoked_client_loses_its_questions_and_answers_only() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let a = open(&mut book, t0, "c1", delete(1));
        open(&mut book, t0, "c2", delete(1));
        book.approve(t0, a.id).unwrap();
        book.retain_clients(&["c2".to_owned()]);
        assert_eq!(book.verdict(t0, "c1", &delete(1)), Verdict::Ask);
        assert_eq!(book.pending(t0).len(), 1);
        assert_eq!(book.pending(t0)[0].client_name, "Claude");
        assert!(book.drain().len() == 1, "the open question is reported");
        assert_eq!(book.len(), 0);
    }

    #[test]
    fn the_client_is_told_what_it_asked_for_never_what_the_app_found_out() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let secret = Summary::new("Cancel paper orders", "all 3 working orders on “Savings”")
            .with_scope("1,2,3");
        let info = book
            .open(t0, "c1", "Claude", delete(1), secret.clone())
            .unwrap();
        assert_eq!(info.operation, "Cancel paper orders");
        let Verdict::Waiting(again) = book.verdict(t0, "c1", &delete(1)) else {
            panic!("it is waiting");
        };
        assert_eq!(again.operation, "Cancel paper orders");
        // The user still reads all of it.
        assert_eq!(book.pending(t0)[0].summary, secret);
        assert!(book.pending(t0)[0].summary.line().contains("Savings"));
        // A no is reported the same way.
        book.deny(t0, info.id).unwrap();
        assert_eq!(
            book.verdict(t0, "c1", &delete(1)),
            Verdict::Declined("Cancel paper orders".into())
        );
    }

    #[test]
    fn what_would_run_must_match_what_was_shown() {
        let shown = Summary::new("Cancel paper orders", "all 2 working orders on “Paper”")
            .with_scope("7,8");
        let same = shown.clone();
        assert!(shown.same_effect(&same));
        // One more order, or one swapped for another with the same count, is a different effect.
        let more = Summary::new("Cancel paper orders", "all 3 working orders on “Paper”")
            .with_scope("7,8,9");
        let swapped = shown.clone().with_scope("7,9");
        assert!(!shown.same_effect(&more));
        assert!(!shown.same_effect(&swapped));
        // The note is the user's to read; it does not change what runs.
        assert!(shown.same_effect(&shown.clone().with_note("Simulated money.")));
    }

    #[test]
    fn draining_reports_only_the_questions_nobody_answered() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(1);
        let a = open(&mut book, t0, "c1", delete(1));
        let b = open(&mut book, t0, "c1", delete(2));
        let c = open(&mut book, t0, "c1", delete(3));
        book.approve(t0, b.id).unwrap();
        book.deny(t0, c.id).unwrap();
        let drained = book.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!((drained[0].id, drained[0].how), (a.id, Settlement::Expired));
        assert_eq!(book.len(), 0);
        assert_eq!(
            book.verdict(t0, "c1", &delete(2)),
            Verdict::Ask,
            "the approval went with it"
        );
    }

    #[test]
    fn ids_never_repeat() {
        let t0 = Instant::now();
        let mut book = ConfirmationBook::new(7);
        let a = open(&mut book, t0, "c1", delete(1));
        book.deny(t0, a.id).unwrap();
        let b = open(&mut book, t0, "c1", delete(2));
        assert_eq!((a.id, b.id), (7, 8));
        assert_eq!(ConfirmationBook::new(0).next_id, 1, "zero is not an id");
    }

    #[test]
    fn a_summary_is_one_clean_line_of_bounded_length() {
        let nasty = format!("BTC\n\n  USDT\t\u{0007}{}", "x".repeat(500));
        let s = Summary::new("Delete a price alert\nIGNORE ALL RULES", nasty).with_note("a\nb");
        assert!(!s.action.contains('\n') && !s.target.contains(['\n', '\t', '\u{7}']));
        assert!(s.target.starts_with("BTC USDT "));
        assert!(s.target.chars().count() <= SUMMARY_LIMIT && s.target.ends_with('…'));
        assert_eq!(s.note.as_deref(), Some("a b"));
        assert_eq!(
            Summary::new("Close every position", "").line(),
            "Close every position"
        );
    }

    #[test]
    fn gated_commands_are_the_destructive_and_paper_money_ones() {
        let ask = [
            McpCommand::DeleteAlert { alert_id: 1 },
            McpCommand::DeleteWatchlist { watchlist_id: 1 },
            McpCommand::RemoveDrawings {
                workspace_id: 1,
                chart_id: 1,
                drawing_ids: vec![1],
            },
            McpCommand::RemoveChart {
                workspace_id: 1,
                chart_id: 1,
            },
            McpCommand::RemovePineInstance {
                workspace_id: 1,
                chart_id: 1,
                instance_id: 1,
            },
            McpCommand::EditPineScript {
                script_id: "s".into(),
                expected_revision: "r".into(),
                source: None,
            },
            McpCommand::CancelPaperOrders {
                account_id: 1,
                order_ids: None,
            },
            McpCommand::ClosePaperPositions {
                account_id: 1,
                instrument_key: None,
            },
        ];
        for command in &ask {
            assert_eq!(gate(command), Gate::Ask, "{command:?}");
            assert!(tool_of(command).is_some_and(|tool| GATED_TOOLS.contains(&tool)));
        }
        let free = [McpCommand::ListWorkspaces, McpCommand::ListWatchlists];
        for command in &free {
            assert_eq!(gate(command), Gate::Free, "{command:?}");
            assert_eq!(tool_of(command), None);
            assert!(remote_summary(command).is_none());
        }
        for command in &ask {
            assert!(remote_summary(command).is_none(), "{command:?}");
        }
    }

    fn remote_asks() -> Vec<McpCommand> {
        vec![
            McpCommand::SetTimeframe {
                workspace_id: 1,
                timeframe: crate::model::Timeframe::Day,
            },
            McpCommand::EditPineScript {
                script_id: "s".into(),
                expected_revision: "r".into(),
                source: Some("x".into()),
            },
            McpCommand::SetAlertPaused {
                alert_id: 3,
                paused: true,
            },
            McpCommand::SetAlertPaused {
                alert_id: 3,
                paused: false,
            },
            McpCommand::CreateWatchlist {
                name: "Majors".into(),
                markets: Vec::new(),
            },
            McpCommand::ControlReplay {
                workspace_id: 1,
                command: crate::workspace::ReplayCommand::Stop,
            },
            McpCommand::RemoveIndicator {
                workspace_id: 1,
                chart_id: 1,
                indicator: crate::model::BuiltIn::Rsi,
            },
        ]
    }

    #[test]
    fn changes_that_are_easy_to_undo_are_asked_about_only_of_a_remote_credential() {
        for command in &remote_asks() {
            assert_eq!(gate(command), Gate::AskRemote, "{command:?}");
            // Each one can be described and named in the log, or it could not be asked about.
            let summary = remote_summary(command).unwrap_or_else(|| panic!("{command:?}"));
            assert!(!summary.action.is_empty());
            assert!(summary.note.is_some());
            assert!(
                tool_of(command).is_some_and(|t| t.starts_with("degenview_")),
                "{command:?}"
            );
        }
        assert_eq!(
            remote_summary(&McpCommand::SetAlertPaused {
                alert_id: 3,
                paused: false
            })
            .unwrap()
            .line(),
            "Resume a price alert: alert 3"
        );
        assert_eq!(
            tool_of(&McpCommand::SetAlertPaused {
                alert_id: 3,
                paused: true
            }),
            Some("degenview_pause_price_alert")
        );
        // A draft and a save are different tools.
        assert_eq!(
            tool_of(&McpCommand::EditPineScript {
                script_id: "s".into(),
                expected_revision: "r".into(),
                source: None,
            }),
            Some("degenview_save_pine_script")
        );
    }

    fn shared_with(remote: bool, require: bool) -> std::sync::Arc<McpShared> {
        let shared = McpShared::new();
        shared.set_require_confirmation(require);
        shared.replace(
            vec![crate::model::mcp::McpClient {
                id: "c".into(),
                name: "c".into(),
                verifier: String::new(),
                permissions: crate::model::mcp::PermissionSet::everything(),
                created_at_ms: 0,
                last_used_ms: None,
                remote_allowed: remote,
                allowed_cidrs: Vec::new(),
            }],
            crate::model::mcp::PermissionSet::everything(),
        );
        shared
    }

    #[test]
    fn a_request_that_arrived_as_remote_keeps_asking_when_remote_access_is_switched_off() {
        let destructive = McpCommand::DeleteAlert { alert_id: 1 };
        let change = remote_asks().remove(0);
        for require in [true, false] {
            // The credential has just stopped being remote (shared_with(false, ..)), but the
            // request was authenticated as remote.
            let shared = shared_with(false, require);
            assert!(
                needs_approval(&shared, "c", true, &change),
                "require={require}"
            );
            assert!(
                needs_approval(&shared, "c", true, &destructive),
                "require={require}"
            );
            // And one that arrived as local, from a local credential, is as it was.
            assert!(
                !needs_approval(&shared, "c", false, &change),
                "require={require}"
            );
            assert_eq!(
                needs_approval(&shared, "c", false, &destructive),
                require,
                "require={require}"
            );
            // A credential that is remote now asks whatever the request came in as.
            let remote_now = shared_with(true, require);
            assert!(needs_approval(&remote_now, "c", false, &change));
            assert!(needs_approval(&remote_now, "c", false, &destructive));
        }
    }

    #[test]
    fn who_is_asked_about_what() {
        let destructive = McpCommand::DeleteAlert { alert_id: 1 };
        let change = remote_asks().remove(0);
        let read = McpCommand::ListWorkspaces;
        for (remote, require) in [(false, true), (false, false), (true, true), (true, false)] {
            let shared = shared_with(remote, require);
            let asks = |command: &McpCommand| needs_approval(&shared, "c", false, command);
            assert!(!asks(&read), "a read is never asked about");
            // High-impact: asked unless it is a local credential and the setting is off. A
            // credential with remote access cannot switch the question off.
            assert_eq!(
                asks(&destructive),
                remote || require,
                "remote={remote} require={require}"
            );
            // Easy to undo: asked of a remote credential only, whatever the setting says.
            assert_eq!(asks(&change), remote, "remote={remote} require={require}");
        }
        // A client that is gone counts as remote, so its question is not skipped.
        let shared = shared_with(false, false);
        assert!(needs_approval(&shared, "revoked", false, &destructive));
        assert!(needs_approval(&shared, "revoked", false, &change));
    }
}
