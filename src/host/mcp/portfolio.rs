use crate::host::portfolio::PortfolioHost;
use crate::mcp::bridge::McpError;
use crate::mcp::portfolio::{
    self, HoldingBase, HoldingsBase, PortfolioInfo, PortfoliosResult, TransactionsResult,
};

fn readable_ledger(
    host: &PortfolioHost,
) -> Result<&crate::model::portfolio::PortfolioLedgerSnapshot, McpError> {
    if host.load_failed() {
        return Err(McpError::AppUnavailable(
            "Portfolio data could not be read, so it is unavailable".into(),
        ));
    }
    Ok(host.snapshot())
}

/// The portfolio ids a read covers: the one asked for, or every unarchived portfolio.
fn portfolio_scope(
    ledger: &crate::model::portfolio::PortfolioLedgerSnapshot,
    portfolio_id: Option<u64>,
) -> Result<(Option<PortfolioInfo>, std::collections::HashSet<u64>), McpError> {
    match portfolio_id {
        Some(id) => {
            let portfolio = ledger
                .portfolios
                .iter()
                .find(|portfolio| portfolio.id == id)
                .ok_or_else(|| {
                    McpError::NotFound(
                        "No such portfolio: use a portfolio_id from degenview_list_portfolios"
                            .into(),
                    )
                })?;
            let count = ledger
                .transactions
                .iter()
                .filter(|transaction| transaction.portfolio_id == id)
                .count();
            Ok((
                Some(portfolio::portfolio_info(
                    portfolio,
                    count,
                    ledger.selected_portfolio_id == Some(id),
                )),
                [id].into_iter().collect(),
            ))
        }
        None => Ok((
            None,
            ledger
                .portfolios
                .iter()
                .filter(|portfolio| !portfolio.is_archived)
                .map(|portfolio| portfolio.id)
                .collect(),
        )),
    }
}

pub fn list_portfolios(host: &PortfolioHost) -> Result<PortfoliosResult, McpError> {
    let ledger = readable_ledger(host)?;
    Ok(PortfoliosResult {
        portfolios: ledger
            .portfolios
            .iter()
            .map(|portfolio| {
                portfolio::portfolio_info(
                    portfolio,
                    ledger
                        .transactions
                        .iter()
                        .filter(|transaction| transaction.portfolio_id == portfolio.id)
                        .count(),
                    ledger.selected_portfolio_id == Some(portfolio.id),
                )
            })
            .collect(),
        privacy_mode: host.privacy_mode(),
    })
}

pub fn portfolio_holdings_base(
    host: &PortfolioHost,
    portfolio_id: Option<u64>,
    now_ms: i64,
) -> Result<HoldingsBase, McpError> {
    let ledger = readable_ledger(host)?;
    let (info, ids) = portfolio_scope(ledger, portfolio_id)?;
    let (holdings, closed) = crate::model::portfolio::accounting::holdings_and_closed(
        &ledger.transactions,
        &ids,
        None,
        &std::collections::HashMap::new(),
        now_ms,
    )
    .map_err(|error| {
        McpError::Conflict(format!(
            "The ledger cannot be replayed, so there are no holdings: {}",
            error.message()
        ))
    })?;
    Ok(HoldingsBase {
        portfolio: info,
        holdings: holdings
            .into_iter()
            .map(|holding| {
                let mut currencies = ledger
                    .transactions
                    .iter()
                    .filter(|transaction| {
                        ids.contains(&transaction.portfolio_id)
                            && transaction.asset.key == holding.asset.key
                            && transaction.price.is_some()
                    })
                    .map(|transaction| transaction.price_currency);
                let first = currencies.next();
                let cost_currency = first.filter(|first| currencies.all(|next| next == *first));
                HoldingBase {
                    instrument: portfolio::instrument_of(
                        holding.asset.source,
                        &holding.asset.key,
                        &holding.asset.symbol,
                    ),
                    cost_currency,
                    holding,
                }
            })
            .collect(),
        closed_realized: closed
            .into_iter()
            .map(|closed| (closed.asset.key, closed.realized_pnl))
            .collect(),
    })
}

pub fn portfolio_transactions(
    host: &PortfolioHost,
    portfolio_id: Option<u64>,
    asset: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<TransactionsResult, McpError> {
    let ledger = readable_ledger(host)?;
    let (_, ids) = portfolio_scope(ledger, portfolio_id)?;
    let wanted = asset.map(str::to_uppercase);
    let rows: Vec<_> = ledger
        .transactions
        .iter()
        .filter(|transaction| ids.contains(&transaction.portfolio_id))
        .filter(|transaction| {
            wanted
                .as_deref()
                .is_none_or(|wanted| transaction.asset.key.to_uppercase() == wanted)
        })
        .collect();
    Ok(portfolio::transactions_result(&rows, offset, limit))
}
