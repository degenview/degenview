//! What the market and watchlist tools return, and the pure parts of building it: search hits,
//! quotes and watchlists as typed values. Free of egui and HTTP; the app-thread projections that
//! need the store live in `host::mcp`.

pub mod tools;

use crate::mcp::chart::data::iso;
use crate::model::watchlist::quote::{ChangeBasis, Freshness, VolumeKind, WatchlistQuote};
use crate::model::watchlist::{Watchlist, WatchlistEntry, WatchlistInstrument};
use crate::model::{DataSource, InstrumentId};
use crate::service::SearchResult;
use schemars::JsonSchema;
use serde::Serialize;

/// Markets one `degenview_get_quotes` call may ask about.
pub const MAX_QUOTE_MARKETS: usize = 50;
/// Entries one `degenview_get_watchlist` page holds at most.
pub const MAX_ENTRY_PAGE: usize = 500;
/// Markets one watchlist edit may add or remove.
pub const MAX_EDIT_MARKETS: usize = 100;

/// The provider's name as every MCP tool spells it.
pub fn source_id(source: DataSource) -> &'static str {
    match source {
        DataSource::Binance => "binance",
        DataSource::Coinbase => "coinbase",
        DataSource::CoinGecko => "coingecko",
        DataSource::DexScreener => "dexscreener",
        DataSource::Alpaca => "alpaca",
        DataSource::Polymarket => "polymarket",
        DataSource::Kalshi => "kalshi",
        DataSource::CoinMarketCap => "coinmarketcap",
    }
}

// MARK: Search

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SymbolHit {
    /// The provider, as `degenview_add_chart` and the watchlist tools take it.
    pub source: String,
    /// The provider's own symbol: pass it back, with `source`, to any tool that takes a market.
    pub symbol: String,
    /// How the app labels it: `BTC/USDT`, `AAPL — Apple Inc.`.
    pub display: String,
    /// The source-qualified identity (`BINANCE:BTCUSDT`), the same one charts and watchlists use.
    pub instrument: String,
    /// The price the provider returned with the hit, when it did (few do; use
    /// `degenview_get_quotes` for a price).
    pub price: Option<f64>,
}

pub fn symbol_hit(result: &SearchResult) -> SymbolHit {
    let instrument = InstrumentId::new(result.source, result.full_symbol.clone());
    SymbolHit {
        source: source_id(result.source).to_owned(),
        symbol: result.full_symbol.clone(),
        display: result.symbol.clone(),
        instrument: instrument.key(),
        price: result.price.filter(|price| price.is_finite()),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SearchSourceError {
    pub source: String,
    /// Why this provider gave nothing ("Add your Alpaca API key in Settings"); the others still
    /// answered.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SymbolSearchResult {
    pub query: String,
    /// Best matches first within each provider; providers in the order asked.
    pub results: Vec<SymbolHit>,
    /// More matches existed than `limit` allowed.
    pub truncated: bool,
    pub source_errors: Vec<SearchSourceError>,
}

// MARK: Candles by symbol

/// Candles for a market that need not be on any chart.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct MarketCandlesResult {
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    pub timeframe: String,
    /// The meaning of each position in a `bars` row.
    pub columns: Vec<String>,
    /// Oldest first. Times are ISO-8601 UTC.
    pub bars: Vec<crate::mcp::types::CandleRow>,
    pub count: usize,
    pub range: Option<crate::mcp::types::CandleRange>,
    pub summary: Option<crate::mcp::types::CandleSummary>,
    /// Whether the newest bar is still forming (its period has not ended).
    pub last_bar_forming: bool,
    pub notes: Vec<String>,
}

// MARK: Quotes

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct QuoteInfo {
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    /// The last price, in the market's own quote currency. Null when none is known.
    pub last: Option<f64>,
    /// The price change over `change_basis`, and as a percentage.
    pub change: Option<f64>,
    pub change_percent: Option<f64>,
    /// `rolling_24h`, `previous_close` or `window`.
    pub change_basis: Option<String>,
    pub volume: Option<f64>,
    /// What `volume` counts: `quote_currency`, `shares` or `base`.
    pub volume_unit: Option<String>,
    /// When the provider priced it (or, failing that, when DegenView received it), ISO-8601 UTC.
    pub as_of: Option<String>,
    /// `live`, `stale`, `market_closed` (the last price is the close), `unavailable` (no answer
    /// yet or the provider refused the symbol) or `unsupported`.
    pub status: String,
    /// Why there is no price, when the provider said.
    pub note: Option<String>,
}

fn change_basis(basis: ChangeBasis) -> &'static str {
    match basis {
        ChangeBasis::Rolling24h => "rolling_24h",
        ChangeBasis::PreviousClose => "previous_close",
        ChangeBasis::Window => "window",
    }
}

fn volume_unit(kind: VolumeKind) -> &'static str {
    match kind {
        VolumeKind::QuoteCurrency => "quote_currency",
        VolumeKind::Shares => "shares",
        VolumeKind::Base => "base",
    }
}

fn status(freshness: Freshness) -> &'static str {
    match freshness {
        Freshness::Live => "live",
        Freshness::Stale => "stale",
        Freshness::MarketClosed => "market_closed",
        Freshness::Unavailable => "unavailable",
        Freshness::Unsupported => "unsupported",
    }
}

/// A quote for `instrument`; with no quote yet, a row that says so rather than an invented zero.
pub fn quote_info(instrument: &InstrumentId, quote: Option<&WatchlistQuote>) -> QuoteInfo {
    let finite = |value: Option<f64>| value.filter(|value| value.is_finite());
    let missing = WatchlistQuote::default();
    let quote = quote.unwrap_or(&missing);
    QuoteInfo {
        instrument: instrument.key(),
        source: source_id(instrument.source).to_owned(),
        symbol: instrument.symbol.clone(),
        last: finite(quote.last),
        change: finite(quote.change),
        change_percent: finite(quote.change_percent),
        change_basis: quote
            .change_basis
            .map(|basis| change_basis(basis).to_owned()),
        volume: finite(quote.volume),
        volume_unit: quote.volume_kind.map(|kind| volume_unit(kind).to_owned()),
        as_of: quote.timestamp_ms.map(iso),
        status: status(quote.freshness).to_owned(),
        note: quote.note.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct QuotesResult {
    pub quotes: Vec<QuoteInfo>,
    /// Markets whose price may still arrive: nothing known yet when the wait ended. They are in
    /// `quotes` with status `unavailable`; ask again to wait longer for a slow provider. A market
    /// the provider refused is not pending: its `note` says why.
    pub pending: usize,
}

// MARK: Watchlists

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WatchlistSummary {
    pub watchlist_id: String,
    pub name: String,
    /// The list the chart star adds to. It can be renamed but not deleted.
    pub is_favorites: bool,
    /// Markets (not sections) in the list.
    pub market_count: usize,
    pub section_count: usize,
    pub updated_at: Option<String>,
}

pub fn watchlist_summary(list: &Watchlist) -> WatchlistSummary {
    WatchlistSummary {
        watchlist_id: list.id.to_string(),
        name: list.name.clone(),
        is_favorites: list.is_favorites,
        market_count: list.instruments().count(),
        section_count: list.sections().count(),
        updated_at: (list.updated_at_ms > 0).then(|| iso(list.updated_at_ms)),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WatchlistsResult {
    pub watchlists: Vec<WatchlistSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WatchlistEntryInfo {
    /// `market`, `section` (a labelled divider; the markets after it belong to it) or
    /// `portfolio_card` (a favourite card, which has no market).
    pub kind: String,
    pub entry_id: String,
    /// The market's name (`Bitcoin`, `BTC/USDT`), or the section's title.
    pub title: String,
    /// The short label under it (`BTC`), when there is one.
    pub label: Option<String>,
    pub source: Option<String>,
    pub symbol: Option<String>,
    pub instrument: Option<String>,
    /// The section a market sits in.
    pub section: Option<String>,
    /// Present when quotes were asked for.
    pub quote: Option<QuoteInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WatchlistDetail {
    #[serde(flatten)]
    pub summary: WatchlistSummary,
    /// Entries in list order, from `offset`.
    pub entries: Vec<WatchlistEntryInfo>,
    pub offset: usize,
    /// All entries in the list, sections included.
    pub total_entries: usize,
    pub truncated: bool,
}

fn market_entry(
    item: &WatchlistInstrument,
    section: Option<&str>,
    quote: Option<QuoteInfo>,
) -> WatchlistEntryInfo {
    let instrument = &item.instrument;
    WatchlistEntryInfo {
        kind: "market".into(),
        entry_id: item.id.to_string(),
        title: item
            .display_name
            .clone()
            .unwrap_or_else(|| item.name.clone()),
        label: (!item.label.is_empty()).then(|| item.label.clone()),
        source: Some(source_id(instrument.source).to_owned()),
        symbol: Some(instrument.symbol.clone()),
        instrument: Some(instrument.key()),
        section: section.map(str::to_owned),
        quote,
    }
}

/// Looks up a market's quote.
pub type QuoteLookup<'a> = dyn Fn(&InstrumentId) -> Option<WatchlistQuote> + 'a;

/// One page of a list. `quote_of` is asked for each market on the page only when `quotes` is set.
pub fn watchlist_detail(
    list: &Watchlist,
    offset: usize,
    limit: usize,
    quote_of: Option<&QuoteLookup<'_>>,
) -> WatchlistDetail {
    let mut section: Option<&str> = None;
    let mut entries = Vec::new();
    for (index, entry) in list.entries.iter().enumerate() {
        let info = match entry {
            WatchlistEntry::Section(found) => {
                section = Some(found.title.as_str());
                WatchlistEntryInfo {
                    kind: "section".into(),
                    entry_id: found.id.to_string(),
                    title: found.title.clone(),
                    label: None,
                    source: None,
                    symbol: None,
                    instrument: None,
                    section: None,
                    quote: None,
                }
            }
            WatchlistEntry::Instrument(item) if item.card.is_some() => WatchlistEntryInfo {
                kind: "portfolio_card".into(),
                entry_id: item.id.to_string(),
                title: item.name.clone(),
                label: (!item.label.is_empty()).then(|| item.label.clone()),
                source: None,
                symbol: None,
                instrument: None,
                section: section.map(str::to_owned),
                quote: None,
            },
            WatchlistEntry::Instrument(item) => {
                let quote = quote_of.map(|quote_of| {
                    quote_info(&item.instrument, quote_of(&item.instrument).as_ref())
                });
                market_entry(item, section, quote)
            }
        };
        if index >= offset && entries.len() < limit {
            entries.push(info);
        }
    }
    let total = list.entries.len();
    WatchlistDetail {
        summary: watchlist_summary(list),
        truncated: offset + entries.len() < total,
        entries,
        offset,
        total_entries: total,
    }
}

/// A page of a list and the markets on it, in order, so the tool can ask for their quotes.
#[derive(Debug, Clone, PartialEq)]
pub struct WatchlistPage {
    pub detail: WatchlistDetail,
    pub markets: Vec<InstrumentId>,
}

/// The markets on one page of a list that a quote can be asked for, in list order.
pub fn page_markets(list: &Watchlist, offset: usize, limit: usize) -> Vec<InstrumentId> {
    list.entries
        .iter()
        .skip(offset)
        .take(limit)
        .filter_map(WatchlistEntry::instrument)
        .filter(|item| item.card.is_none())
        .map(|item| item.instrument.clone())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WatchlistChangeResult {
    /// `created`, `updated`, `unchanged` or `deleted`.
    pub status: String,
    /// The list as it is now; null after a delete.
    pub watchlist: Option<WatchlistSummary>,
    /// Markets added, by `instrument`.
    pub added: Vec<String>,
    /// Markets that were already in the list.
    pub already_present: Vec<String>,
    /// Markets removed.
    pub removed: Vec<String>,
    /// Markets asked to be removed or moved that were not in the list.
    pub not_present: Vec<String>,
    /// Set when the list was renamed.
    pub renamed_to: Option<String>,
    /// Markets moved to another section (or the top), by `instrument`.
    pub moved: Vec<String>,
    /// Titles of the sections made.
    pub sections_added: Vec<String>,
    /// New titles of the sections renamed.
    pub sections_renamed: Vec<String>,
    /// The sections deleted, as they were named in the request. Their markets stay in the list.
    pub sections_deleted: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::WatchlistSection;

    fn hit(source: DataSource, display: &str, full: &str) -> SearchResult {
        SearchResult {
            symbol: display.into(),
            full_symbol: full.into(),
            source,
            price: Some(f64::NAN),
            event: None,
            base: None,
            quote: None,
        }
    }

    #[test]
    fn a_search_hit_carries_the_identity_every_other_tool_takes() {
        let hit = symbol_hit(&hit(DataSource::Binance, "BTC/USDT", "BTCUSDT"));
        assert_eq!(hit.source, "binance");
        assert_eq!(hit.symbol, "BTCUSDT");
        assert_eq!(hit.instrument, "BINANCE:BTCUSDT");
        assert_eq!(hit.price, None, "a non-finite price is not reported");
    }

    #[test]
    fn a_missing_quote_reads_unavailable_not_zero() {
        let id = InstrumentId::new(DataSource::Coinbase, "BTC-USD");
        let info = quote_info(&id, None);
        assert_eq!(info.status, "unavailable");
        assert_eq!(info.last, None);
        assert_eq!(info.instrument, "COINBASE:BTC-USD");
    }

    #[test]
    fn a_quote_reports_its_basis_unit_and_time() {
        let id = InstrumentId::new(DataSource::Alpaca, "AAPL");
        let quote = WatchlistQuote {
            last: Some(190.5),
            change: Some(1.5),
            change_percent: Some(0.79),
            change_basis: Some(ChangeBasis::PreviousClose),
            volume: Some(1_000.0),
            volume_kind: Some(VolumeKind::Shares),
            timestamp_ms: Some(1_760_918_400_000),
            freshness: Freshness::MarketClosed,
            note: None,
        };
        let info = quote_info(&id, Some(&quote));
        assert_eq!(info.status, "market_closed");
        assert_eq!(info.change_basis.as_deref(), Some("previous_close"));
        assert_eq!(info.volume_unit.as_deref(), Some("shares"));
        assert_eq!(info.as_of.as_deref(), Some("2025-10-20T00:00:00Z"));
    }

    fn list() -> Watchlist {
        let mut list = Watchlist::new("Majors");
        list.add(
            WatchlistInstrument::from_market(DataSource::Binance, "BTCUSDT", "BTCUSDT"),
            None,
        )
        .unwrap();
        list.entries
            .push(WatchlistEntry::Section(WatchlistSection::new("Alts")));
        list.add(
            WatchlistInstrument::from_market(DataSource::Binance, "ETHUSDT", "ETHUSDT"),
            None,
        )
        .unwrap();
        list
    }

    #[test]
    fn a_page_names_the_section_each_market_sits_in() {
        let detail = watchlist_detail(&list(), 0, 10, None);
        assert_eq!(detail.summary.market_count, 2);
        assert_eq!(detail.summary.section_count, 1);
        let kinds: Vec<_> = detail.entries.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["market", "section", "market"]);
        assert_eq!(detail.entries[0].section, None);
        assert_eq!(detail.entries[2].section.as_deref(), Some("Alts"));
        assert!(detail.entries.iter().all(|entry| entry.quote.is_none()));
        assert!(!detail.truncated);
    }

    #[test]
    fn paging_keeps_the_section_a_later_page_starts_inside() {
        let detail = watchlist_detail(&list(), 2, 1, None);
        assert_eq!(detail.entries.len(), 1);
        assert_eq!(detail.entries[0].section.as_deref(), Some("Alts"));
        let first = watchlist_detail(&list(), 0, 1, None);
        assert!(first.truncated);
        assert_eq!(first.total_entries, 3);
    }

    #[test]
    fn quotes_are_asked_only_for_markets_on_the_page() {
        let asked = std::cell::RefCell::new(Vec::new());
        let quote_of = |id: &InstrumentId| {
            asked.borrow_mut().push(id.key());
            None
        };
        let detail = watchlist_detail(&list(), 0, 10, Some(&quote_of));
        assert_eq!(*asked.borrow(), ["BINANCE:BTCUSDT", "BINANCE:ETHUSDT"]);
        assert_eq!(
            detail.entries[0].quote.as_ref().map(|q| q.status.as_str()),
            Some("unavailable")
        );
        assert_eq!(page_markets(&list(), 1, 5).len(), 1);
    }
}
