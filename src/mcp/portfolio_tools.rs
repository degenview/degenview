//! Read-only portfolio tools. The ledger is read on the app thread; the prices that value the
//! holdings come from the app's quote feed on the server's own threads, as for `get_quotes`.

use crate::mcp::bridge::McpError;
use crate::mcp::bridge::{McpCommand, McpReply, REQUEST_TIMEOUT};
use crate::mcp::market_tools::quote_wait;
use crate::mcp::portfolio::{
    DEFAULT_TRANSACTION_PAGE, HoldingsResult, MAX_PRICED_HOLDINGS, MAX_TRANSACTION_PAGE,
    PortfoliosResult, TransactionsResult, value_holdings,
};
use crate::mcp::state::ClientIdentity;
use crate::mcp::tools::{DegenViewMcp, client_of, failure, invalid, unexpected};
use crate::model::InstrumentId;
use crate::model::mcp::Permission;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;

pub const LIST_PORTFOLIOS: &str = "degenview_list_portfolios";
pub const GET_PORTFOLIO_HOLDINGS: &str = "degenview_get_portfolio_holdings";
pub const LIST_PORTFOLIO_TRANSACTIONS: &str = "degenview_list_portfolio_transactions";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HoldingsParams {
    /// A `portfolio_id` from `degenview_list_portfolios`. Omit for every unarchived portfolio
    /// together.
    #[schemars(length(min = 1, max = 20))]
    pub portfolio_id: Option<String>,
    /// Value the holdings at live prices. Default true. Waits up to `wait_seconds` for prices not
    /// known yet.
    pub include_prices: Option<bool>,
    /// How long to wait for prices, 0 to 15 seconds. Default 5.
    #[schemars(range(min = 0, max = 15))]
    pub wait_seconds: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransactionsParams {
    /// A `portfolio_id` from `degenview_list_portfolios`. Omit for every unarchived portfolio.
    #[schemars(length(min = 1, max = 20))]
    pub portfolio_id: Option<String>,
    /// Only this asset, as `degenview_get_portfolio_holdings` names it (`BINANCE:BTCUSDT`).
    #[schemars(length(min = 1, max = 150))]
    pub asset: Option<String>,
    /// Rows to skip, newest first. Default 0.
    pub offset: Option<u32>,
    /// Rows to return, 1 to 200. Default 50.
    #[schemars(range(min = 1, max = 200))]
    pub limit: Option<u32>,
}

fn portfolio_id(raw: Option<&str>) -> Result<Option<u64>, CallToolResult> {
    raw.map(|raw| {
        raw.trim().parse::<u64>().map_err(|_| {
            invalid("portfolio_id must be an id returned by degenview_list_portfolios")
        })
    })
    .transpose()
}

#[tool_router(router = portfolio_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_list_portfolios",
        description = "List the user's portfolios: id, name, base currency, whether archived, how many transactions, and which one the app has selected. `privacy_mode` says whether the app hides amounts on screen; the figures returned are real. Read-only.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_portfolios(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<PortfoliosResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client,
                Permission::PortfolioRead,
                McpCommand::ListPortfolios,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Portfolios(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_portfolio_holdings",
        description = "What a portfolio holds, from its transactions by weighted-average cost: per asset the quantity, cost basis, average cost and realized profit, and (with `include_prices`, default) the live price, value, unrealized profit and return, day change and allocation. Totals are per currency, never added across currencies. A holding bought in a different currency than its market quotes in gets price and value but no profit figure (`currency_mismatch`), since converting needs an exchange rate this read does not use; the Portfolio tab converts. `closed_positions_realized_pnl` is profit on positions sold down to nothing. Omit `portfolio_id` for all unarchived portfolios together. Read-only.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_portfolio_holdings(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<HoldingsParams>,
    ) -> Result<Json<HoldingsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let portfolio_id = portfolio_id(params.portfolio_id.as_deref())?;
        let wait = quote_wait(params.wait_seconds)?;
        let priced = params.include_prices.unwrap_or(true);
        let base = match self
            .bridge
            .call(
                &client,
                Permission::PortfolioRead,
                McpCommand::GetPortfolioHoldings { portfolio_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PortfolioHoldings(base)) => *base,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        let mut quotes: HashMap<String, crate::model::watchlist::quote::WatchlistQuote> =
            HashMap::new();
        let mut capped = false;
        if priced && !base.holdings.is_empty() {
            let mut instruments: Vec<InstrumentId> = Vec::new();
            for entry in &base.holdings {
                if !instruments
                    .iter()
                    .any(|known| known.key() == entry.instrument.key())
                {
                    instruments.push(entry.instrument.clone());
                }
            }
            if instruments.len() > MAX_PRICED_HOLDINGS {
                instruments.truncate(MAX_PRICED_HOLDINGS);
                capped = true;
            }
            let held = self.wait_for_quotes(&instruments, wait).await?;
            self.still_allowed(&client, Permission::PortfolioRead)?;
            for (instrument, quote) in instruments.iter().zip(held) {
                if let Some(quote) = quote {
                    quotes.insert(instrument.key(), quote);
                }
            }
        }
        let mut result = value_holdings(&base, &|id| quotes.get(&id.key()).cloned(), priced);
        if capped {
            result.notes.push(format!(
                "Only the first {MAX_PRICED_HOLDINGS} markets were priced."
            ));
        }
        Ok(Json(result))
    }

    #[tool(
        name = "degenview_list_portfolio_transactions",
        description = "A portfolio's ledger, newest first: kind (buy, sell, transfer, reward, fee, …), asset, quantity, price and its currency, fee, time, notes and where it came from. Filter to one asset by its name from degenview_get_portfolio_holdings. Pages of up to 200 (`offset`, `limit`). Read-only.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_portfolio_transactions(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<TransactionsParams>,
    ) -> Result<Json<TransactionsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let portfolio_id = portfolio_id(params.portfolio_id.as_deref())?;
        let limit = params
            .limit
            .map_or(DEFAULT_TRANSACTION_PAGE, |limit| limit as usize);
        if !(1..=MAX_TRANSACTION_PAGE).contains(&limit) {
            return Err(invalid(format!(
                "limit must be between 1 and {MAX_TRANSACTION_PAGE}"
            )));
        }
        let asset = match params.asset.as_deref().map(str::trim) {
            None => None,
            Some(asset) if (1..=150).contains(&asset.chars().count()) => Some(asset.to_owned()),
            Some(_) => return Err(invalid("asset must be 1 to 150 characters")),
        };
        match self
            .bridge
            .call(
                &client,
                Permission::PortfolioRead,
                McpCommand::ListPortfolioTransactions {
                    portfolio_id,
                    asset,
                    offset: params.offset.unwrap_or(0) as usize,
                    limit,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PortfolioTransactions(result)) => Ok(Json(*result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}

impl DegenViewMcp {
    /// Fails when `client` lost `permission` while the call was waiting.
    pub(super) fn still_allowed(
        &self,
        client: &ClientIdentity,
        permission: Permission,
    ) -> Result<(), CallToolResult> {
        if self.shared.allows(&client.id, permission) {
            Ok(())
        } else {
            Err(failure(&McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
            )))
        }
    }
}
