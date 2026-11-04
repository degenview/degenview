//! The market and watchlist tools: symbol search, quotes, and watchlists.
//!
//! Search and quotes need no app state, so they run on the server's own threads: a provider
//! search on a blocking worker, and a quote through a short-lived demand on the app's quote feed
//! (`McpShared::lease_quotes`), released however the call ends. Watchlists are user data, so
//! they go through the bridge and run on the app thread like every other mutation.

use crate::mcp::bridge::{McpCommand, McpError, McpReply, REQUEST_TIMEOUT};
use crate::mcp::markets::{
    MAX_EDIT_MARKETS, MAX_ENTRY_PAGE, MAX_QUOTE_MARKETS, QuoteInfo, QuotesResult,
    SearchSourceError, SymbolSearchResult, WatchlistChangeResult, WatchlistDetail,
    WatchlistsResult, quote_info, source_id, symbol_hit,
};
use crate::mcp::state::ClientIdentity;
use crate::mcp::tools::{
    DegenViewMcp, McpSource, alpaca_configured, client_of, failure, invalid, unexpected,
};
use crate::model::mcp::Permission;
use crate::model::watchlist::WatchlistInstrument;
use crate::model::watchlist_quote::{Freshness, WatchlistQuote};
use crate::model::watchlist_text::market_item;
use crate::model::{DataSource, InstrumentId};
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const SEARCH_SYMBOLS: &str = "degenview_search_symbols";
pub const GET_QUOTES: &str = "degenview_get_quotes";
pub const LIST_WATCHLISTS: &str = "degenview_list_watchlists";
pub const GET_WATCHLIST: &str = "degenview_get_watchlist";
pub const CREATE_WATCHLIST: &str = "degenview_create_watchlist";
pub const UPDATE_WATCHLIST: &str = "degenview_update_watchlist";
pub const DELETE_WATCHLIST: &str = "degenview_delete_watchlist";

/// Results per provider when the caller does not say.
const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 25;
const MAX_QUERY_CHARS: usize = 64;
/// A whole search, across providers. The slowest provider decides, so this is the ceiling.
const SEARCH_DEADLINE: Duration = Duration::from_secs(20);
/// How long a quote request waits for prices when the caller does not say.
const DEFAULT_QUOTE_WAIT: Duration = Duration::from_secs(5);
const MAX_QUOTE_WAIT_SECS: u32 = 15;
const QUOTE_POLL: Duration = Duration::from_millis(100);
/// Entries a watchlist page holds when the caller does not say.
const DEFAULT_ENTRY_PAGE: usize = 100;

/// Providers searched when the caller names none: every one a market can be charted from.
const ALL_SOURCES: [McpSource; 5] = [
    McpSource::Binance,
    McpSource::Coinbase,
    McpSource::Coingecko,
    McpSource::Dexscreener,
    McpSource::Alpaca,
];

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarketRef {
    /// The market data provider.
    pub source: McpSource,
    /// The provider's own symbol, as `degenview_search_symbols` returns it: `BTCUSDT` (binance),
    /// `BTC-USD` (coinbase), `bitcoin` (coingecko), `chain/address` (dexscreener), `AAPL`
    /// (alpaca).
    #[schemars(length(min = 1, max = 128))]
    pub symbol: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSymbolsParams {
    /// What to look for: a ticker, a name or part of one (`btc`, `apple`).
    #[schemars(length(min = 1, max = 64))]
    pub query: String,
    /// Providers to search. Default: all of them. Alpaca (US stocks) needs API keys in Settings.
    #[schemars(length(min = 1, max = 5))]
    pub sources: Option<Vec<McpSource>>,
    /// Most results per provider, 1 to 25. Default 10.
    #[schemars(range(min = 1, max = 25))]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetQuotesParams {
    /// The markets to price, up to 50.
    #[schemars(length(min = 1, max = 50))]
    pub markets: Vec<MarketRef>,
    /// How long to wait for prices that are not known yet, 0 to 15 seconds. Default 5.
    #[schemars(range(min = 0, max = 15))]
    pub wait_seconds: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchlistIdParams {
    /// A `watchlist_id` from `degenview_list_watchlists`.
    #[schemars(length(min = 1, max = 20))]
    pub watchlist_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetWatchlistParams {
    /// A `watchlist_id` from `degenview_list_watchlists`.
    #[schemars(length(min = 1, max = 20))]
    pub watchlist_id: String,
    /// Entries to skip (sections count as entries). Default 0.
    pub offset: Option<u32>,
    /// Entries to return, 1 to 500. Default 100. With `include_quotes`, at most 50 of them may
    /// be markets.
    #[schemars(range(min = 1, max = 500))]
    pub limit: Option<u32>,
    /// Also read each market's price, 24-hour change and volume (needs the markets.read
    /// permission too). Waits up to `wait_seconds` for prices not known yet.
    pub include_quotes: Option<bool>,
    /// How long to wait for quotes, 0 to 15 seconds. Default 5.
    #[schemars(range(min = 0, max = 15))]
    pub wait_seconds: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWatchlistParams {
    /// The list's name: up to 200 characters, no control characters.
    #[schemars(length(min = 1, max = 200))]
    pub name: String,
    /// Markets to put in it, up to 100.
    #[schemars(length(max = 100))]
    pub markets: Option<Vec<MarketRef>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWatchlistParams {
    /// A `watchlist_id` from `degenview_list_watchlists`.
    #[schemars(length(min = 1, max = 20))]
    pub watchlist_id: String,
    /// A new name.
    #[schemars(length(min = 1, max = 200))]
    pub rename: Option<String>,
    /// Markets to add at the end of the list, up to 100. One already in the list is skipped.
    #[schemars(length(max = 100))]
    pub add: Option<Vec<MarketRef>>,
    /// Markets to remove, up to 100. One not in the list is skipped.
    #[schemars(length(max = 100))]
    pub remove: Option<Vec<MarketRef>>,
}

fn parse_watchlist_id(raw: &str) -> Result<u64, CallToolResult> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid("watchlist_id must be an id returned by degenview_list_watchlists"))
}

/// The rows an add would make, one per distinct market (`InstrumentId::key` decides which are the
/// same), or the first market that is not shaped like one on its provider.
fn resolve_markets(refs: &[MarketRef]) -> Result<Vec<WatchlistInstrument>, CallToolResult> {
    if refs.len() > MAX_EDIT_MARKETS {
        return Err(invalid(format!(
            "give at most {MAX_EDIT_MARKETS} markets per call"
        )));
    }
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for market in refs {
        let item = market_item(market.source.data_source(), &market.symbol).map_err(|reason| {
            invalid(format!(
                "{} {}: {reason}",
                source_id(market.source.data_source()),
                market.symbol
            ))
        })?;
        if seen.insert(item.instrument.key()) {
            items.push(item);
        }
    }
    Ok(items)
}

/// A quote that will not change by waiting longer: priced, or refused with a reason.
fn is_settled(quote: Option<&WatchlistQuote>) -> bool {
    quote.is_some_and(|quote| {
        quote.freshness.is_current()
            || quote.freshness == Freshness::Unsupported
            || quote.note.is_some()
    })
}

pub(super) fn quote_wait(seconds: Option<u32>) -> Result<Duration, CallToolResult> {
    match seconds {
        None => Ok(DEFAULT_QUOTE_WAIT),
        Some(seconds) if seconds <= MAX_QUOTE_WAIT_SECS => {
            Ok(Duration::from_secs(u64::from(seconds)))
        }
        Some(_) => Err(invalid(format!(
            "wait_seconds must be between 0 and {MAX_QUOTE_WAIT_SECS}"
        ))),
    }
}

fn clean_query(raw: &str) -> Result<String, CallToolResult> {
    let query = raw.trim();
    if query.is_empty() {
        return Err(invalid("query is empty"));
    }
    if query.chars().count() > MAX_QUERY_CHARS || query.chars().any(char::is_control) {
        return Err(invalid(format!(
            "query must be plain text of at most {MAX_QUERY_CHARS} characters"
        )));
    }
    Ok(query.to_owned())
}

/// Searches `sources` side by side. A provider that fails is reported on its own; the others
/// still answer.
fn search_all(
    search: &crate::mcp::state::MarketSearch,
    sources: &[McpSource],
    query: &str,
    alpaca_ready: bool,
) -> Vec<(McpSource, Result<Vec<crate::service::SearchResult>, String>)> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .iter()
            .map(|source| {
                let source = *source;
                scope.spawn(move || {
                    let data = source.data_source();
                    if data == DataSource::Alpaca && !alpaca_ready {
                        return (
                            source,
                            Err("Alpaca needs API credentials: add them in DegenView Settings > API Keys"
                                .to_owned()),
                        );
                    }
                    (source, search(data, query))
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(sources)
            .map(|(handle, source)| {
                handle
                    .join()
                    .unwrap_or_else(|_| (*source, Err("The search failed".to_owned())))
            })
            .collect()
    })
}

#[tool_router(router = market_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_search_symbols",
        description = "Search the market data providers for symbols by ticker or name. Returns, per hit, the `source` and `symbol` to pass to degenview_add_chart, degenview_get_quotes, degenview_create_price_alert and the watchlist tools, plus how the app labels it. Searches Binance, Coinbase, CoinGecko, DEXScreener and (with API keys) Alpaca stocks side by side; a provider that fails is reported in `source_errors` while the others still answer. Hits only prove the provider lists the market; ask degenview_get_quotes for a price.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn search_symbols(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SearchSymbolsParams>,
    ) -> Result<Json<SymbolSearchResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let query = clean_query(&params.query)?;
        let limit = params
            .limit
            .map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);
        if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
            return Err(invalid(format!(
                "limit must be between 1 and {MAX_SEARCH_LIMIT}"
            )));
        }
        let mut sources: Vec<McpSource> = Vec::new();
        let asked = params.sources.unwrap_or_else(|| ALL_SOURCES.to_vec());
        if asked.is_empty() {
            return Err(invalid(
                "sources is empty: name at least one provider or omit it",
            ));
        }
        for source in asked {
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        let Some(permit) = self.shared.try_search_job() else {
            return Err(failure(&McpError::ResourceLimit(
                "Too many searches are running; try again shortly".into(),
            )));
        };
        let alpaca_ready = !sources.contains(&McpSource::Alpaca) || alpaca_configured().await;
        let search = self.shared.market_search();
        let worker = {
            let (query, sources) = (query.clone(), sources.clone());
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                search_all(&search, &sources, &query, alpaca_ready)
            })
        };
        let outcomes = match tokio::time::timeout(SEARCH_DEADLINE, worker).await {
            Ok(Ok(outcomes)) => outcomes,
            Ok(Err(_)) => {
                return Err(failure(&McpError::AppUnavailable(
                    "The search was interrupted".into(),
                )));
            }
            Err(_) => return Err(failure(&McpError::Timeout)),
        };
        if !self.shared.allows(&client.id, Permission::MarketsRead) {
            return Err(failure(&McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
            )));
        }

        let (mut results, mut source_errors, mut truncated) = (Vec::new(), Vec::new(), false);
        for (source, outcome) in outcomes {
            match outcome {
                Ok(hits) => {
                    let wanted = source.data_source();
                    let mut hits: Vec<_> = hits
                        .into_iter()
                        .filter(|hit| hit.source == wanted && !hit.source.is_prediction_market())
                        .collect();
                    truncated |= hits.len() > limit;
                    hits.truncate(limit);
                    results.extend(hits.iter().map(symbol_hit));
                }
                Err(message) => source_errors.push(SearchSourceError {
                    source: source_id(source.data_source()).to_owned(),
                    message: message.chars().take(300).collect(),
                }),
            }
        }
        Ok(Json(SymbolSearchResult {
            query,
            results,
            truncated,
            source_errors,
        }))
    }

    #[tool(
        name = "degenview_get_quotes",
        description = "Read the last price, 24-hour change (previous-close change for stocks) and volume of up to 50 markets, from the same live feed the app's watchlists use. A price that is not known yet is waited for up to `wait_seconds` (default 5); markets still unpriced are returned with status `unavailable` and counted in `pending`, so ask again. `status` is `live`, `stale`, `market_closed` (a stock outside US hours: the last price is the close), `unavailable` or `unsupported`; a `note` says why a provider gave nothing. Works for any market, not only ones on a chart or watchlist.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_quotes(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetQuotesParams>,
    ) -> Result<Json<QuotesResult>, CallToolResult> {
        let client = client_of(&parts)?;
        if params.markets.is_empty() || params.markets.len() > MAX_QUOTE_MARKETS {
            return Err(invalid(format!(
                "give between 1 and {MAX_QUOTE_MARKETS} markets"
            )));
        }
        let wait = quote_wait(params.wait_seconds)?;
        let markets: Vec<InstrumentId> = resolve_markets(&params.markets)?
            .into_iter()
            .map(|item| item.instrument)
            .collect();
        let (quotes, pending) = self.read_quotes(&client, markets, wait).await?;
        Ok(Json(QuotesResult { quotes, pending }))
    }

    #[tool(
        name = "degenview_list_watchlists",
        description = "List the user's watchlists: id, name, how many markets and sections each holds, and which one is Favorites (the list the chart star adds to). Read one with degenview_get_watchlist.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_watchlists(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<WatchlistsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::WatchlistsRead,
                McpCommand::ListWatchlists,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Watchlists(lists)) => Ok(Json(lists)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_watchlist",
        description = "Read a watchlist's entries in order: markets (with `source` and `symbol` to pass to other tools), the section each sits in, and the section dividers themselves. Pages of up to 500 entries (`offset`, `limit`). With `include_quotes` each market also carries its price, change and volume (see degenview_get_quotes for the fields), limited to 50 markets per page; this needs the markets.read permission as well.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_watchlist(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetWatchlistParams>,
    ) -> Result<Json<WatchlistDetail>, CallToolResult> {
        let client = client_of(&parts)?;
        let watchlist_id = parse_watchlist_id(&params.watchlist_id)?;
        let limit = params
            .limit
            .map_or(DEFAULT_ENTRY_PAGE, |limit| limit as usize);
        if !(1..=MAX_ENTRY_PAGE).contains(&limit) {
            return Err(invalid(format!(
                "limit must be between 1 and {MAX_ENTRY_PAGE}"
            )));
        }
        let wait = quote_wait(params.wait_seconds)?;
        let with_quotes = params.include_quotes.unwrap_or(false);
        if with_quotes && !self.shared.allows(&client.id, Permission::MarketsRead) {
            return Err(failure(&McpError::PermissionDenied(format!(
                "This credential lacks the {} permission that include_quotes needs",
                Permission::MarketsRead.id()
            ))));
        }
        let page = match self
            .bridge
            .call(
                &client.id,
                Permission::WatchlistsRead,
                McpCommand::GetWatchlist {
                    watchlist_id,
                    offset: params.offset.unwrap_or(0) as usize,
                    limit,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Watchlist(page)) => *page,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        let mut detail = page.detail;
        if with_quotes && !page.markets.is_empty() {
            if page.markets.len() > MAX_QUOTE_MARKETS {
                return Err(invalid(format!(
                    "this page has {} markets; with include_quotes ask for a smaller limit so it holds at most {MAX_QUOTE_MARKETS}",
                    page.markets.len()
                )));
            }
            let (quotes, _) = self.read_quotes(&client, page.markets, wait).await?;
            let mut quotes = quotes.into_iter();
            for entry in detail.entries.iter_mut().filter(|e| e.kind == "market") {
                entry.quote = quotes.next();
            }
        }
        Ok(Json(detail))
    }

    #[tool(
        name = "degenview_create_watchlist",
        description = "Create a watchlist, optionally with markets (`source` + `symbol`, as degenview_search_symbols returns them). Fails if the name is empty or too long, or there are already 200 lists. The list syncs through iCloud like any other.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_watchlist(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<CreateWatchlistParams>,
    ) -> Result<Json<WatchlistChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let markets = resolve_markets(params.markets.as_deref().unwrap_or(&[]))?;
        self.change_watchlist(
            &client,
            Permission::WatchlistsWrite,
            McpCommand::CreateWatchlist {
                name: params.name,
                markets,
            },
        )
        .await
    }

    #[tool(
        name = "degenview_update_watchlist",
        description = "Rename a watchlist and/or add and remove markets (`source` + `symbol`), all in one change: if anything is refused (a bad name, a full list) nothing changes. Removals run first. A market already in the list, or one to remove that is not, is skipped and reported (`already_present`, `not_present`), so a retry is harmless. New markets go at the end of the list.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_watchlist(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<UpdateWatchlistParams>,
    ) -> Result<Json<WatchlistChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let watchlist_id = parse_watchlist_id(&params.watchlist_id)?;
        let add = resolve_markets(params.add.as_deref().unwrap_or(&[]))?;
        let remove: Vec<InstrumentId> = resolve_markets(params.remove.as_deref().unwrap_or(&[]))?
            .into_iter()
            .map(|item| item.instrument)
            .collect();
        if params.rename.is_none() && add.is_empty() && remove.is_empty() {
            return Err(invalid("give rename, add or remove"));
        }
        let removing: HashSet<String> = remove.iter().map(InstrumentId::key).collect();
        if let Some(both) = add
            .iter()
            .find(|item| removing.contains(&item.instrument.key()))
        {
            return Err(invalid(format!(
                "{} is in both add and remove",
                both.instrument.key()
            )));
        }
        self.change_watchlist(
            &client,
            Permission::WatchlistsWrite,
            McpCommand::UpdateWatchlist {
                watchlist_id,
                rename: params.rename,
                add,
                remove,
            },
        )
        .await
    }

    #[tool(
        name = "degenview_delete_watchlist",
        description = "Permanently delete a watchlist and its markets and sections. This cannot be undone from here. Favorites (is_favorites) cannot be deleted.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn delete_watchlist(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<WatchlistIdParams>,
    ) -> Result<Json<WatchlistChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let watchlist_id = parse_watchlist_id(&params.watchlist_id)?;
        self.change_watchlist(
            &client,
            Permission::WatchlistsDelete,
            McpCommand::DeleteWatchlist { watchlist_id },
        )
        .await
    }
}

impl DegenViewMcp {
    async fn change_watchlist(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        command: McpCommand,
    ) -> Result<Json<WatchlistChangeResult>, CallToolResult> {
        match self
            .bridge
            .call(&client.id, permission, command, REQUEST_TIMEOUT)
            .await
        {
            Ok(McpReply::WatchlistChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    /// Holds a demand on the quote feed for `markets` and waits up to `wait` for them to settle.
    /// The demand ends when this returns or is cancelled. One entry per market, in order.
    pub(super) async fn wait_for_quotes(
        &self,
        markets: &[InstrumentId],
        wait: Duration,
    ) -> Result<Vec<Option<WatchlistQuote>>, CallToolResult> {
        let Some(access) = self.shared.quotes() else {
            return Err(failure(&McpError::AppUnavailable(
                "Quotes are unavailable right now".into(),
            )));
        };
        let Some(_permit) = self.shared.try_quote_job() else {
            return Err(failure(&McpError::ResourceLimit(
                "Too many quote requests are waiting; try again shortly".into(),
            )));
        };
        let lease = self.shared.lease_quotes(access, markets.to_vec());
        let deadline = Instant::now() + wait;
        loop {
            let held: Vec<Option<WatchlistQuote>> = markets
                .iter()
                .map(|market| lease.book().get(market))
                .collect();
            let pending = held
                .iter()
                .filter(|quote| !is_settled(quote.as_ref()))
                .count();
            if pending == 0 || Instant::now() >= deadline {
                return Ok(held);
            }
            tokio::time::sleep(QUOTE_POLL).await;
        }
    }

    /// Prices `markets`, in order, waiting up to `wait` for any not known yet. Returns the quotes
    /// and how many are still unpriced.
    async fn read_quotes(
        &self,
        client: &ClientIdentity,
        markets: Vec<InstrumentId>,
        wait: Duration,
    ) -> Result<(Vec<QuoteInfo>, usize), CallToolResult> {
        let held = self.wait_for_quotes(&markets, wait).await?;
        if !self.shared.allows(&client.id, Permission::MarketsRead) {
            return Err(failure(&McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
            )));
        }
        let pending = held
            .iter()
            .filter(|quote| !is_settled(quote.as_ref()))
            .count();
        let quotes = markets
            .iter()
            .zip(&held)
            .map(|(market, quote)| quote_info(market, quote.as_ref()))
            .collect();
        Ok((quotes, pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist_quote::SourceQuote;

    fn market(source: McpSource, symbol: &str) -> MarketRef {
        MarketRef {
            source,
            symbol: symbol.into(),
        }
    }

    #[test]
    fn spellings_of_one_market_become_one_row() {
        let items = resolve_markets(&[
            market(McpSource::Binance, "BTCUSDT"),
            market(McpSource::Binance, "btcusdt"),
            market(McpSource::Coinbase, "eth-usd"),
        ])
        .unwrap();
        let keys: Vec<_> = items.iter().map(|item| item.instrument.key()).collect();
        assert_eq!(keys, ["BINANCE:BTCUSDT", "COINBASE:ETH-USD"]);
    }

    #[test]
    fn a_symbol_not_shaped_like_a_market_is_refused_by_name() {
        let error = resolve_markets(&[market(McpSource::Coinbase, "BTCUSD")]).unwrap_err();
        let text = format!("{error:?}");
        assert!(text.contains("coinbase BTCUSD"), "{text}");
        assert!(resolve_markets(&[market(McpSource::Binance, "A|B")]).is_err());
        let many: Vec<_> = (0..=MAX_EDIT_MARKETS)
            .map(|n| market(McpSource::Alpaca, &format!("S{n}")))
            .collect();
        assert!(resolve_markets(&many).is_err());
    }

    #[test]
    fn only_a_price_or_a_reason_ends_the_wait() {
        assert!(!is_settled(None));
        assert!(!is_settled(Some(&WatchlistQuote::default())));
        let source = SourceQuote {
            price: 1.0,
            ..SourceQuote::default()
        };
        let live = WatchlistQuote::from_source(DataSource::Binance, &source, 0);
        // `from_source` stamps freshness against its own receive time, so it reads live.
        assert!(is_settled(Some(&live)));
        assert!(is_settled(Some(
            &live.with_freshness(Freshness::MarketClosed)
        )));
        assert!(!is_settled(Some(&live.with_freshness(Freshness::Stale))));
        assert!(is_settled(Some(&WatchlistQuote::unsupported(None))));
        let refused = WatchlistQuote {
            note: Some("Unknown symbol".into()),
            ..WatchlistQuote::default()
        };
        assert!(is_settled(Some(&refused)));
    }

    #[test]
    fn query_and_wait_arguments_are_checked() {
        assert!(clean_query("  btc ").is_ok_and(|q| q == "btc"));
        assert!(clean_query("   ").is_err());
        assert!(clean_query("a\nb").is_err());
        assert!(clean_query(&"x".repeat(65)).is_err());
        assert_eq!(quote_wait(None).unwrap(), DEFAULT_QUOTE_WAIT);
        assert_eq!(quote_wait(Some(0)).unwrap(), Duration::ZERO);
        assert!(quote_wait(Some(16)).is_err());
        assert!(parse_watchlist_id("12").is_ok());
        assert!(parse_watchlist_id("x").is_err());
    }
}
