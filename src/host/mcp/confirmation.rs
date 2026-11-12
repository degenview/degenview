//! The app's side of the MCP confirmation step: the questions waiting for the user, the
//! summaries that say what a command would do, and the decisions the user makes.
//!
//! Everything here runs on the app thread or the UI thread. The pure rules (which commands ask,
//! how long a question lives) are in [`crate::mcp::confirm`]; this module adds what needs the
//! app's data: it looks a target up to describe it, records the user's answer in the activity
//! log, and keeps the book in step with the server (revoked clients, confirmation turned off).
//!
//! A summary is built from what the app knows about the target, never from text the agent sent,
//! so an agent cannot word its own question. A target that does not exist (or could never be
//! changed) is refused with the error the command itself would give, without asking anyone.

use crate::host::alert::AlertHost;
use crate::host::mcp::{self, find_alert, require_loaded};
use crate::host::paper::PaperHost;
use crate::mcp::bridge::{McpCommand, McpError};
use crate::mcp::chart::data;
use crate::mcp::confirm::{
    ConfirmationBook, OpenError, PendingInfo, PendingView, Settled, Settlement, Summary, Verdict,
};
use crate::mcp::state::McpShared;
use crate::model::alert::AlertId;
use crate::model::alert::pine::PineAlertSubscription;
use crate::model::mcp::{McpAuditEntry, McpAuditKind, McpAuditOutcome};
use crate::model::paper::{PaperOrderChanges, PaperOrderRequest, PaperOrderSide, PaperOrderType};
use crate::model::watchlist::{new_id, now_ms};
use crate::store::watchlist::WatchlistStore;
use crate::workspace::ChartWorkspace;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The questions waiting for an answer, shared between the app thread (which asks) and the UI
/// (which shows them and takes the answers).
pub struct ConfirmationHost {
    book: ConfirmationBook,
    shared: Arc<McpShared>,
    /// Questions opened since the last [`Self::take_fresh`], for a notification.
    fresh: Vec<PendingView>,
    /// The server generation the book was filled under. Stopping the server (to turn it off or to
    /// change its port) moves it on, and nothing asked or approved under an older one survives.
    generation: u64,
}

impl ConfirmationHost {
    pub fn new(shared: Arc<McpShared>) -> Self {
        Self {
            book: ConfirmationBook::new(new_id()),
            generation: shared.generation(),
            shared,
            fresh: Vec::new(),
        }
    }

    /// The open questions, oldest first, as the dialog shows them.
    pub fn pending(&self, now: Instant) -> Vec<PendingView> {
        self.book.pending(now)
    }

    /// Questions opened since the last call (each is returned once).
    pub fn take_fresh(&mut self) -> Vec<PendingView> {
        std::mem::take(&mut self.fresh)
    }

    /// The user said yes.
    pub fn approve(&mut self, now: Instant, id: u64) {
        if let Some(settled) = self.book.approve(now, id) {
            self.record(&settled);
        }
    }

    /// The user said no (or closed the question).
    pub fn deny(&mut self, now: Instant, id: u64) {
        if let Some(settled) = self.book.deny(now, id) {
            self.record(&settled);
        }
    }

    /// Housekeeping, once a frame: questions nobody answered are dropped (and logged), what was
    /// asked by a client that no longer exists is forgotten, and with confirmation switched off
    /// or the server stopped and started again nothing is left waiting or approved.
    pub fn tick(&mut self, now: Instant) {
        for settled in self.book.sweep(now) {
            self.record(&settled);
        }
        // Turning the server off is a stop signal: a yes given before it must not fire after it
        // comes back, even within the time it would have lasted.
        let generation = self.shared.generation();
        if generation != self.generation {
            self.generation = generation;
            for settled in self.book.drain() {
                self.record(&settled);
            }
            self.fresh.clear();
        } else if !self.shared.require_confirmation() {
            // Switching the setting off ends the questions of local credentials. A credential
            // with remote access is not covered by it (`confirm::needs_approval`), so what it
            // asked stays open.
            let remote: Vec<String> = self
                .shared
                .clients()
                .into_iter()
                .filter(|client| client.remote_allowed)
                .map(|client| client.id)
                .collect();
            for settled in self.book.drain_where(|id| !remote.iter().any(|r| r == id)) {
                self.record(&settled);
            }
            let alive: Vec<u64> = self.book.pending(now).iter().map(|view| view.id).collect();
            self.fresh.retain(|view| alive.contains(&view.id));
        }
        // A question and its answer are tied to the client that asked.
        let known: Vec<String> = self.shared.clients().into_iter().map(|c| c.id).collect();
        self.book.retain_clients(&known);
    }

    /// Writes the answer into the activity log.
    fn record(&self, settled: &Settled) {
        let outcome = match settled.how {
            Settlement::Approved => McpAuditOutcome::Approved,
            Settlement::Declined => McpAuditOutcome::Declined,
            Settlement::Expired => McpAuditOutcome::Expired,
        };
        self.shared.audit().push(McpAuditEntry {
            id: new_id(),
            at_ms: now_ms(),
            kind: McpAuditKind::Confirmation,
            client_id: settled.client_id.clone(),
            client_name: settled.client_name.clone(),
            tool: settled.tool.to_owned(),
            outcome,
            count: 1,
            ..McpAuditEntry::default()
        });
    }
}

/// Decides whether `client_id` may run `command` now: `Ok` to run it, or the error to answer with
/// (`CONFIRMATION_REQUIRED` with the new question, `CONFIRMATION_DENIED`, or the target's own
/// error). `describe` builds the summary and is only called when a new question is needed. The
/// host's lock is not held while it runs, since it takes other locks.
pub fn gate_command(
    host: &Mutex<ConfirmationHost>,
    now: Instant,
    client_id: &str,
    client_name: &str,
    remote_origin: bool,
    command: &McpCommand,
    describe: impl FnOnce() -> Result<Summary, McpError>,
) -> Result<(), McpError> {
    let unavailable = || McpError::AppUnavailable("Confirmations are unavailable".into());
    let required = {
        let host = host.lock().map_err(|_| unavailable())?;
        crate::mcp::confirm::needs_approval(&host.shared, client_id, remote_origin, command)
    };
    if !required {
        return Ok(());
    }
    let verdict = host
        .lock()
        .map_err(|_| unavailable())?
        .book
        .verdict(now, client_id, command);
    let ask = |summary: Summary| {
        let mut host = host.lock().map_err(|_| unavailable())?;
        match host
            .book
            .open(now, client_id, client_name, command.clone(), summary)
        {
            Ok(info) => {
                host.remember_fresh(now, &info);
                Err(McpError::ConfirmationRequired(info))
            }
            Err(OpenError::ClientFull) => Err(McpError::ResourceLimit(
                "Too many approvals from this client are waiting. Ask the user to answer them in DegenView first".into(),
            )),
            Err(OpenError::Full) => Err(McpError::ResourceLimit(
                "Too many approvals are waiting in DegenView. Ask the user to answer them first".into(),
            )),
        }
    };
    match verdict {
        Verdict::Run(approved) => {
            // What the user said yes to must still be what would happen: an "all" is whatever is
            // working now, not what was working when it was asked. The approval is spent either
            // way; if the target moved, the user is asked about what it is now.
            let current = describe()?;
            if current.same_effect(&approved) {
                Ok(())
            } else {
                ask(current)
            }
        }
        Verdict::Waiting(info) => Err(McpError::ConfirmationRequired(info)),
        Verdict::Declined(operation) => Err(McpError::ConfirmationDenied(operation)),
        Verdict::Ask => ask(describe()?),
    }
}

impl ConfirmationHost {
    fn remember_fresh(&mut self, now: Instant, info: &PendingInfo) {
        if let Some(view) = self
            .book
            .pending(now)
            .into_iter()
            .find(|view| view.id == info.id)
        {
            self.fresh.push(view);
        }
    }
}

// MARK: Summaries

/// "0.5" and "70,000" without trailing zeros.
fn number(value: f64) -> String {
    let text = format!("{value:.8}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

fn side_word(side: PaperOrderSide) -> &'static str {
    match side {
        PaperOrderSide::Buy => "Buy",
        PaperOrderSide::Sell => "Sell",
    }
}

fn kind_text(kind: PaperOrderType, limit: Option<f64>, stop: Option<f64>) -> String {
    match kind {
        PaperOrderType::Market => "at market".to_owned(),
        PaperOrderType::Limit => {
            limit.map_or("limit".to_owned(), |p| format!("limit {}", number(p)))
        }
        PaperOrderType::Stop => stop.map_or("stop".to_owned(), |p| format!("stop {}", number(p))),
        PaperOrderType::StopLimit => format!(
            "stop-limit {} / {}",
            stop.map_or("?".to_owned(), number),
            limit.map_or("?".to_owned(), number)
        ),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn delete_alert(host: &AlertHost, id: AlertId) -> Result<Summary, McpError> {
    require_loaded(host)?;
    let info = mcp::alert_info(find_alert(host, id)?, None);
    let condition = match info.condition.as_str() {
        "crosses_above" => "crosses above",
        "crosses_below" => "crosses below",
        "rises_by" => "rises by",
        "falls_by" => "falls by",
        _ => "alert",
    };
    let level = info
        .target_price
        .map(|price| format!("{price} {}", info.currency))
        .or_else(|| info.percent.map(|percent| format!("{percent}%")))
        .unwrap_or_default();
    Ok(Summary::new(
        "Delete a price alert",
        format!("{} {condition} {level}", info.symbol),
    )
    .with_note("Its trigger history is kept."))
}

pub fn delete_watchlist(store: &WatchlistStore, id: u64) -> Result<Summary, McpError> {
    let list = mcp::watchlist_by_id(store, id)?;
    if list.is_favorites {
        return Err(McpError::Conflict(
            "Favorites cannot be deleted; remove its markets or rename it instead".into(),
        ));
    }
    Ok(Summary::new(
        "Delete a watchlist",
        format!(
            "“{}” with {}",
            list.name,
            plural(list.entries.len(), "market", "markets")
        ),
    ))
}

fn chart_label(workspace: &ChartWorkspace, chart_id: u64) -> Result<String, McpError> {
    workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == chart_id)
        .map(|chart| chart.display_symbol)
        .ok_or_else(|| mcp::no_chart(workspace, chart_id))
}

pub fn remove_chart(workspace: &ChartWorkspace, chart_id: u64) -> Result<Summary, McpError> {
    let label = chart_label(workspace, chart_id)?;
    Ok(Summary::new(
        "Remove a chart",
        format!("{label} in “{}”", workspace.title()),
    )
    .with_note("Its drawings go with it."))
}

pub fn remove_drawings(
    workspace: &ChartWorkspace,
    chart_id: u64,
    ids: &[u64],
) -> Result<Summary, McpError> {
    let label = chart_label(workspace, chart_id)?;
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| mcp::no_chart(workspace, chart_id))?;
    // The same check the removal makes: unknown or locked drawings are refused here, unasked.
    let commands = data::plan_remove(data.drawings, ids)?;
    Ok(Summary::new(
        format!("Delete {}", plural(commands.len(), "drawing", "drawings")),
        format!("on {label}"),
    )
    .with_note("The user can undo it in the app."))
}

pub fn remove_pine_instance(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    subscriptions: &[PineAlertSubscription],
) -> Result<Summary, McpError> {
    let summary = workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == chart_id)
        .ok_or_else(|| mcp::no_chart(workspace, chart_id))?;
    let name = summary
        .pine_instances
        .iter()
        .find(|(id, _)| *id == instance_id)
        .map(|(_, name)| name.clone())
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No script instance {instance_id} on chart {chart_id}"
            ))
        })?;
    mcp::guard_armed_alerts(subscriptions, chart_id, instance_id, "removing it")?;
    Ok(Summary::new(
        "Remove a script from a chart",
        format!("{name} on {}", summary.display_symbol),
    ))
}

pub fn save_script(name: &str) -> Summary {
    Summary::new("Save a script over its saved version", format!("“{name}”"))
        .with_note("The saved source is overwritten and cannot be undone from here.")
}

pub fn place_order(host: &PaperHost, request: &PaperOrderRequest) -> Result<Summary, McpError> {
    let account = mcp::tradable_paper_account(host, request.account_id)?;
    Ok(Summary::new(
        "Place a paper order",
        format!(
            "{} {} {} {} on “{}”",
            side_word(request.side),
            number(request.quantity),
            request.instrument.display_name,
            kind_text(request.kind, request.limit_price, request.stop_price),
            account
        ),
    )
    .with_note("Simulated money."))
}

pub fn modify_order(
    host: &PaperHost,
    order_id: u64,
    changes: &PaperOrderChanges,
) -> Result<Summary, McpError> {
    let order = host
        .snapshot()
        .orders
        .iter()
        .find(|order| order.id == order_id)
        .ok_or_else(|| McpError::NotFound("No such paper order".into()))?;
    let account = mcp::tradable_paper_account(host, order.account_id)?;
    let mut changed = Vec::new();
    if let Some(quantity) = changes.quantity {
        changed.push(format!("quantity {}", number(quantity)));
    }
    if let Some(limit) = changes.limit_price {
        changed.push(format!("limit {}", number(limit)));
    }
    if let Some(stop) = changes.stop_price {
        changed.push(format!("stop {}", number(stop)));
    }
    Ok(Summary::new(
        "Change a paper order",
        format!(
            "{} {} {} on “{}”: {}",
            side_word(order.side),
            number(order.original_quantity),
            order.instrument.display_name,
            account,
            changed.join(", ")
        ),
    )
    .with_note("Simulated money."))
}

pub fn cancel_orders(
    host: &PaperHost,
    account_id: u64,
    order_ids: Option<&[u64]>,
) -> Result<Summary, McpError> {
    let account = mcp::tradable_paper_account(host, account_id)?;
    // The orders an "all" reaches, by id: the approval holds only for these.
    let mut scope = String::new();
    let target = match order_ids {
        Some(ids) => plural(ids.len(), "named order", "named orders"),
        None => {
            let working: Vec<u64> = host
                .snapshot()
                .orders
                .iter()
                .filter(|order| order.account_id == account_id && order.status.is_working())
                .map(|order| order.id)
                .collect();
            scope = scope_of(working.iter().map(u64::to_string));
            format!(
                "all {}",
                plural(working.len(), "working order", "working orders")
            )
        }
    };
    Ok(
        Summary::new("Cancel paper orders", format!("{target} on “{account}”"))
            .with_scope(scope)
            .with_note("Simulated money."),
    )
}

/// The things a command reaches as one comparable string: sorted, so only the set matters.
fn scope_of(items: impl Iterator<Item = String>) -> String {
    let mut items: Vec<String> = items.collect();
    items.sort();
    items.join(",")
}

pub fn close_positions(
    host: &PaperHost,
    account_id: u64,
    instrument_key: Option<&str>,
) -> Result<Summary, McpError> {
    let account = mcp::tradable_paper_account(host, account_id)?;
    let open: Vec<&crate::model::paper::PaperPosition> = host
        .snapshot()
        .positions
        .iter()
        .filter(|position| position.account_id == account_id && position.signed_quantity != 0.0)
        .filter(|position| instrument_key.is_none_or(|key| key == position.instrument.key))
        .collect();
    let target = match instrument_key {
        Some(_) => match open.first() {
            Some(position) => position.instrument.display_name.clone(),
            None => {
                return Err(McpError::NotFound(
                    "That account has no open position in that market".into(),
                ));
            }
        },
        None => format!(
            "all {}",
            plural(open.len(), "open position", "open positions")
        ),
    };
    // The positions an "all" reaches: the approval holds only for these.
    let scope = scope_of(open.iter().map(|position| position.instrument.key.clone()));
    Ok(
        Summary::new("Close paper positions", format!("{target} on “{account}”"))
            .with_scope(scope)
            .with_note("Simulated money."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mcp::McpClient;

    fn alert_command() -> McpCommand {
        McpCommand::DeleteAlert { alert_id: 1 }
    }

    fn host_with(
        clients: &[&str],
        require: bool,
    ) -> (Arc<Mutex<ConfirmationHost>>, Arc<McpShared>) {
        let shared = McpShared::new();
        shared.set_require_confirmation(require);
        shared.replace(
            clients
                .iter()
                .map(|id| McpClient {
                    id: (*id).to_owned(),
                    name: format!("client {id}"),
                    verifier: String::new(),
                    permissions: crate::model::mcp::PermissionSet::everything(),
                    created_at_ms: 0,
                    last_used_ms: None,
                    remote_allowed: false,
                    allowed_cidrs: Vec::new(),
                })
                .collect(),
            crate::model::mcp::PermissionSet::everything(),
        );
        (
            Arc::new(Mutex::new(ConfirmationHost::new(shared.clone()))),
            shared,
        )
    }

    fn say() -> Result<Summary, McpError> {
        Ok(Summary::new(
            "Delete a price alert",
            "BTC/USDT crosses above 70,000",
        ))
    }

    fn cancel_all() -> McpCommand {
        McpCommand::CancelPaperOrders {
            account_id: 1,
            order_ids: None,
        }
    }

    fn orders(ids: &[u64]) -> Result<Summary, McpError> {
        Ok(Summary::new(
            "Cancel paper orders",
            format!("all {} working orders on “Paper”", ids.len()),
        )
        .with_scope(scope_of(ids.iter().map(u64::to_string))))
    }

    fn approve_the_one_open(host: &Arc<Mutex<ConfirmationHost>>, now: Instant) {
        let id = host.lock().unwrap().pending(now)[0].id;
        host.lock().unwrap().approve(now, id);
    }

    #[test]
    fn an_all_approval_holds_only_for_what_the_user_was_shown() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        let first = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            orders(&[7, 8])
        });
        assert!(matches!(first, Err(McpError::ConfirmationRequired(_))));
        approve_the_one_open(&host, now);
        // Another order appeared before the agent repeated the call: not what was approved.
        let repeat = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            orders(&[7, 8, 9])
        });
        let Err(McpError::ConfirmationRequired(info)) = repeat else {
            panic!("it must ask again: {repeat:?}");
        };
        assert_eq!(info.operation, "Cancel paper orders");
        // The user is asked about what it is now, and the old yes is spent.
        let view = &host.lock().unwrap().pending(now)[0];
        assert!(view.summary.line().contains("all 3 working orders"));
        approve_the_one_open(&host, now);
        assert!(
            gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || orders(
                &[7, 8, 9]
            ))
            .is_ok()
        );
    }

    #[test]
    fn swapping_one_order_for_another_is_also_a_change() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        let _ = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            orders(&[7, 8])
        });
        approve_the_one_open(&host, now);
        let repeat = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            orders(&[7, 9])
        });
        assert!(
            matches!(repeat, Err(McpError::ConfirmationRequired(_))),
            "same count, different orders"
        );
    }

    #[test]
    fn an_unchanged_target_runs_on_the_approval() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        let _ = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            orders(&[8, 7])
        });
        approve_the_one_open(&host, now);
        // The order of the ids is not a change.
        assert!(
            gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || orders(
                &[7, 8]
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_target_that_vanished_after_approval_is_refused_not_run() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        let _ = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            orders(&[7])
        });
        approve_the_one_open(&host, now);
        let gone = gate_command(&host, now, "c1", "Claude", false, &cancel_all(), || {
            Err(McpError::NotFound("No such account".into()))
        });
        assert!(matches!(gone, Err(McpError::NotFound(_))), "{gone:?}");
    }

    #[test]
    fn stopping_and_starting_the_server_clears_every_question_and_approval() {
        let (host, shared) = host_with(&["c1"], true);
        let now = Instant::now();
        let _ = gate_command(&host, now, "c1", "Claude", false, &alert_command(), say);
        approve_the_one_open(&host, now);
        let _ = gate_command(
            &host,
            now,
            "c1",
            "Claude",
            false,
            &McpCommand::DeleteAlert { alert_id: 2 },
            say,
        );
        assert_eq!(host.lock().unwrap().pending(now).len(), 1);

        // Nothing moved: the frame changes nothing.
        host.lock().unwrap().tick(now);
        assert_eq!(host.lock().unwrap().pending(now).len(), 1);

        // The server stopped (off, or a new port) and came back.
        shared.bump_generation();
        host.lock().unwrap().tick(now);
        assert!(host.lock().unwrap().pending(now).is_empty());
        let again = gate_command(&host, now, "c1", "Claude", false, &alert_command(), say);
        assert!(
            matches!(again, Err(McpError::ConfirmationRequired(_))),
            "the approval did not survive: {again:?}"
        );
        // The question nobody answered is in the log as expired.
        let rows = shared.audit().take(now_ms(), true);
        assert!(
            rows.iter()
                .any(|row| row.outcome == McpAuditOutcome::Expired)
        );
    }

    #[test]
    fn a_gated_call_is_held_then_runs_once_after_the_user_approves() {
        let (host, shared) = host_with(&["c1"], true);
        let now = Instant::now();
        let held =
            gate_command(&host, now, "c1", "Claude", false, &alert_command(), say).unwrap_err();
        let McpError::ConfirmationRequired(info) = held else {
            panic!("{held:?}");
        };
        // The client is told the action; the user reads the target.
        assert_eq!(info.operation, "Delete a price alert");
        assert_eq!(
            host.lock().unwrap().pending(now)[0].summary.line(),
            "Delete a price alert: BTC/USDT crosses above 70,000"
        );

        // Repeating it meanwhile gets the same question and does not describe it again.
        let again = gate_command(&host, now, "c1", "Claude", false, &alert_command(), || {
            panic!("describes a question that is already open")
        })
        .unwrap_err();
        assert!(matches!(again, McpError::ConfirmationRequired(ref a) if a.id == info.id));

        host.lock().unwrap().approve(now, info.id);
        assert!(gate_command(&host, now, "c1", "Claude", false, &alert_command(), say).is_ok());
        // Used up.
        assert!(gate_command(&host, now, "c1", "Claude", false, &alert_command(), say).is_err());

        // The user's answer is in the activity log.
        let rows = shared.audit().take(0, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, McpAuditKind::Confirmation);
        assert_eq!(rows[0].outcome, McpAuditOutcome::Approved);
        assert_eq!(rows[0].tool, "degenview_delete_price_alert");
        assert_eq!(rows[0].client_id, "c1");
    }

    #[test]
    fn a_declined_call_is_refused_with_the_reason_and_logged() {
        let (host, shared) = host_with(&["c1"], true);
        let now = Instant::now();
        let McpError::ConfirmationRequired(info) =
            gate_command(&host, now, "c1", "Claude", false, &alert_command(), say).unwrap_err()
        else {
            panic!("not held");
        };
        host.lock().unwrap().deny(now, info.id);
        match gate_command(&host, now, "c1", "Claude", false, &alert_command(), say) {
            Err(McpError::ConfirmationDenied(operation)) => {
                assert!(operation.contains("Delete a price alert"));
            }
            other => panic!("{other:?}"),
        }
        let rows = shared.audit().take(0, false);
        assert_eq!(rows[0].outcome, McpAuditOutcome::Declined);
    }

    #[test]
    fn free_commands_and_a_switched_off_setting_never_ask() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        let free = McpCommand::ListWorkspaces;
        assert!(
            gate_command(&host, now, "c1", "Claude", false, &free, || panic!(
                "described"
            ))
            .is_ok()
        );

        let (off, _) = host_with(&["c1"], false);
        assert!(
            gate_command(
                &off,
                now,
                "c1",
                "Claude",
                false,
                &alert_command(),
                || panic!("described")
            )
            .is_ok()
        );
    }

    #[test]
    fn a_target_that_cannot_be_described_is_refused_without_asking() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        let missing = || Err(McpError::NotFound("No such alert".into()));
        let error =
            gate_command(&host, now, "c1", "Claude", false, &alert_command(), missing).unwrap_err();
        assert!(matches!(error, McpError::NotFound(_)));
        assert!(
            host.lock().unwrap().pending(now).is_empty(),
            "nobody was asked"
        );
    }

    #[test]
    fn too_many_open_questions_are_refused_not_queued_forever() {
        let (host, _) = host_with(&["c1"], true);
        let now = Instant::now();
        for n in 0..crate::mcp::confirm::MAX_PENDING_PER_CLIENT as u64 {
            let command = McpCommand::DeleteAlert { alert_id: n };
            assert!(gate_command(&host, now, "c1", "Claude", false, &command, say).is_err());
        }
        let command = McpCommand::DeleteAlert { alert_id: 999 };
        let error = gate_command(&host, now, "c1", "Claude", false, &command, say).unwrap_err();
        assert!(matches!(error, McpError::ResourceLimit(_)), "{error:?}");
    }

    #[test]
    fn unanswered_questions_expire_into_the_log_and_new_ones_are_announced_once() {
        let (host, shared) = host_with(&["c1"], true);
        let now = Instant::now();
        let _ = gate_command(&host, now, "c1", "Claude", false, &alert_command(), say);
        assert_eq!(host.lock().unwrap().take_fresh().len(), 1);
        assert!(
            host.lock().unwrap().take_fresh().is_empty(),
            "announced once"
        );

        let late = now + crate::mcp::confirm::PENDING_TTL + std::time::Duration::from_secs(1);
        host.lock().unwrap().tick(late);
        assert!(host.lock().unwrap().pending(late).is_empty());
        let rows = shared.audit().take(0, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, McpAuditOutcome::Expired);
    }

    #[test]
    fn a_revoked_client_loses_its_questions_and_turning_the_setting_off_clears_them() {
        let (host, shared) = host_with(&["c1", "c2"], true);
        let now = Instant::now();
        let _ = gate_command(&host, now, "c1", "Claude", false, &alert_command(), say);
        let _ = gate_command(&host, now, "c2", "Other", false, &alert_command(), say);
        assert_eq!(host.lock().unwrap().pending(now).len(), 2);

        // c1 is revoked.
        shared.replace(
            shared
                .clients()
                .into_iter()
                .filter(|c| c.id != "c1")
                .collect(),
            shared.global(),
        );
        host.lock().unwrap().tick(now);
        let left = host.lock().unwrap().pending(now);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].client_name, "Other");

        shared.set_require_confirmation(false);
        host.lock().unwrap().tick(now);
        assert!(host.lock().unwrap().pending(now).is_empty());
    }

    #[test]
    fn a_remote_credential_keeps_asking_when_the_setting_is_off_and_its_questions_stay_open() {
        let (host, shared) = host_with(&["local", "remote"], true);
        shared.replace(
            shared
                .clients()
                .into_iter()
                .map(|mut c| {
                    c.remote_allowed = c.id == "remote";
                    c
                })
                .collect(),
            shared.global(),
        );
        let now = Instant::now();
        let change = || McpCommand::SetTimeframe {
            workspace_id: 1,
            timeframe: crate::model::Timeframe::Day,
        };
        let described = || Ok(Summary::new("Change the timeframe", "Day"));

        // A change that is easy to undo: free for the local credential, asked of the remote one.
        assert!(gate_command(&host, now, "local", "Local", false, &change(), described).is_ok());
        assert!(matches!(
            gate_command(&host, now, "remote", "Remote", false, &change(), described),
            Err(McpError::ConfirmationRequired(_))
        ));
        let _ = gate_command(&host, now, "local", "Local", false, &alert_command(), say);
        assert_eq!(host.lock().unwrap().pending(now).len(), 2);

        // The setting off ends the local question and not the remote one, and the notification
        // for it is still to be sent.
        shared.set_require_confirmation(false);
        host.lock().unwrap().tick(now);
        let left = host.lock().unwrap().pending(now);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].client_name, "Remote");
        assert_eq!(host.lock().unwrap().take_fresh().len(), 1);

        // And with the setting off, a destructive call from the remote credential still asks,
        // while the local one runs.
        assert!(gate_command(&host, now, "local", "Local", false, &alert_command(), say).is_ok());
        assert!(matches!(
            gate_command(&host, now, "remote", "Remote", false, &alert_command(), say),
            Err(McpError::ConfirmationRequired(_))
        ));
    }

    #[test]
    fn a_change_that_arrived_as_remote_is_still_asked_about_after_remote_access_is_switched_off() {
        // The credential is local now (host_with makes it so), but the request was authenticated
        // as remote and has been waiting in the queue.
        let (host, _shared) = host_with(&["c1"], true);
        let now = Instant::now();
        let change = McpCommand::SetTimeframe {
            workspace_id: 1,
            timeframe: crate::model::Timeframe::Day,
        };
        let described = || Ok(Summary::new("Change the timeframe", "Day"));
        assert!(matches!(
            gate_command(&host, now, "c1", "Claude", true, &change, described),
            Err(McpError::ConfirmationRequired(_))
        ));
        // As a local request from a local credential it would have run.
        assert!(gate_command(&host, now, "c1", "Claude", false, &change, described).is_ok());
    }

    #[test]
    fn numbers_read_like_prices() {
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(70000.0), "70000");
        assert_eq!(number(0.00000001), "0.00000001");
        assert_eq!(number(0.0), "0");
        assert_eq!(plural(1, "market", "markets"), "1 market");
        assert_eq!(plural(0, "market", "markets"), "0 markets");
    }
}
