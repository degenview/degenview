//! Paper-trading tools: read accounts, positions, orders and history, and place, change and
//! cancel simulated orders and close simulated positions. Simulated money only: nothing here
//! reaches a broker, and Alpaca stays a data source.
//!
//! Writing is split over three off-by-default permissions (place and change, cancel, close), an
//! agent never acts on an account another Mac runs, a call can be a dry run, and a `request_id`
//! makes a retried call place nothing twice. On top of these, with "Ask before high-impact
//! requests" on (the default), every write here except a dry run waits for the user's approval in
//! the app (`mcp::confirm`): the first call answers `CONFIRMATION_REQUIRED`, and the same call
//! repeated after the user approves runs once.

use crate::mcp::bridge::{McpReply, REQUEST_TIMEOUT};
use crate::mcp::paper::{
    DEFAULT_HISTORY_PAGE, MAX_HISTORY_PAGE, PaperAccountDetail, PaperAccountInfo,
    PaperAccountsResult, PaperChangeResult, PaperCommand, PaperFillsResult, PaperOrderResult,
    PaperTradesResult,
};
use crate::mcp::state::ClientIdentity;
use crate::mcp::tools::{Caller, DegenViewMcp, McpSource, failure, invalid, parse_id, unexpected};
use crate::model::mcp::Permission;
use crate::model::paper::{
    PaperInstrument, PaperOrderChanges, PaperOrderRequest, PaperOrderSide, PaperOrderType,
    PaperTimeInForce,
};
use crate::model::watchlist::text::market_item;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub const LIST_PAPER_ACCOUNTS: &str = "degenview_list_paper_accounts";
pub const GET_PAPER_ACCOUNT: &str = "degenview_get_paper_account";
pub const LIST_PAPER_FILLS: &str = "degenview_list_paper_fills";
pub const LIST_PAPER_TRADES: &str = "degenview_list_paper_trades";
pub const PLACE_PAPER_ORDER: &str = "degenview_place_paper_order";
pub const MODIFY_PAPER_ORDER: &str = "degenview_modify_paper_order";
pub const CANCEL_PAPER_ORDERS: &str = "degenview_cancel_paper_orders";
pub const CLOSE_PAPER_POSITIONS: &str = "degenview_close_paper_positions";

/// The largest quantity or price a call may carry: far past anything real, short of overflow.
const MAX_NUMBER: f64 = 1e15;
const MAX_ORDER_IDS: usize = 50;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaperAccountParams {
    /// An `account_id` from `degenview_list_paper_accounts`.
    #[schemars(length(min = 1, max = 20))]
    pub account_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaperHistoryParams {
    /// An `account_id` from `degenview_list_paper_accounts`.
    #[schemars(length(min = 1, max = 20))]
    pub account_id: String,
    /// Rows to skip, newest first. Default 0.
    pub offset: Option<u32>,
    /// Rows to return, 1 to 200. Default 50.
    #[schemars(range(min = 1, max = 200))]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SideArg {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderTypeArg {
    Market,
    Limit,
    Stop,
    StopLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForceArg {
    Day,
    GoodTilCanceled,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceOrderParams {
    /// An `account_id` from `degenview_list_paper_accounts`.
    #[schemars(length(min = 1, max = 20))]
    pub account_id: String,
    /// The market data provider of the market to trade.
    pub source: McpSource,
    /// The provider's own symbol, as `degenview_search_symbols` returns it. A chart of this
    /// market must be open in DegenView (`degenview_add_chart`): paper orders fill from chart
    /// prices, so without one there is no price and the order is refused.
    #[schemars(length(min = 1, max = 128))]
    pub symbol: String,
    pub side: SideArg,
    pub order_type: OrderTypeArg,
    /// In units of the asset (shares for stocks). Positive.
    pub quantity: f64,
    /// Required for `limit` and `stop_limit`, forbidden otherwise.
    pub limit_price: Option<f64>,
    /// Required for `stop` and `stop_limit`, forbidden otherwise.
    pub stop_price: Option<f64>,
    /// Default `good_til_canceled`.
    pub time_in_force: Option<TimeInForceArg>,
    /// A take-profit price: an exit order placed once the entry fills.
    pub take_profit: Option<f64>,
    /// A stop-loss price: an exit order placed once the entry fills.
    pub stop_loss: Option<f64>,
    /// Only check the order against the account and the market: nothing is placed, recorded or
    /// reserved. Answers `would_place`, or why it would be refused.
    pub dry_run: Option<bool>,
    /// Your own id for this order, 1 to 64 letters, digits, `-`, `_` or `.`. Calling again with
    /// the same id for the same account returns the order already placed (`already_placed`)
    /// instead of placing another, so a call that timed out can be retried safely. The same id
    /// with a different order is refused (`CONFLICT`). Remembered until DegenView quits, and
    /// forgotten if the order no longer exists (the account was reset).
    #[schemars(length(min = 1, max = 64))]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModifyOrderParams {
    /// An `order_id` of a working order, from `degenview_get_paper_account`.
    #[schemars(length(min = 1, max = 20))]
    pub order_id: String,
    pub quantity: Option<f64>,
    pub limit_price: Option<f64>,
    pub stop_price: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelOrdersParams {
    /// An `account_id` from `degenview_list_paper_accounts`.
    #[schemars(length(min = 1, max = 20))]
    pub account_id: String,
    /// The working orders to cancel, up to 50.
    #[schemars(length(min = 1, max = 50))]
    pub order_ids: Option<Vec<String>>,
    /// Cancel every working order of the account. Give this or `order_ids`.
    pub all: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClosePositionsParams {
    /// An `account_id` from `degenview_list_paper_accounts`.
    #[schemars(length(min = 1, max = 20))]
    pub account_id: String,
    /// The market of the position to close; give with `symbol`.
    pub source: Option<McpSource>,
    #[schemars(length(min = 1, max = 128))]
    pub symbol: Option<String>,
    /// Close every open position of the account. Give this or a market.
    pub all: Option<bool>,
}

/// A quantity or price: finite, positive and within reason.
fn positive(what: &str, value: f64) -> Result<f64, CallToolResult> {
    if value.is_finite() && value > 0.0 && value <= MAX_NUMBER {
        Ok(value)
    } else {
        Err(invalid(format!("{what} must be a positive number")))
    }
}

fn optional_price(what: &str, value: Option<f64>) -> Result<Option<f64>, CallToolResult> {
    value.map(|value| positive(what, value)).transpose()
}

fn clean_request_id(raw: Option<&str>) -> Result<Option<String>, CallToolResult> {
    raw.map(|raw| {
        let id = raw.trim();
        if (1..=64).contains(&id.chars().count())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            Ok(id.to_owned())
        } else {
            Err(invalid(
                "request_id is 1 to 64 letters, digits, '-', '_' or '.'",
            ))
        }
    })
    .transpose()
}

/// The order a call asks for, or the first argument that is wrong for its type.
fn order_request(
    params: &PlaceOrderParams,
    account: u64,
) -> Result<PaperOrderRequest, CallToolResult> {
    use OrderTypeArg as T;
    let source = params.source.data_source();
    let item = market_item(source, &params.symbol).map_err(invalid)?;
    let quantity = positive("quantity", params.quantity)?;
    let limit = optional_price("limit_price", params.limit_price)?;
    let stop = optional_price("stop_price", params.stop_price)?;
    let (needs_limit, needs_stop) = match params.order_type {
        T::Market => (false, false),
        T::Limit => (true, false),
        T::Stop => (false, true),
        T::StopLimit => (true, true),
    };
    if needs_limit != limit.is_some() {
        return Err(invalid(if needs_limit {
            "this order type needs a limit_price"
        } else {
            "limit_price only applies to limit and stop_limit orders"
        }));
    }
    if needs_stop != stop.is_some() {
        return Err(invalid(if needs_stop {
            "this order type needs a stop_price"
        } else {
            "stop_price only applies to stop and stop_limit orders"
        }));
    }
    let instrument = PaperInstrument::chart(
        &item.instrument.symbol,
        source.display_symbol(&item.instrument.symbol),
        source,
    );
    Ok(PaperOrderRequest {
        account_id: account,
        instrument,
        side: match params.side {
            SideArg::Buy => PaperOrderSide::Buy,
            SideArg::Sell => PaperOrderSide::Sell,
        },
        kind: match params.order_type {
            T::Market => PaperOrderType::Market,
            T::Limit => PaperOrderType::Limit,
            T::Stop => PaperOrderType::Stop,
            T::StopLimit => PaperOrderType::StopLimit,
        },
        quantity,
        limit_price: limit,
        stop_price: stop,
        time_in_force: match params.time_in_force {
            None | Some(TimeInForceArg::GoodTilCanceled) => PaperTimeInForce::GoodTilCanceled,
            Some(TimeInForceArg::Day) => PaperTimeInForce::Day,
        },
        take_profit: optional_price("take_profit", params.take_profit)?,
        stop_loss: optional_price("stop_loss", params.stop_loss)?,
    })
}

fn order_id(raw: &str) -> Result<u64, CallToolResult> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid("an order id is an order_id returned by degenview_get_paper_account"))
}

fn account_id(raw: &str) -> Result<u64, CallToolResult> {
    parse_id(raw, "account_id", "degenview_list_paper_accounts")
}

fn page(params: &PaperHistoryParams) -> Result<(usize, usize), CallToolResult> {
    let limit = params
        .limit
        .map_or(DEFAULT_HISTORY_PAGE, |limit| limit as usize);
    if !(1..=MAX_HISTORY_PAGE).contains(&limit) {
        return Err(invalid(format!(
            "limit must be between 1 and {MAX_HISTORY_PAGE}"
        )));
    }
    Ok((params.offset.unwrap_or(0) as usize, limit))
}

impl DegenViewMcp {
    /// A write grant is not a read grant: the account (cash, equity, margin) goes back only to a
    /// credential that holds `paper.read` now, checked after the write ran.
    fn hide_unread_account(&self, client: &ClientIdentity, account: &mut Option<PaperAccountInfo>) {
        if !self.shared.allows(&client.id, Permission::PaperRead) {
            *account = None;
        }
    }
}

#[tool_router(router = paper_router, vis = "pub(in crate::mcp)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_list_paper_accounts",
        description = "List the paper-trading accounts (simulated money): name, currency, cash, equity, realized and unrealized profit and loss, margin and available funds, and which one is selected. An account another Mac runs says so in `run_by_another_mac`. Read-only: orders cannot be placed or changed over MCP.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_paper_accounts(
        &self,
        Caller(client): Caller,
    ) -> Result<Json<PaperAccountsResult>, CallToolResult> {
        match self
            .bridge
            .call(
                &client,
                Permission::PaperRead,
                PaperCommand::ListAccounts,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperAccounts(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_paper_account",
        description = "One paper account with its open positions (side, quantity, entry, mark price, unrealized profit and loss, return) and the orders that can still fill (pending, working, partly filled, with bracket roles). A position without a quote has a null mark and no profit figure. Fills and closed trades are separate: degenview_list_paper_fills, degenview_list_paper_trades.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_paper_account(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PaperAccountParams>,
    ) -> Result<Json<PaperAccountDetail>, CallToolResult> {
        let account_id = account_id(&params.account_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::PaperRead,
                PaperCommand::GetAccount { account_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperAccount(detail)) => Ok(Json(*detail)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_paper_fills",
        description = "The fills of a paper account, newest first: market, side, quantity, price, commission, what the price was taken from (bid/ask or a fallback), and time. Pages of up to 200 (`offset`, `limit`).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_paper_fills(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PaperHistoryParams>,
    ) -> Result<Json<PaperFillsResult>, CallToolResult> {
        let account_id = account_id(&params.account_id)?;
        let (offset, limit) = page(&params)?;
        match self
            .bridge
            .call(
                &client,
                Permission::PaperRead,
                PaperCommand::ListHistory {
                    account_id,
                    closed_trades: false,
                    offset,
                    limit,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperFills(result)) => Ok(Json(*result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_paper_trades",
        description = "The closed trades of a paper account, newest exit first: side, quantity, entry and exit price and time, gross and net profit and loss. The reply also totals winners, losers and net profit and loss over all of the account's trades, not only the page. Pages of up to 200 (`offset`, `limit`).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_paper_trades(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PaperHistoryParams>,
    ) -> Result<Json<PaperTradesResult>, CallToolResult> {
        let account_id = account_id(&params.account_id)?;
        let (offset, limit) = page(&params)?;
        match self
            .bridge
            .call(
                &client,
                Permission::PaperRead,
                PaperCommand::ListHistory {
                    account_id,
                    closed_trades: true,
                    offset,
                    limit,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperTrades(result)) => Ok(Json(*result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_place_paper_order",
        description = "Place a simulated order in a paper-trading account: market, limit, stop or stop-limit, buy or sell, optionally with take-profit and stop-loss exits. Simulated money only: nothing reaches a broker. The order fills from the prices of an open DegenView chart of that market (degenview_add_chart first; with none the order is refused for lack of a price) and, while it works, fills only while DegenView runs. The reply is the order as it stands (`filled`, `working`, or the refusal), the fills it produced and, if the credential also holds paper.read, the account after. Use `dry_run` to check an order without placing it, and `request_id` so a retry after a timeout cannot place it twice. An account another Mac runs is refused. Check degenview_get_paper_account for what is open afterwards. Replay does not hold paper orders back: they fill at live prices. A dry run (dry_run: true) is never asked about. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn place_paper_order(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PlaceOrderParams>,
    ) -> Result<Json<PaperOrderResult>, CallToolResult> {
        let account = account_id(&params.account_id)?;
        let request = order_request(&params, account)?;
        let request_id = clean_request_id(params.request_id.as_deref())?;
        match self
            .bridge
            .call(
                &client,
                Permission::PaperOrdersPlace,
                PaperCommand::PlaceOrder {
                    request: Box::new(request),
                    dry_run: params.dry_run.unwrap_or(false),
                    request_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperOrder(result)) => {
                let mut result = *result;
                self.hide_unread_account(&client, &mut result.account);
                Ok(Json(result))
            }
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_modify_paper_order",
        description = "Change a working paper order's quantity, limit price or stop price (only the ones that apply to its type). Returns the order afterwards and, if the credential also holds paper.read, the account. An order that has filled, been canceled or been rejected cannot be changed. An account another Mac runs is refused. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn modify_paper_order(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<ModifyOrderParams>,
    ) -> Result<Json<PaperChangeResult>, CallToolResult> {
        let order_id = order_id(&params.order_id)?;
        let changes = PaperOrderChanges {
            quantity: optional_price("quantity", params.quantity)?,
            limit_price: optional_price("limit_price", params.limit_price)?,
            stop_price: optional_price("stop_price", params.stop_price)?,
        };
        if changes == PaperOrderChanges::default() {
            return Err(invalid("give quantity, limit_price or stop_price"));
        }
        match self
            .bridge
            .call(
                &client,
                Permission::PaperOrdersPlace,
                PaperCommand::ModifyOrder { order_id, changes },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperChange(result)) => {
                let mut result = *result;
                self.hide_unread_account(&client, &mut result.account);
                Ok(Json(result))
            }
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_cancel_paper_orders",
        description = "Cancel working paper orders: the ones named in `order_ids` (up to 50), or with `all` every working order of the account. An order that is not the account's, or can no longer be canceled, is reported in `failures`; the rest are canceled. `nothing_to_do` when no order was working. An account another Mac runs is refused. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn cancel_paper_orders(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<CancelOrdersParams>,
    ) -> Result<Json<PaperChangeResult>, CallToolResult> {
        let account_id = account_id(&params.account_id)?;
        let order_ids = match (&params.order_ids, params.all.unwrap_or(false)) {
            (Some(ids), false) if !ids.is_empty() && ids.len() <= MAX_ORDER_IDS => Some(
                ids.iter()
                    .map(|id| order_id(id))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            (None, true) => None,
            _ => {
                return Err(invalid(format!(
                    "give order_ids (1 to {MAX_ORDER_IDS}) or all: true, not both and not neither"
                )));
            }
        };
        match self
            .bridge
            .call(
                &client,
                Permission::PaperOrdersCancel,
                PaperCommand::CancelOrders {
                    account_id,
                    order_ids,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperChange(result)) => {
                let mut result = *result;
                self.hide_unread_account(&client, &mut result.account);
                Ok(Json(result))
            }
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_close_paper_positions",
        description = "Close paper positions at the market: the one in a given market (`source` + `symbol`), or with `all` every open position of the account. Each close is a market order against the position, so it needs a current price from an open chart of that market. A position that cannot be closed is reported in `failures`; the others still close. `nothing_to_do` when `all` and nothing is open. An account another Mac runs is refused. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn close_paper_positions(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<ClosePositionsParams>,
    ) -> Result<Json<PaperChangeResult>, CallToolResult> {
        let account_id = account_id(&params.account_id)?;
        let instrument_key = match (&params.source, &params.symbol, params.all.unwrap_or(false)) {
            (Some(source), Some(symbol), false) => {
                let source = source.data_source();
                let item = market_item(source, symbol).map_err(invalid)?;
                Some(
                    PaperInstrument::chart(
                        &item.instrument.symbol,
                        source.display_symbol(&item.instrument.symbol),
                        source,
                    )
                    .key,
                )
            }
            (None, None, true) => None,
            _ => {
                return Err(invalid(
                    "give source and symbol, or all: true, not both and not neither",
                ));
            }
        };
        match self
            .bridge
            .call(
                &client,
                Permission::PaperPositionsClose,
                PaperCommand::ClosePositions {
                    account_id,
                    instrument_key,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PaperChange(result)) => {
                let mut result = *result;
                self.hide_unread_account(&client, &mut result.account);
                Ok(Json(result))
            }
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}
