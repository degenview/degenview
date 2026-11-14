use crate::host::mcp::confirmation;
use crate::host::paper::{PaperHost, PaperHostError, owner_text};
use crate::mcp::bridge::McpError;
use crate::mcp::bridge::McpReply;
use crate::mcp::confirm::Summary;
use crate::mcp::paper::PaperCommand;
use crate::mcp::paper::{
    self, PaperAccountDetail, PaperAccountInfo, PaperAccountsResult, PaperChangeResult,
    PaperFillsResult, PaperOrderResult, PaperTradesResult,
};
use eframe::egui;
use std::sync::{Mutex, MutexGuard};

fn paper_unavailable() -> McpError {
    McpError::AppUnavailable("Paper trading is unavailable right now".into())
}

fn lock(paper: &Mutex<PaperHost>) -> Result<MutexGuard<'_, PaperHost>, McpError> {
    paper.lock().map_err(|_| paper_unavailable())
}

/// Runs a paper-trading command on the app thread. `orders` remembers the orders placed by
/// `request_id`, so a retried call places nothing twice.
pub fn run(
    command: PaperCommand,
    paper: &Mutex<PaperHost>,
    orders: &Mutex<OrderMemory>,
    ctx: &egui::Context,
) -> Result<McpReply, McpError> {
    match command {
        PaperCommand::ListAccounts => {
            list_paper_accounts(&*lock(paper)?).map(McpReply::PaperAccounts)
        }
        PaperCommand::GetAccount { account_id } => get_paper_account(&*lock(paper)?, account_id)
            .map(|detail| McpReply::PaperAccount(Box::new(detail))),
        PaperCommand::ListHistory {
            account_id,
            closed_trades,
            offset,
            limit,
        } => {
            let host = lock(paper)?;
            if closed_trades {
                paper_trades(&host, account_id, offset, limit)
                    .map(|result| McpReply::PaperTrades(Box::new(result)))
            } else {
                paper_fills(&host, account_id, offset, limit)
                    .map(|result| McpReply::PaperFills(Box::new(result)))
            }
        }
        PaperCommand::PlaceOrder {
            request,
            dry_run,
            request_id,
        } => {
            let mut host = lock(paper)?;
            let mut memory = orders.lock().map_err(|_| paper_unavailable())?;
            let result = place_paper_order(
                &mut host,
                &mut memory,
                *request,
                dry_run,
                request_id.as_deref(),
            )?;
            ctx.request_repaint();
            Ok(McpReply::PaperOrder(Box::new(result)))
        }
        PaperCommand::ModifyOrder { order_id, changes } => {
            let result = modify_paper_order(&mut *lock(paper)?, order_id, changes)?;
            ctx.request_repaint();
            Ok(McpReply::PaperChange(Box::new(result)))
        }
        PaperCommand::CancelOrders {
            account_id,
            order_ids,
        } => {
            let result = cancel_paper_orders(&mut *lock(paper)?, account_id, order_ids.as_deref())?;
            ctx.request_repaint();
            Ok(McpReply::PaperChange(Box::new(result)))
        }
        PaperCommand::ClosePositions {
            account_id,
            instrument_key,
        } => {
            let result =
                close_paper_positions(&mut *lock(paper)?, account_id, instrument_key.as_deref())?;
            ctx.request_repaint();
            Ok(McpReply::PaperChange(Box::new(result)))
        }
    }
}

/// What a paper-trading command the user is asked about would do (see [`confirmation`]).
pub fn describe(command: &PaperCommand, paper: &Mutex<PaperHost>) -> Result<Summary, McpError> {
    match command {
        PaperCommand::PlaceOrder { request, .. } => {
            confirmation::place_order(&*lock(paper)?, request)
        }
        PaperCommand::ModifyOrder { order_id, changes } => {
            confirmation::modify_order(&*lock(paper)?, *order_id, changes)
        }
        PaperCommand::CancelOrders {
            account_id,
            order_ids,
        } => confirmation::cancel_orders(&*lock(paper)?, *account_id, order_ids.as_deref()),
        PaperCommand::ClosePositions {
            account_id,
            instrument_key,
        } => confirmation::close_positions(&*lock(paper)?, *account_id, instrument_key.as_deref()),
        _ => confirmation::plainly(command),
    }
}

fn paper_account(
    host: &PaperHost,
    account_id: u64,
) -> Result<&crate::model::paper::PaperAccount, McpError> {
    if host.load_failed() {
        return Err(McpError::AppUnavailable(
            "Paper-trading data could not be read, so it is unavailable".into(),
        ));
    }
    host.snapshot()
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| {
            McpError::NotFound(
                "No such paper account: use an account_id from degenview_list_paper_accounts"
                    .into(),
            )
        })
}

fn paper_account_info(
    host: &PaperHost,
    account: &crate::model::paper::PaperAccount,
) -> PaperAccountInfo {
    paper::account_info(
        account,
        &host.metrics(account.id),
        host.snapshot().selected_account_id == Some(account.id),
        host.owned_elsewhere(account.id).map(owner_text),
    )
}

pub fn list_paper_accounts(host: &PaperHost) -> Result<PaperAccountsResult, McpError> {
    if host.load_failed() {
        return Err(McpError::AppUnavailable(
            "Paper-trading data could not be read, so it is unavailable".into(),
        ));
    }
    Ok(PaperAccountsResult {
        accounts: host
            .snapshot()
            .accounts
            .iter()
            .map(|account| paper_account_info(host, account))
            .collect(),
    })
}

pub fn get_paper_account(
    host: &PaperHost,
    account_id: u64,
) -> Result<PaperAccountDetail, McpError> {
    let account = paper_account(host, account_id)?;
    let snapshot = host.snapshot();
    Ok(PaperAccountDetail {
        account: paper_account_info(host, account),
        positions: snapshot
            .positions
            .iter()
            .filter(|position| position.account_id == account_id && position.signed_quantity != 0.0)
            .map(|position| {
                paper::position_info(
                    position,
                    host.mark(position),
                    host.unrealized_pnl(position),
                    host.return_ratio(position),
                )
            })
            .collect(),
        open_orders: snapshot
            .orders
            .iter()
            .filter(|order| order.account_id == account_id && paper::is_open(order))
            .map(paper::order_info)
            .collect(),
    })
}

pub fn paper_fills(
    host: &PaperHost,
    account_id: u64,
    offset: usize,
    limit: usize,
) -> Result<PaperFillsResult, McpError> {
    paper_account(host, account_id)?;
    let fills: Vec<_> = host
        .snapshot()
        .fills
        .iter()
        .filter(|fill| fill.account_id == account_id)
        .collect();
    Ok(paper::fills_result(account_id, &fills, offset, limit))
}

pub fn paper_trades(
    host: &PaperHost,
    account_id: u64,
    offset: usize,
    limit: usize,
) -> Result<PaperTradesResult, McpError> {
    paper_account(host, account_id)?;
    let trades: Vec<_> = host
        .snapshot()
        .closed_trades
        .iter()
        .filter(|trade| trade.account_id == account_id)
        .collect();
    Ok(paper::trades_result(account_id, &trades, offset, limit))
}

/// Placed orders by `request_id`, so a retry of a call that timed out does not place a second
/// order. Held in memory only and bounded: it is a guard against an agent's retry, not a ledger.
/// A record keeps the request it was made for, so the same id with another order is a conflict
/// rather than a silent answer with the first one.
#[derive(Default)]
pub struct OrderMemory {
    entries: std::collections::VecDeque<(String, crate::model::paper::PaperOrderRequest, u64)>,
}

impl OrderMemory {
    const CAPACITY: usize = 512;

    fn key(account_id: u64, request_id: &str) -> String {
        format!("{account_id}:{request_id}")
    }

    /// The request and order id recorded for `request_id` on `account_id`.
    pub fn recall(
        &self,
        account_id: u64,
        request_id: &str,
    ) -> Option<(&crate::model::paper::PaperOrderRequest, u64)> {
        let key = Self::key(account_id, request_id);
        self.entries
            .iter()
            .find(|(known, _, _)| *known == key)
            .map(|(_, request, order_id)| (request, *order_id))
    }

    /// Drops the record, when the order it names no longer exists.
    pub fn forget(&mut self, account_id: u64, request_id: &str) {
        let key = Self::key(account_id, request_id);
        self.entries.retain(|(known, _, _)| *known != key);
    }

    /// Records `order_id`, replacing any earlier record of the same id.
    pub fn remember(
        &mut self,
        account_id: u64,
        request_id: &str,
        request: &crate::model::paper::PaperOrderRequest,
        order_id: u64,
    ) {
        self.forget(account_id, request_id);
        if self.entries.len() >= Self::CAPACITY {
            self.entries.pop_front();
        }
        self.entries
            .push_back((Self::key(account_id, request_id), request.clone(), order_id));
    }
}

fn paper_error(error: PaperHostError) -> McpError {
    use crate::model::paper::PaperTradingError as E;
    let message = error.to_string();
    match error {
        PaperHostError::Engine(E::AccountNotFound | E::OrderNotFound) => {
            McpError::NotFound(message)
        }
        PaperHostError::Engine(
            E::InvalidQuantity(_) | E::InvalidPrice(_) | E::UnsupportedSymbol,
        ) => McpError::InvalidArgument(message),
        PaperHostError::Engine(_) => McpError::Conflict(message),
        _ => McpError::AppUnavailable(message),
    }
}

/// An account this Mac runs. One another Mac runs is not touched: acting on it from the GUI takes
/// it over, which an agent must not do silently.
fn tradable_account(host: &PaperHost, account_id: u64) -> Result<(), McpError> {
    paper_account(host, account_id)?;
    if let Some(elsewhere) = host.owned_elsewhere(account_id) {
        return Err(McpError::Conflict(format!(
            "{} Change it in DegenView to take it over; it is not changed over MCP.",
            owner_text(elsewhere)
        )));
    }
    Ok(())
}

/// The name of a paper account an agent may trade: it exists and this Mac runs it.
pub(super) fn tradable_paper_account(
    host: &PaperHost,
    account_id: u64,
) -> Result<String, McpError> {
    tradable_account(host, account_id)?;
    Ok(paper_account(host, account_id)?.name.clone())
}

fn account_after(host: &PaperHost, account_id: u64) -> Result<PaperAccountInfo, McpError> {
    paper_account(host, account_id).map(|account| paper_account_info(host, account))
}

fn fills_of(host: &PaperHost, order_ids: &[u64]) -> Vec<paper::PaperFillInfo> {
    host.snapshot()
        .fills
        .iter()
        .filter(|fill| order_ids.contains(&fill.order_id))
        .map(paper::fill_info)
        .collect()
}

fn orders_of(host: &PaperHost, order_ids: &[u64]) -> Vec<paper::PaperOrderInfo> {
    order_ids
        .iter()
        .filter_map(|id| host.snapshot().orders.iter().find(|order| order.id == *id))
        .map(paper::order_info)
        .collect()
}

pub fn place_paper_order(
    host: &mut PaperHost,
    memory: &mut OrderMemory,
    request: crate::model::paper::PaperOrderRequest,
    dry_run: bool,
    request_id: Option<&str>,
) -> Result<PaperOrderResult, McpError> {
    let account_id = request.account_id;
    tradable_account(host, account_id)?;
    if let Some(request_id) = request_id
        && let Some((earlier, order_id)) = memory
            .recall(account_id, request_id)
            .map(|(earlier, order_id)| (earlier.clone(), order_id))
    {
        let ids = [order_id];
        if let Some(order) = orders_of(host, &ids).into_iter().next() {
            if earlier != request {
                return Err(McpError::Conflict(
                    "That request_id was used for a different order. Use a new request_id for a \
                     different order, or repeat the same order to get the first one back"
                        .into(),
                ));
            }
            return Ok(PaperOrderResult {
                status: "already_placed".into(),
                order: Some(order),
                bracket_orders: bracket_of(host, order_id),
                fills: fills_of(host, &ids),
                account: Some(account_after(host, account_id)?),
            });
        }
        // The order is gone (the account was reset): the id is free again.
        memory.forget(account_id, request_id);
    }
    if dry_run {
        host.check_order(request).map_err(paper_error)?;
        return Ok(PaperOrderResult {
            status: "would_place".into(),
            order: None,
            bracket_orders: Vec::new(),
            fills: Vec::new(),
            account: Some(account_after(host, account_id)?),
        });
    }
    let order_id = host.submit(request.clone()).map_err(paper_error)?;
    if let Some(request_id) = request_id {
        memory.remember(account_id, request_id, &request, order_id);
    }
    Ok(PaperOrderResult {
        status: "placed".into(),
        order: orders_of(host, &[order_id]).into_iter().next(),
        bracket_orders: bracket_of(host, order_id),
        fills: fills_of(host, &[order_id]),
        account: Some(account_after(host, account_id)?),
    })
}

/// The exit orders of a bracket, which are children of its entry order.
fn bracket_of(host: &PaperHost, entry_id: u64) -> Vec<paper::PaperOrderInfo> {
    host.snapshot()
        .orders
        .iter()
        .filter(|order| order.parent_order_id == Some(entry_id))
        .map(paper::order_info)
        .collect()
}

pub fn modify_paper_order(
    host: &mut PaperHost,
    order_id: u64,
    changes: crate::model::paper::PaperOrderChanges,
) -> Result<PaperChangeResult, McpError> {
    let account_id = host
        .snapshot()
        .orders
        .iter()
        .find(|order| order.id == order_id)
        .map(|order| order.account_id)
        .ok_or_else(|| McpError::NotFound("No such paper order".into()))?;
    tradable_account(host, account_id)?;
    host.modify(order_id, changes).map_err(paper_error)?;
    Ok(PaperChangeResult {
        status: "modified".into(),
        orders: orders_of(host, &[order_id]),
        fills: fills_of(host, &[order_id]),
        failures: Vec::new(),
        account: Some(account_after(host, account_id)?),
    })
}

pub fn cancel_paper_orders(
    host: &mut PaperHost,
    account_id: u64,
    order_ids: Option<&[u64]>,
) -> Result<PaperChangeResult, McpError> {
    tradable_account(host, account_id)?;
    let working: Vec<u64> = host
        .snapshot()
        .orders
        .iter()
        .filter(|order| order.account_id == account_id && order.status.is_working())
        .map(|order| order.id)
        .collect();
    let targets: Vec<u64> = match order_ids {
        None => working.clone(),
        Some(ids) => ids.to_vec(),
    };
    let (mut done, mut failures) = (Vec::new(), Vec::new());
    for id in &targets {
        // Naming an order of another account is refused, not cancelled.
        let belongs = host
            .snapshot()
            .orders
            .iter()
            .any(|order| order.id == *id && order.account_id == account_id);
        if !belongs {
            failures.push(format!("Order {id}: not an order of this account"));
            continue;
        }
        match host.cancel(*id) {
            Ok(()) => done.push(*id),
            Err(error) => failures.push(format!("Order {id}: {error}")),
        }
    }
    Ok(PaperChangeResult {
        status: if done.is_empty() && failures.is_empty() {
            "nothing_to_do"
        } else {
            "canceled"
        }
        .into(),
        orders: orders_of(host, &done),
        fills: Vec::new(),
        failures,
        account: Some(account_after(host, account_id)?),
    })
}

pub fn close_paper_positions(
    host: &mut PaperHost,
    account_id: u64,
    instrument_key: Option<&str>,
) -> Result<PaperChangeResult, McpError> {
    tradable_account(host, account_id)?;
    let keys: Vec<String> = host
        .snapshot()
        .positions
        .iter()
        .filter(|position| position.account_id == account_id && position.signed_quantity != 0.0)
        .map(|position| position.instrument.key.clone())
        .filter(|key| instrument_key.is_none_or(|wanted| wanted == key))
        .collect();
    if keys.is_empty() && instrument_key.is_some() {
        return Err(McpError::NotFound(
            "That account has no open position in that market".into(),
        ));
    }
    let (mut order_ids, mut failures) = (Vec::new(), Vec::new());
    for key in &keys {
        match host.close_position(account_id, key) {
            Ok(order_id) => order_ids.push(order_id),
            Err(error) => failures.push(format!("{key}: {error}")),
        }
    }
    Ok(PaperChangeResult {
        status: if keys.is_empty() {
            "nothing_to_do"
        } else {
            "closed"
        }
        .into(),
        orders: orders_of(host, &order_ids),
        fills: fills_of(host, &order_ids),
        failures,
        account: Some(account_after(host, account_id)?),
    })
}
