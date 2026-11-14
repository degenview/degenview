//! Portfolio CSV: the app's own transaction export and import, the holdings and history exports,
//! and the CoinMarketCap transaction export reader. Pure text in, preview out — nothing here
//! touches the ledger; the import sheets show the preview and the host commits it.
//!
//! Differences from the Swift service, all in the user's favour: records may contain quoted line
//! breaks (a note with a newline round-trips), and timestamps accept fractional seconds (the
//! export writes them, so a file exported here imports here). Error line numbers still count
//! non-empty records, header = line 1, as before.

use crate::format::{civil_from_days, days_from_civil};
use crate::model::DataSource;
use crate::model::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioHolding, PortfolioSnapshot,
    PortfolioSource, PortfolioTransaction, PortfolioTransactionType,
};
use std::collections::{HashMap, HashSet};

pub const HEADER: &str = "portfolio,asset_id,symbol,name,source,type,quantity,price,currency,fee,fee_currency,date,notes,external_id";
pub const HOLDINGS_HEADER: &str = "portfolio,asset_id,symbol,quantity,average_cost,current_price,current_value,realized_pnl,unrealized_pnl,as_of";
pub const HISTORY_HEADER: &str =
    "portfolio_id,timestamp,value,net_contributions,realized_pnl,unrealized_pnl,is_complete";

const REQUIRED_COLUMNS: [&str; 8] = [
    "portfolio",
    "asset_id",
    "symbol",
    "source",
    "type",
    "quantity",
    "currency",
    "date",
];

/// What an import is about to write, with the problems found. Errors block it; warnings don't.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CsvPreview {
    pub transactions: Vec<PortfolioTransaction>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl CsvPreview {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    fn failed(error: impl Into<String>) -> Self {
        Self {
            errors: vec![error.into()],
            ..Self::default()
        }
    }
}

/// Where new transactions get their identity and timestamps.
pub struct Mint<'a> {
    pub now_ms: i64,
    pub next_id: &'a mut dyn FnMut() -> u64,
}

// MARK: Names

/// The name a transaction type has in files and on screen.
pub fn type_name(kind: PortfolioTransactionType) -> &'static str {
    use PortfolioTransactionType::*;
    match kind {
        Buy => "Buy",
        Sell => "Sell",
        TransferIn => "Transfer In",
        TransferOut => "Transfer Out",
        Reward => "Reward",
        StakingReward => "Staking Reward",
        Airdrop => "Airdrop",
        Mining => "Mining",
        Interest => "Interest",
        Fee => "Fee",
        Adjustment => "Adjustment",
    }
}

pub const ALL_TYPES: [PortfolioTransactionType; 11] = [
    PortfolioTransactionType::Buy,
    PortfolioTransactionType::Sell,
    PortfolioTransactionType::TransferIn,
    PortfolioTransactionType::TransferOut,
    PortfolioTransactionType::Reward,
    PortfolioTransactionType::StakingReward,
    PortfolioTransactionType::Airdrop,
    PortfolioTransactionType::Mining,
    PortfolioTransactionType::Interest,
    PortfolioTransactionType::Fee,
    PortfolioTransactionType::Adjustment,
];

pub fn parse_type(text: &str) -> Option<PortfolioTransactionType> {
    ALL_TYPES.into_iter().find(|kind| type_name(*kind) == text)
}

const ALL_SOURCES: [DataSource; 8] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
    DataSource::Alpaca,
    DataSource::Polymarket,
    DataSource::Kalshi,
    DataSource::CoinMarketCap,
];

pub fn parse_source(text: &str) -> Option<DataSource> {
    ALL_SOURCES
        .into_iter()
        .find(|source| source.display_name() == text)
}

pub fn parse_currency(text: &str) -> Option<PortfolioCurrency> {
    use crate::model::portfolio::format::PORTFOLIO_CURRENCIES;
    PORTFOLIO_CURRENCIES
        .into_iter()
        .find(|currency| currency.code() == text)
}

// MARK: Records

/// Splits CSV text into records of fields. Quotes may span lines; `""` is a literal quote;
/// blank lines are skipped. CRLF and LF both end a record.
pub fn parse_records(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let finish = |records: &mut Vec<Vec<String>>, fields: &mut Vec<String>, field: &mut String| {
        fields.push(std::mem::take(field));
        let record = std::mem::take(fields);
        // A line with nothing on it is not a record.
        if !(record.len() == 1 && record[0].is_empty()) {
            records.push(record);
        }
    };
    while let Some(character) = chars.next() {
        match character {
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            }
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            '\r' if !quoted => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                finish(&mut records, &mut fields, &mut field);
            }
            '\n' if !quoted => finish(&mut records, &mut fields, &mut field),
            other => field.push(other),
        }
    }
    if !field.is_empty() || !fields.is_empty() {
        finish(&mut records, &mut fields, &mut field);
    }
    records
}

/// Quotes a field only when it has to be.
pub fn escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn row(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| escape(field))
        .collect::<Vec<_>>()
        .join(",")
}

fn number(value: f64) -> String {
    format!("{value}")
}

// MARK: Dates

/// "2024-03-05T14:30:00Z", "…:00.250Z" or "…:00+02:00" → epoch milliseconds.
pub fn parse_iso8601(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', 't'])?;
    let mut parts = date.split('-');
    let (year, month, day) = (
        parts.next()?.parse::<i64>().ok()?,
        parts.next()?.parse::<i64>().ok()?,
        parts.next()?.parse::<i64>().ok()?,
    );
    if parts.next().is_some() || date.len() != 10 {
        return None;
    }
    let (clock, offset_secs) = split_offset(rest)?;
    let (clock, fraction) = match clock.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (clock, None),
    };
    let mut hms = clock.split(':');
    let (hour, minute, second) = (
        hms.next()?.parse::<i64>().ok()?,
        hms.next()?.parse::<i64>().ok()?,
        hms.next()?.parse::<i64>().ok()?,
    );
    if hms.next().is_some() || clock.len() != 8 {
        return None;
    }
    let millis = match fraction {
        None => 0,
        Some(digits) if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => {
            let padded = format!("{digits:0<3}");
            padded[..3].parse::<i64>().ok()?
        }
        Some(_) => return None,
    };
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset_secs;
    Some(seconds * 1_000 + millis)
}

/// Splits the clock from its `Z` / `±hh:mm` / `±hhmm` suffix; the offset is in seconds.
fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
        return Some((clock, 0));
    }
    let index = rest.rfind(['+', '-'])?;
    let (clock, suffix) = rest.split_at(index);
    let sign = if suffix.starts_with('-') { -1 } else { 1 };
    let digits: String = suffix[1..].chars().filter(|c| *c != ':').collect();
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = digits[2..].parse().ok()?;
    Some((clock, sign * (hours * 3_600 + minutes * 60)))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Epoch milliseconds → "2024-03-05T14:30:00.250Z".
pub fn format_iso8601(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let millis = epoch_ms.rem_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn format_iso8601_seconds(epoch_ms: i64) -> String {
    let text = format_iso8601(epoch_ms);
    format!("{}Z", &text[..19])
}

// MARK: Export

pub fn export_transactions(
    transactions: &[PortfolioTransaction],
    portfolios: &[Portfolio],
) -> String {
    let mut sorted: Vec<&PortfolioTransaction> = transactions.iter().collect();
    sorted.sort_by_key(|transaction| transaction.timestamp);
    let mut lines = vec![HEADER.to_owned()];
    for transaction in sorted {
        let portfolio = portfolios
            .iter()
            .find(|portfolio| portfolio.id == transaction.portfolio_id)
            .map_or("", |portfolio| portfolio.name.as_str());
        lines.push(row(&[
            portfolio.to_owned(),
            transaction.asset.key.clone(),
            transaction.asset.symbol.clone(),
            transaction.asset.name.clone(),
            transaction.asset.source.display_name().to_owned(),
            type_name(transaction.kind).to_owned(),
            number(transaction.quantity),
            transaction.price.map(number).unwrap_or_default(),
            transaction.price_currency.code().to_owned(),
            number(transaction.fee),
            transaction.fee_currency.code().to_owned(),
            format_iso8601(transaction.timestamp),
            transaction.notes.clone(),
            transaction
                .external_transaction_id
                .clone()
                .unwrap_or_default(),
        ]));
    }
    lines.join("\n")
}

pub fn export_holdings(
    holdings: &[PortfolioHolding],
    portfolio_name: &str,
    timestamp_ms: i64,
) -> String {
    let stamp = format_iso8601_seconds(timestamp_ms);
    let optional = |value: Option<f64>| value.map(number).unwrap_or_default();
    let mut lines = vec![HOLDINGS_HEADER.to_owned()];
    for holding in holdings {
        lines.push(row(&[
            portfolio_name.to_owned(),
            holding.asset.key.clone(),
            holding.asset.symbol.clone(),
            number(holding.quantity),
            number(holding.average_cost),
            optional(holding.current_price),
            optional(holding.current_value),
            number(holding.realized_pnl),
            optional(holding.unrealized_pnl),
            stamp.clone(),
        ]));
    }
    lines.join("\n")
}

pub fn export_history(snapshots: &[PortfolioSnapshot]) -> String {
    let mut sorted: Vec<&PortfolioSnapshot> = snapshots.iter().collect();
    sorted.sort_by_key(|snapshot| snapshot.timestamp);
    let mut lines = vec![HISTORY_HEADER.to_owned()];
    for snapshot in sorted {
        lines.push(
            [
                snapshot.portfolio_id.to_string(),
                format_iso8601_seconds(snapshot.timestamp),
                number(snapshot.value),
                number(snapshot.net_contributions),
                number(snapshot.realized_pnl),
                number(snapshot.unrealized_pnl),
                snapshot.is_complete.to_string(),
            ]
            .join(","),
        );
    }
    lines.join("\n")
}

// MARK: Import

fn positive(text: &str) -> Option<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn decimal(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Reads the app's own transaction export. Every row is checked; a bad row is an error and is
/// left out, so the preview can show everything wrong at once.
pub fn preview(csv: &str, portfolios: &[Portfolio], mint: &mut Mint) -> CsvPreview {
    let records = parse_records(csv);
    let Some(header) = records.first() else {
        return CsvPreview::failed("The CSV is empty.");
    };
    let mut indexes: HashMap<String, usize> = HashMap::new();
    for (index, name) in header.iter().enumerate() {
        indexes.entry(name.to_lowercase()).or_insert(index);
    }
    let missing: Vec<&str> = REQUIRED_COLUMNS
        .into_iter()
        .filter(|name| !indexes.contains_key(*name))
        .collect();
    if !missing.is_empty() {
        return CsvPreview::failed(format!("Missing columns: {}.", missing.join(", ")));
    }
    let mut result = CsvPreview::default();
    for (offset, record) in records.iter().skip(1).enumerate() {
        let line = offset + 2;
        let value = |name: &str| {
            indexes
                .get(name)
                .and_then(|index| record.get(*index))
                .map_or("", String::as_str)
        };
        let wanted = value("portfolio").trim().to_lowercase();
        let mut named = portfolios
            .iter()
            .filter(|portfolio| portfolio.name.trim().to_lowercase() == wanted);
        let Some(portfolio) = named.next() else {
            result
                .errors
                .push(format!("Line {line}: unknown portfolio."));
            continue;
        };
        if named.next().is_some() {
            // Which one the rows belong to cannot be told from the name.
            result.errors.push(format!(
                "Line {line}: more than one portfolio is named \"{}\".",
                value("portfolio")
            ));
            continue;
        }
        let fields = (
            parse_source(value("source")),
            parse_type(value("type")),
            positive(value("quantity")),
            parse_currency(value("currency")),
            parse_iso8601(value("date")),
        );
        let (Some(source), Some(kind), Some(quantity), Some(currency), Some(timestamp)) = fields
        else {
            result.errors.push(format!(
                "Line {line}: invalid source, type, quantity, currency, or ISO-8601 date."
            ));
            continue;
        };
        if value("asset_id").trim().is_empty() {
            result
                .errors
                .push(format!("Line {line}: missing asset_id."));
            continue;
        }
        let price = decimal(value("price")).filter(|price| *price >= 0.0);
        if price.is_none() && !value("price").trim().is_empty() {
            result.errors.push(format!(
                "Line {line}: the price is not a number of zero or more."
            ));
            continue;
        }
        let fee = decimal(value("fee")).filter(|fee| *fee >= 0.0);
        if fee.is_none() && !value("fee").trim().is_empty() {
            result.errors.push(format!(
                "Line {line}: the fee is not a number of zero or more."
            ));
            continue;
        }
        let fee_currency = parse_currency(value("fee_currency")).unwrap_or(currency);
        let name = match value("name") {
            "" => value("symbol"),
            name => name,
        };
        let external_id = Some(value("external_id").to_owned()).filter(|id| !id.is_empty());
        let transaction = PortfolioTransaction {
            id: (mint.next_id)(),
            portfolio_id: portfolio.id,
            asset: PortfolioAsset {
                key: value("asset_id").to_owned(),
                symbol: value("symbol").to_owned(),
                name: name.to_owned(),
                source,
                quote_currency: currency,
                metadata: HashMap::new(),
            },
            kind,
            quantity,
            price,
            price_currency: currency,
            fee: fee.unwrap_or(0.0),
            fee_currency,
            timestamp,
            notes: value("notes").to_owned(),
            source: PortfolioSource::Csv,
            external_transaction_id: external_id,
            created_at: mint.now_ms,
            updated_at: mint.now_ms,
        };
        if transaction.external_transaction_id.is_none() {
            result.warnings.push(format!(
                "Line {line}: no external_id; repeat imports cannot be deduplicated reliably."
            ));
        }
        result.transactions.push(transaction);
    }
    result
}

// MARK: CoinMarketCap

#[derive(Clone, Debug, PartialEq)]
pub struct CmcRow {
    /// Stable across re-imports: built from the row's own values.
    pub id: String,
    pub line: usize,
    pub timestamp: i64,
    pub token: String,
    pub kind: PortfolioTransactionType,
    pub price: f64,
    pub amount: f64,
    pub total_value: f64,
    pub fee: f64,
    pub fee_currency: Option<PortfolioCurrency>,
    pub notes: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CmcPreview {
    pub rows: Vec<CmcRow>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl CmcPreview {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// The distinct tickers in the file, sorted.
    pub fn symbols(&self) -> Vec<String> {
        let mut symbols: Vec<String> = self.rows.iter().map(|row| row.token.clone()).collect();
        symbols.sort();
        symbols.dedup();
        symbols
    }

    /// Turns the rows into ledger transactions. A row whose ticker has no mapping, or that the
    /// user skipped, is left out with a warning. A fee in a currency other than USD needs a
    /// historical rate in `fee_fx_rates` (keyed by row id) or it is an error.
    pub fn transactions(
        &self,
        portfolio_id: u64,
        mappings: &HashMap<String, PortfolioAsset>,
        fee_fx_rates: &HashMap<String, f64>,
        skipped_rows: &HashSet<String>,
        mint: &mut Mint,
    ) -> CsvPreview {
        let mut errors = self.errors.clone();
        let mut warnings = self.warnings.clone();
        let skipped: Vec<&CmcRow> = self
            .rows
            .iter()
            .filter(|row| !mappings.contains_key(&row.token) || skipped_rows.contains(&row.id))
            .collect();
        if !skipped.is_empty() {
            let mut symbols: Vec<&str> = skipped.iter().map(|row| row.token.as_str()).collect();
            symbols.sort();
            symbols.dedup();
            warnings.push(format!(
                "Skipped {} transactions for unmapped tickers: {}. Only mapped tickers will be imported.",
                skipped.len(),
                symbols.join(", ")
            ));
        }
        let mut transactions = Vec::new();
        for row in &self.rows {
            if skipped_rows.contains(&row.id) {
                continue;
            }
            let Some(asset) = mappings.get(&row.token) else {
                continue;
            };
            let fee_currency = row.fee_currency.unwrap_or(PortfolioCurrency::Usd);
            let mut fee = row.fee;
            let mut notes = row.notes.clone();
            if row.fee != 0.0 && fee_currency != PortfolioCurrency::Usd {
                let Some(rate) = fee_fx_rates
                    .get(&row.id)
                    .copied()
                    .filter(|rate| *rate > 0.0)
                else {
                    errors.push(format!(
                        "Line {}: supply the historical {} → USD rate or skip this transaction.",
                        row.line,
                        fee_currency.code()
                    ));
                    continue;
                };
                fee = row.fee * rate;
                let disclosure = format!(
                    "CMC fee: {} {} × {rate} = {fee} USD",
                    row.fee,
                    fee_currency.code()
                );
                notes = if notes.is_empty() {
                    disclosure
                } else {
                    format!("{notes} · {disclosure}")
                };
            }
            transactions.push(PortfolioTransaction {
                id: (mint.next_id)(),
                portfolio_id,
                asset: asset.clone(),
                kind: row.kind,
                quantity: row.amount,
                price: Some(row.price),
                price_currency: PortfolioCurrency::Usd,
                fee,
                fee_currency: PortfolioCurrency::Usd,
                timestamp: row.timestamp,
                notes,
                source: PortfolioSource::CoinMarketCap,
                external_transaction_id: Some(row.id.clone()),
                created_at: mint.now_ms,
                updated_at: mint.now_ms,
            });
        }
        CsvPreview {
            transactions,
            errors,
            warnings,
        }
    }
}

fn cmc_type(raw: &str) -> Option<PortfolioTransactionType> {
    match raw {
        "buy" => Some(PortfolioTransactionType::Buy),
        "sell" => Some(PortfolioTransactionType::Sell),
        "transfer in" | "transfer_in" => Some(PortfolioTransactionType::TransferIn),
        "transfer out" | "transfer_out" => Some(PortfolioTransactionType::TransferOut),
        other => ALL_TYPES
            .into_iter()
            .find(|kind| type_name(*kind).to_lowercase() == other),
    }
}

fn cmc_decimal(text: &str) -> Option<f64> {
    let cleaned = text.replace(',', "");
    if cleaned == "--" || cleaned.is_empty() {
        return None;
    }
    decimal(&cleaned)
}

/// The UTC offset in a header like "Date (UTC+2)" or "Date (UTC-05:30)", in seconds. No sign
/// means UTC; an unparsable offset is None (UTC is used, with a warning).
fn header_offset(header: &str) -> Option<i64> {
    let Some(start) = header.find(['+', '-']) else {
        return Some(0);
    };
    let suffix: String = header[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | ':'))
        .collect();
    let sign = if suffix.starts_with('-') { -1 } else { 1 };
    let mut parts = suffix[1..].split(':');
    let hours: i64 = parts.next()?.parse().ok()?;
    let minutes: i64 = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some(sign * (hours * 3_600 + minutes * 60))
}

/// "2024-03-05 14:30:00" in the file's own offset → epoch milliseconds.
fn parse_cmc_date(text: &str, offset_secs: i64) -> Option<i64> {
    let (date, time) = text.trim().split_once(' ')?;
    parse_iso8601(&format!("{date}T{}Z", time.trim())).map(|ms| ms - offset_secs * 1_000)
}

pub fn preview_coinmarketcap(csv: &str) -> CmcPreview {
    let records = parse_records(csv);
    let Some(header) = records.first() else {
        return CmcPreview {
            errors: vec!["The CoinMarketCap CSV is empty.".to_owned()],
            ..CmcPreview::default()
        };
    };
    let normalized: Vec<String> = header
        .iter()
        .map(|name| name.trim().to_lowercase())
        .collect();
    let column = |prefix: &str| normalized.iter().position(|name| name.starts_with(prefix));
    let columns = (
        column("date"),
        column("token"),
        column("type"),
        column("price"),
        column("amount"),
        column("total value"),
        column("fee"),
        column("fee currency"),
        column("notes"),
    );
    let (
        Some(date_index),
        Some(token_index),
        Some(type_index),
        Some(price_index),
        Some(amount_index),
        Some(total_index),
        Some(fee_index),
        Some(fee_currency_index),
        Some(notes_index),
    ) = columns
    else {
        return CmcPreview {
            errors: vec!["This is not a recognized CoinMarketCap transaction export.".to_owned()],
            ..CmcPreview::default()
        };
    };

    let offset = header_offset(&header[date_index]);
    let offset_secs = offset.unwrap_or(0);
    let mut result = CmcPreview::default();
    for (position, record) in records.iter().skip(1).enumerate() {
        let line = position + 2;
        let field = |index: usize| record.get(index).map_or("", |value| value.trim());
        let token = field(token_index).to_uppercase();
        let raw_type = field(type_index).to_lowercase();
        let parsed = (
            cmc_type(&raw_type),
            parse_cmc_date(field(date_index), offset_secs),
            cmc_decimal(field(price_index)),
            cmc_decimal(field(amount_index)).filter(|amount| *amount > 0.0),
            cmc_decimal(field(total_index)),
        );
        let (Some(kind), Some(timestamp), Some(price), Some(amount), Some(total)) = parsed else {
            result.errors.push(format!(
                "Line {line}: invalid date, token, type, price, amount, or total value."
            ));
            continue;
        };
        if token.is_empty() {
            result.errors.push(format!(
                "Line {line}: invalid date, token, type, price, amount, or total value."
            ));
            continue;
        }
        let fee = cmc_decimal(field(fee_index)).unwrap_or(0.0);
        let fee_currency = parse_currency(&field(fee_currency_index).to_uppercase());
        if fee != 0.0 && fee_currency.is_none() {
            result.errors.push(format!(
                "Line {line}: fee currency is missing or unsupported."
            ));
            continue;
        }
        if (price * amount - total).abs() > f64::max(0.02, total * 0.001) {
            result.warnings.push(format!(
                "Line {line}: total value differs from price × amount."
            ));
        }
        let id = format!(
            "cmc:{}:{token}:{raw_type}:{amount}:{price}:{total}",
            timestamp.div_euclid(1_000)
        );
        result.rows.push(CmcRow {
            id,
            line,
            timestamp,
            token,
            kind,
            price,
            amount,
            total_value: total,
            fee,
            fee_currency,
            notes: field(notes_index).to_owned(),
        });
    }
    if offset.is_none() {
        result
            .warnings
            .push("The date header had no usable UTC offset; UTC was used.".to_owned());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portfolios() -> Vec<Portfolio> {
        vec![
            Portfolio::new(1, "Main", 0),
            Portfolio::new(2, "Long Term", 0),
        ]
    }

    fn run<T>(test: impl FnOnce(&mut Mint) -> T) -> T {
        let mut next = 100_u64;
        let mut ids = move || {
            next += 1;
            next - 1
        };
        let mut mint = Mint {
            now_ms: 5,
            next_id: &mut ids,
        };
        test(&mut mint)
    }

    #[test]
    fn records_handle_quotes_and_line_breaks() {
        let records = parse_records("a,b\r\n\"x,1\",\"he said \"\"hi\"\"\"\n\n\"two\nlines\",z");
        assert_eq!(
            records,
            vec![
                vec!["a".to_owned(), "b".to_owned()],
                vec!["x,1".to_owned(), "he said \"hi\"".to_owned()],
                vec!["two\nlines".to_owned(), "z".to_owned()],
            ]
        );
        assert!(parse_records("").is_empty());
        assert!(parse_records("\n\n").is_empty());
        assert_eq!(
            parse_records("a,"),
            vec![vec!["a".to_owned(), String::new()]]
        );
        for value in ["plain", "a,b", "q\"uote", "line\nbreak", ""] {
            let again = parse_records(&format!("{},end", escape(value)));
            assert_eq!(again[0][0], value, "{value:?}");
        }
    }

    #[test]
    fn timestamps_read_every_iso_form_and_write_milliseconds() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso8601("2024-03-05T14:30:00Z"),
            Some(1_709_649_000_000)
        );
        assert_eq!(
            parse_iso8601("2024-03-05T14:30:00.250Z"),
            Some(1_709_649_000_250)
        );
        assert_eq!(
            parse_iso8601("2024-03-05T16:30:00+02:00"),
            Some(1_709_649_000_000)
        );
        assert_eq!(
            parse_iso8601("2024-03-05T09:00:00-0530"),
            Some(1_709_649_000_000)
        );
        assert_eq!(
            parse_iso8601("2024-03-05T14:30:00.5Z"),
            Some(1_709_649_000_500)
        );
        for bad in [
            "",
            "2024-03-05",
            "2024-13-05T00:00:00Z",
            "2024-02-30T00:00:00Z",
            "2024-03-05T25:00:00Z",
            "2024-03-05T14:30:00",
            "yesterday",
        ] {
            assert_eq!(parse_iso8601(bad), None, "{bad}");
        }
        assert_eq!(
            parse_iso8601("2024-02-29T00:00:00Z"),
            Some(1_709_164_800_000),
            "leap day"
        );
        assert_eq!(
            format_iso8601(1_709_649_000_250),
            "2024-03-05T14:30:00.250Z"
        );
        assert_eq!(format_iso8601(0), "1970-01-01T00:00:00.000Z");
        for ms in [0, 1, 1_709_649_000_250, 4_102_444_800_999] {
            assert_eq!(parse_iso8601(&format_iso8601(ms)), Some(ms));
        }
    }

    fn sample() -> PortfolioTransaction {
        PortfolioTransaction {
            id: 1,
            portfolio_id: 1,
            asset: PortfolioAsset {
                key: "Binance:BTCUSDT".into(),
                symbol: "BTCUSDT".into(),
                name: "Bitcoin, \"BTC\"".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::StakingReward,
            quantity: 0.5,
            price: Some(60_000.25),
            price_currency: PortfolioCurrency::Usd,
            fee: 1.5,
            fee_currency: PortfolioCurrency::Eur,
            timestamp: 1_709_649_000_250,
            notes: "first\nsecond".into(),
            source: PortfolioSource::Manual,
            external_transaction_id: Some("tx-1".into()),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn exported_transactions_import_back_unchanged() {
        let original = sample();
        let csv = export_transactions(std::slice::from_ref(&original), &portfolios());
        assert!(csv.starts_with(HEADER));
        let preview = run(|mint| preview(&csv, &portfolios(), mint));
        assert!(preview.is_valid(), "{:?}", preview.errors);
        assert!(preview.warnings.is_empty());
        let imported = &preview.transactions[0];
        assert_eq!(imported.id, 100);
        assert_eq!((imported.created_at, imported.updated_at), (5, 5));
        assert_eq!(imported.source, PortfolioSource::Csv);
        let expected = PortfolioTransaction {
            id: 100,
            source: PortfolioSource::Csv,
            created_at: 5,
            updated_at: 5,
            ..original
        };
        assert_eq!(*imported, expected);
    }

    #[test]
    fn the_preview_names_every_problem_by_line() {
        let csv = format!(
            "{HEADER}\n\
             Nowhere,k,SYM,Name,Binance,Buy,1,1,USD,0,USD,2024-03-05T14:30:00Z,,id\n\
             main,k,SYM,,Binance,Buy,1,,USD,,,2024-03-05T14:30:00Z,,\n\
             Main,k,SYM,Name,Nope,Buy,1,1,USD,0,USD,2024-03-05T14:30:00Z,,id\n\
             Main,k,SYM,Name,Binance,Buy,0,1,USD,0,USD,2024-03-05T14:30:00Z,,id\n\
             Main,k,SYM,Name,Binance,Buy,1,1,USD,0,USD,not a date,,id"
        );
        let result = run(|mint| preview(&csv, &portfolios(), mint));
        assert!(!result.is_valid());
        assert_eq!(result.errors[0], "Line 2: unknown portfolio.");
        assert_eq!(result.errors.len(), 4);
        assert!(result.errors[1].starts_with("Line 4: invalid source, type, quantity"));
        assert!(result.errors[3].starts_with("Line 6:"));
        assert_eq!(
            result.transactions.len(),
            1,
            "the one good row is still read"
        );
        let good = &result.transactions[0];
        assert_eq!(
            good.asset.name, "SYM",
            "an empty name falls back to the symbol"
        );
        assert_eq!((good.price, good.fee), (None, 0.0));
        assert_eq!(
            good.fee_currency,
            PortfolioCurrency::Usd,
            "fee currency defaults to the price currency"
        );
        assert_eq!(
            result.warnings,
            ["Line 3: no external_id; repeat imports cannot be deduplicated reliably."]
        );
    }

    #[test]
    fn rows_with_a_bad_asset_price_or_fee_are_errors_not_silently_fixed() {
        let row = |asset: &str, price: &str, fee: &str| {
            format!(
                "Main,{asset},SYM,Name,Binance,Buy,1,{price},USD,{fee},USD,2024-03-05T14:30:00Z,,id"
            )
        };
        let csv = format!(
            "{HEADER}\n{}\n{}\n{}\n{}\n{}",
            row("", "1", "0"),
            row("k", "-5", "0"),
            row("k", "1", "abc"),
            row("k", "1", "-1"),
            row("k", "0", ""),
        );
        let result = run(|mint| preview(&csv, &portfolios(), mint));
        assert_eq!(result.errors.len(), 4, "{:?}", result.errors);
        assert!(result.errors[0].starts_with("Line 2: missing asset_id"));
        assert!(result.errors[1].starts_with("Line 3: the price"));
        assert!(result.errors[2].starts_with("Line 4: the fee"));
        assert!(result.errors[3].starts_with("Line 5: the fee"));
        // A free asset with no fee is fine.
        assert_eq!(result.transactions.len(), 1);
        assert_eq!(result.transactions[0].price, Some(0.0));
    }

    #[test]
    fn a_name_two_portfolios_share_is_an_error_not_a_guess() {
        let twins = vec![Portfolio::new(1, "Main", 0), Portfolio::new(2, " main ", 0)];
        let csv =
            format!("{HEADER}\nMain,k,SYM,Name,Binance,Buy,1,1,USD,0,USD,2024-03-05T14:30:00Z,,id");
        let result = run(|mint| preview(&csv, &twins, mint));
        assert!(result.transactions.is_empty());
        assert!(result.errors[0].starts_with("Line 2: more than one portfolio"));
    }

    #[test]
    fn a_file_with_the_wrong_columns_says_which() {
        let result = run(|mint| preview("portfolio,symbol\nMain,X", &portfolios(), mint));
        assert_eq!(
            result.errors,
            ["Missing columns: asset_id, source, type, quantity, currency, date."]
        );
        assert_eq!(
            run(|mint| preview("", &portfolios(), mint)).errors,
            ["The CSV is empty."]
        );
        assert!(
            run(|mint| preview(&HEADER.to_uppercase(), &portfolios(), mint)).is_valid(),
            "headers are case-insensitive"
        );
    }

    #[test]
    fn holdings_and_history_export_their_columns() {
        let holding = PortfolioHolding {
            asset: sample().asset,
            quantity: 2.0,
            cost_basis: 100.0,
            average_cost: 50.0,
            realized_pnl: 3.0,
            current_price: Some(60.0),
            previous_day_price: None,
            current_value: Some(120.0),
            unrealized_pnl: None,
            allocation: 1.0,
            is_price_stale: false,
        };
        let csv = export_holdings(&[holding], "Main", 1_709_649_000_250);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], HOLDINGS_HEADER);
        assert_eq!(
            lines[1],
            "Main,Binance:BTCUSDT,BTCUSDT,2,50,60,120,3,,2024-03-05T14:30:00Z"
        );
        let snapshot = |timestamp, value| PortfolioSnapshot {
            portfolio_id: 7,
            timestamp,
            value,
            net_contributions: 10.0,
            realized_pnl: 1.0,
            unrealized_pnl: 2.5,
            is_complete: timestamp > 0,
        };
        let history = export_history(&[snapshot(86_400_000, 12.0), snapshot(0, 11.0)]);
        let lines: Vec<&str> = history.lines().collect();
        assert_eq!(lines[0], HISTORY_HEADER);
        assert_eq!(
            lines[1], "7,1970-01-01T00:00:00Z,11,10,1,2.5,false",
            "oldest first"
        );
        assert_eq!(lines[2], "7,1970-01-02T00:00:00Z,12,10,1,2.5,true");
    }

    const CMC: &str = "Date (UTC+2),Token,Type,Price (USD),Amount,Total value (USD),Fee,Fee currency,Notes\n\
        2024-03-05 16:30:00,btc,Buy,\"60,000.00\",0.5,\"30,000.00\",--,,first\n\
        2024-03-06 10:00:00,ETH,Transfer In,3000,2,6000,1.5,EUR,\n\
        2024-03-07 10:00:00,ETH,Sell,3000,1,2500,0,,off";

    #[test]
    fn coinmarketcap_rows_keep_their_offset_and_a_stable_id() {
        let preview = preview_coinmarketcap(CMC);
        assert!(preview.is_valid(), "{:?}", preview.errors);
        assert_eq!(preview.rows.len(), 3);
        let first = &preview.rows[0];
        assert_eq!(
            first.timestamp, 1_709_649_000_000,
            "16:30 at UTC+2 is 14:30Z"
        );
        assert_eq!(
            (first.token.as_str(), first.kind),
            ("BTC", PortfolioTransactionType::Buy)
        );
        assert_eq!(
            (first.price, first.amount, first.total_value, first.fee),
            (60_000.0, 0.5, 30_000.0, 0.0)
        );
        assert_eq!(first.id, "cmc:1709649000:BTC:buy:0.5:60000:30000");
        assert_eq!(first.line, 2);
        assert_eq!(preview.rows[1].kind, PortfolioTransactionType::TransferIn);
        assert_eq!(preview.rows[1].fee_currency, Some(PortfolioCurrency::Eur));
        assert_eq!(preview.symbols(), ["BTC", "ETH"]);
        assert_eq!(
            preview.warnings,
            ["Line 4: total value differs from price × amount."]
        );
    }

    #[test]
    fn coinmarketcap_problems_are_reported_per_line() {
        assert_eq!(
            preview_coinmarketcap("a,b\n1,2").errors,
            ["This is not a recognized CoinMarketCap transaction export."]
        );
        assert_eq!(
            preview_coinmarketcap("").errors,
            ["The CoinMarketCap CSV is empty."]
        );
        let csv = "Date,Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
            2024-03-05 16:30:00,BTC,Swap,1,1,1,,,\n\
            2024-03-05 16:30:00,BTC,Buy,1,1,1,2,XYZ,\n\
            2024-03-05 16:30:00,BTC,Buy,1,0,1,,,";
        let result = preview_coinmarketcap(csv);
        assert!(result.rows.is_empty());
        assert!(result.errors[0].starts_with("Line 2: invalid date, token"));
        assert_eq!(
            result.errors[1],
            "Line 3: fee currency is missing or unsupported."
        );
        assert!(result.errors[2].starts_with("Line 4:"));
        let odd = preview_coinmarketcap(
            "Date (UTC+x),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes",
        );
        assert_eq!(
            odd.warnings,
            ["The date header had no usable UTC offset; UTC was used."]
        );
        assert_eq!(
            header_offset("Date (UTC-05:30)"),
            Some(-(5 * 3_600 + 30 * 60))
        );
        assert_eq!(header_offset("Date"), Some(0));
    }

    fn mapping(symbol: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: format!("CoinGecko:{symbol}"),
            symbol: symbol.into(),
            name: symbol.into(),
            source: DataSource::CoinGecko,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn mapped_rows_become_transactions_and_the_rest_are_skipped() {
        let preview = preview_coinmarketcap(CMC);
        let mappings = HashMap::from([("BTC".to_owned(), mapping("BTC"))]);
        let result =
            run(|mint| preview.transactions(1, &mappings, &HashMap::new(), &HashSet::new(), mint));
        assert!(result.is_valid());
        assert_eq!(result.transactions.len(), 1);
        let transaction = &result.transactions[0];
        assert_eq!(transaction.source, PortfolioSource::CoinMarketCap);
        assert_eq!(
            transaction.external_transaction_id.as_deref(),
            Some(preview.rows[0].id.as_str())
        );
        assert_eq!(transaction.notes, "first");
        assert_eq!(transaction.price_currency, PortfolioCurrency::Usd);
        assert!(result.warnings.iter().any(|warning| warning
            == "Skipped 2 transactions for unmapped tickers: ETH. Only mapped tickers will be imported."));
    }

    #[test]
    fn a_foreign_fee_needs_a_rate_or_a_skip() {
        let preview = preview_coinmarketcap(CMC);
        let mappings = HashMap::from([
            ("BTC".to_owned(), mapping("BTC")),
            ("ETH".to_owned(), mapping("ETH")),
        ]);
        let eth_buy = preview.rows[1].id.clone();

        let missing =
            run(|mint| preview.transactions(1, &mappings, &HashMap::new(), &HashSet::new(), mint));
        assert_eq!(
            missing.errors,
            ["Line 3: supply the historical EUR → USD rate or skip this transaction."]
        );
        assert!(!missing.is_valid());
        assert_eq!(
            missing.transactions.len(),
            2,
            "the other rows still convert"
        );

        let rates = HashMap::from([(eth_buy.clone(), 1.1)]);
        let converted =
            run(|mint| preview.transactions(1, &mappings, &rates, &HashSet::new(), mint));
        assert!(converted.is_valid());
        let transaction = converted
            .transactions
            .iter()
            .find(|t| t.kind == PortfolioTransactionType::TransferIn)
            .unwrap();
        assert!((transaction.fee - 1.65).abs() < 1e-9);
        assert_eq!(transaction.fee_currency, PortfolioCurrency::Usd);
        assert_eq!(
            transaction.notes,
            "CMC fee: 1.5 EUR × 1.1 = 1.6500000000000001 USD"
        );

        let skipped = HashSet::from([eth_buy]);
        let result =
            run(|mint| preview.transactions(1, &mappings, &HashMap::new(), &skipped, mint));
        assert!(result.is_valid(), "a skipped row needs no rate");
        assert_eq!(result.transactions.len(), 2);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.starts_with("Skipped 1 transactions"))
        );
    }

    #[test]
    fn type_source_and_currency_names_round_trip() {
        for kind in ALL_TYPES {
            assert_eq!(parse_type(type_name(kind)), Some(kind));
        }
        for source in ALL_SOURCES {
            assert_eq!(parse_source(source.display_name()), Some(source));
        }
        assert_eq!(
            parse_source("binance"),
            None,
            "names are exact, as in the export"
        );
        assert_eq!(parse_currency("CHF"), Some(PortfolioCurrency::Chf));
        assert_eq!(parse_currency("usd"), None);
    }
}
