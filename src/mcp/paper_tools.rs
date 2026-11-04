//! Read-only paper-trading tools. There is deliberately no tool that places, changes or cancels
//! an order: that needs the confirmation policy (`docs/mcp-remaining-work.md` item 3) first.

use crate::mcp::bridge::{McpCommand, McpReply, REQUEST_TIMEOUT};
use crate::mcp::paper::{
    DEFAULT_HISTORY_PAGE, MAX_HISTORY_PAGE, PaperAccountDetail, PaperAccountsResult,
    PaperFillsResult, PaperTradesResult,
};
use crate::mcp::tools::{DegenViewMcp, client_of, failure, invalid, unexpected};
use crate::model::mcp::Permission;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub const LIST_PAPER_ACCOUNTS: &str = "degenview_list_paper_accounts";
pub const GET_PAPER_ACCOUNT: &str = "degenview_get_paper_account";
pub const LIST_PAPER_FILLS: &str = "degenview_list_paper_fills";
pub const LIST_PAPER_TRADES: &str = "degenview_list_paper_trades";

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

fn account_id(raw: &str) -> Result<u64, CallToolResult> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid("account_id must be an id returned by degenview_list_paper_accounts"))
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

#[tool_router(router = paper_router, vis = "pub(super)")]
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
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<PaperAccountsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PaperRead,
                McpCommand::ListPaperAccounts,
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PaperAccountParams>,
    ) -> Result<Json<PaperAccountDetail>, CallToolResult> {
        let client = client_of(&parts)?;
        let account_id = account_id(&params.account_id)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PaperRead,
                McpCommand::GetPaperAccount { account_id },
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PaperHistoryParams>,
    ) -> Result<Json<PaperFillsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let account_id = account_id(&params.account_id)?;
        let (offset, limit) = page(&params)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PaperRead,
                McpCommand::ListPaperHistory {
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PaperHistoryParams>,
    ) -> Result<Json<PaperTradesResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let account_id = account_id(&params.account_id)?;
        let (offset, limit) = page(&params)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PaperRead,
                McpCommand::ListPaperHistory {
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
}
