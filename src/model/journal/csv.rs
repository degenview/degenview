//! The Trade Journal's CSV: the format it imports and the files it exports.
//!
//! **Import** reads executions (fills), one per row. Required columns are the time, the symbol,
//! the side, the quantity and the price; everything else is optional. Columns are found by their
//! header and can be remapped by the user. Every row either becomes a validated execution or is
//! rejected *with its row number and the reason*: nothing is dropped silently and nothing is
//! guessed (a missing fee stays "unknown", a time with no zone needs the user to say which zone).
//! A file's rows have stable identities (the file's own execution id, else a hash of the fill and
//! its place among identical fills), so importing the same file twice adds nothing.
//!
//! **Export** writes full-precision decimals, ISO-8601 UTC times and an explicit currency. The
//! executions file is the import format, so an export can be imported back.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

use super::calendar::Offset;
use super::metric::Metric;
use super::model::{
    AssetClass, CostBasis, Execution, ExecutionStatus, InstrumentRef, JournalAccount, Side,
    SourceId, Trade, TradingSourceKind,
};
use super::normalize;
use super::reviews::Review;
use super::risk;
use super::source::{
    ExecutionBatch, RejectedExecution, SourceCapabilities, SourceError, TradingActivitySource,
};
use crate::model::portfolio_csv::{escape, format_iso8601, parse_iso8601, parse_records};
use crate::model::{DataSource, InstrumentId};

/// The most rows one import reads, and the most text.
pub const MAX_ROWS: usize = 200_000;
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

// MARK: Fields and columns

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Field {
    Account,
    SourceExecId,
    Timestamp,
    Symbol,
    Provider,
    Side,
    Quantity,
    Price,
    Fee,
    FeeCurrency,
    Multiplier,
    AssetClass,
    Currency,
    OrderId,
}

impl Field {
    pub const ALL: [Field; 14] = [
        Field::Timestamp,
        Field::Symbol,
        Field::Side,
        Field::Quantity,
        Field::Price,
        Field::Fee,
        Field::Account,
        Field::SourceExecId,
        Field::Provider,
        Field::FeeCurrency,
        Field::Multiplier,
        Field::AssetClass,
        Field::Currency,
        Field::OrderId,
    ];

    pub fn required(self) -> bool {
        matches!(
            self,
            Field::Timestamp | Field::Symbol | Field::Side | Field::Quantity | Field::Price
        )
    }

    /// The header this field is written under.
    pub fn name(self) -> &'static str {
        match self {
            Field::Account => "account",
            Field::SourceExecId => "source_exec_id",
            Field::Timestamp => "timestamp",
            Field::Symbol => "symbol",
            Field::Provider => "provider",
            Field::Side => "side",
            Field::Quantity => "quantity",
            Field::Price => "price",
            Field::Fee => "fee",
            Field::FeeCurrency => "fee_currency",
            Field::Multiplier => "multiplier",
            Field::AssetClass => "asset_class",
            Field::Currency => "currency",
            Field::OrderId => "order_id",
        }
    }

    /// What the mapping sheet calls it.
    pub fn label(self) -> &'static str {
        match self {
            Field::Account => "Account",
            Field::SourceExecId => "Execution id",
            Field::Timestamp => "Time",
            Field::Symbol => "Symbol",
            Field::Provider => "Chart provider",
            Field::Side => "Side",
            Field::Quantity => "Quantity",
            Field::Price => "Price",
            Field::Fee => "Fee",
            Field::FeeCurrency => "Fee currency",
            Field::Multiplier => "Contract multiplier",
            Field::AssetClass => "Asset class",
            Field::Currency => "Currency",
            Field::OrderId => "Order id",
        }
    }

    /// Normalised headers (lower case, letters and digits only) that name this field.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Field::Account => &["account", "accountname", "accountid"],
            Field::SourceExecId => &[
                "sourceexecid",
                "executionid",
                "execid",
                "fillid",
                "tradeid",
                "id",
            ],
            Field::Timestamp => &[
                "timestamp",
                "time",
                "datetime",
                "date",
                "filledat",
                "executedat",
                "executiontime",
                "tradetime",
                "filltime",
            ],
            Field::Symbol => &["symbol", "ticker", "instrument", "asset", "market", "pair"],
            Field::Provider => &["provider", "exchange", "source", "venue"],
            Field::Side => &["side", "action", "buysell", "direction"],
            Field::Quantity => &[
                "quantity",
                "qty",
                "size",
                "amount",
                "shares",
                "filledquantity",
            ],
            Field::Price => &[
                "price",
                "fillprice",
                "executionprice",
                "avgprice",
                "tradeprice",
            ],
            Field::Fee => &["fee", "fees", "commission", "commissions"],
            Field::FeeCurrency => &["feecurrency", "commissioncurrency"],
            Field::Multiplier => &["multiplier", "contractsize", "pointvalue"],
            Field::AssetClass => &["assetclass", "class"],
            Field::Currency => &["currency", "quotecurrency"],
            Field::OrderId => &["orderid", "order", "orderref"],
        }
    }
}

fn normalised(header: &str) -> String {
    header
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Which column holds each field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnMap(BTreeMap<Field, usize>);

impl ColumnMap {
    /// Finds each field's column by its header. A column serves one field; the first match wins,
    /// in the order of [`Field::ALL`], so `side` is not taken for a `direction` column when it
    /// has its own.
    pub fn detect(headers: &[String]) -> Self {
        let names: Vec<String> = headers.iter().map(|header| normalised(header)).collect();
        let mut map = BTreeMap::new();
        let mut used = vec![false; names.len()];
        for field in Field::ALL {
            // Aliases in their stated order: the most specific first.
            let found = field.aliases().iter().find_map(|alias| {
                names
                    .iter()
                    .enumerate()
                    .position(|(index, name)| !used[index] && name == alias)
            });
            if let Some(index) = found {
                used[index] = true;
                map.insert(field, index);
            }
        }
        Self(map)
    }

    pub fn get(&self, field: Field) -> Option<usize> {
        self.0.get(&field).copied()
    }

    pub fn set(&mut self, field: Field, column: Option<usize>) {
        match column {
            Some(column) => {
                self.0.insert(field, column);
            }
            None => {
                self.0.remove(&field);
            }
        }
    }

    /// Required fields with no column.
    pub fn missing(&self) -> Vec<Field> {
        Field::ALL
            .into_iter()
            .filter(|field| field.required() && !self.0.contains_key(field))
            .collect()
    }
}

// MARK: Reading

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Splits the text into a header and rows. The first non-empty record is the header.
pub fn read_table(text: &str) -> Result<Table, String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "The file is larger than {} MB.",
            MAX_BYTES / 1024 / 1024
        ));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = parse_records(text).into_iter();
    let Some(headers) = records.next() else {
        return Err("The file is empty.".to_owned());
    };
    if headers.len() < 2 {
        return Err("The first line should be a header with one name per column.".to_owned());
    }
    let rows: Vec<Vec<String>> = records.collect();
    if rows.is_empty() {
        return Err("The file has a header and no rows.".to_owned());
    }
    if rows.len() > MAX_ROWS {
        return Err(format!("The file has more than {MAX_ROWS} rows."));
    }
    Ok(Table { headers, rows })
}

/// How a time with no zone in it is read.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NaiveTime {
    #[default]
    Utc,
    /// This Mac's time zone, as it was at that moment.
    Local,
}

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// The account every row without an account column value goes to.
    pub account_name: String,
    /// The currency of every amount in the file. A row that names another is rejected.
    pub currency: String,
    pub initial_balance: Option<Decimal>,
    pub cost_basis: CostBasis,
    pub naive_time: NaiveTime,
    /// The chart provider for rows that name none, so their trades can be shown on charts.
    pub default_provider: Option<DataSource>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            account_name: "Imported".to_owned(),
            currency: "USD".to_owned(),
            initial_balance: None,
            cost_basis: CostBasis::AverageCost,
            naive_time: NaiveTime::Utc,
            default_provider: None,
        }
    }
}

/// One row, understood.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub account: String,
    pub source_exec_id: Option<String>,
    pub ts_ms: i64,
    pub symbol: String,
    pub provider: Option<DataSource>,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee: Option<Decimal>,
    pub fee_currency: Option<String>,
    pub multiplier: Decimal,
    pub asset_class: AssetClass,
    pub order_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowOutcome {
    /// 1-based, counting the header as row 1: the number a spreadsheet shows.
    pub row: usize,
    pub result: Result<Candidate, String>,
}

/// Reads every row of `table` with `map`. `offset` turns a local clock time into UTC.
pub fn interpret(
    table: &Table,
    map: &ColumnMap,
    options: &ImportOptions,
    offset: Offset<'_>,
) -> Vec<RowOutcome> {
    table
        .rows
        .iter()
        .enumerate()
        .map(|(index, record)| RowOutcome {
            row: index + 2,
            result: interpret_row(record, map, options, offset),
        })
        .collect()
}

fn cell<'a>(record: &'a [String], map: &ColumnMap, field: Field) -> Option<&'a str> {
    let text = record.get(map.get(field)?)?.trim();
    (!text.is_empty()).then_some(text)
}

fn interpret_row(
    record: &[String],
    map: &ColumnMap,
    options: &ImportOptions,
    offset: Offset<'_>,
) -> Result<Candidate, String> {
    let need = |field: Field| {
        cell(record, map, field).ok_or_else(|| format!("{} is empty", field.label().to_lowercase()))
    };
    let ts_ms = parse_timestamp(need(Field::Timestamp)?, options.naive_time, offset)?;
    let symbol = need(Field::Symbol)?.to_owned();
    let side = parse_side(need(Field::Side)?)?;
    let quantity = parse_decimal(need(Field::Quantity)?, "quantity")?;
    let price = parse_decimal(need(Field::Price)?, "price")?;
    let fee = cell(record, map, Field::Fee)
        .map(|text| parse_decimal(text, "fee"))
        .transpose()?
        .map(|fee| fee.abs());
    let multiplier = cell(record, map, Field::Multiplier)
        .map(|text| parse_decimal(text, "contract multiplier"))
        .transpose()?
        .unwrap_or(Decimal::ONE);
    if let Some(currency) = cell(record, map, Field::Currency)
        && !currency.eq_ignore_ascii_case(&options.currency)
    {
        return Err(format!(
            "the row is in {currency} but the account is in {}; import each currency to its own account",
            options.currency
        ));
    }
    let provider = match cell(record, map, Field::Provider) {
        Some(text) => Some(
            parse_provider(text)
                .ok_or_else(|| format!("provider {text:?} is not one of the app's"))?,
        ),
        None => options.default_provider,
    };
    let candidate = Candidate {
        account: cell(record, map, Field::Account)
            .map_or_else(|| options.account_name.clone(), str::to_owned),
        source_exec_id: cell(record, map, Field::SourceExecId).map(str::to_owned),
        ts_ms,
        symbol,
        provider,
        side,
        quantity,
        price,
        fee,
        fee_currency: cell(record, map, Field::FeeCurrency).map(str::to_uppercase),
        multiplier,
        asset_class: cell(record, map, Field::AssetClass)
            .map_or(AssetClass::Other, parse_asset_class),
        order_ref: cell(record, map, Field::OrderId).map(str::to_owned),
    };
    // Validate as the store will, so the preview shows what the import will refuse.
    normalize::validate(&execution_of(&candidate, options, "preview", "preview"))?;
    Ok(candidate)
}

pub fn parse_side(text: &str) -> Result<Side, String> {
    match normalised(text).as_str() {
        "buy" | "b" | "bought" | "bot" | "long" | "buytoopen" | "buytoclose" => Ok(Side::Buy),
        "sell" | "s" | "sold" | "sld" | "short" | "sellshort" | "selltoopen" | "selltoclose" => {
            Ok(Side::Sell)
        }
        _ => Err(format!("side {text:?} is not buy or sell")),
    }
}

pub fn parse_decimal(text: &str, what: &str) -> Result<Decimal, String> {
    let cleaned: String = text
        .trim()
        .trim_start_matches(['$', '€', '£'])
        .chars()
        .filter(|c| *c != ',' && *c != ' ' && *c != '_')
        .collect();
    Decimal::from_str(&cleaned)
        .or_else(|_| Decimal::from_scientific(&cleaned))
        .map_err(|_| format!("{what} {text:?} is not a number"))
}

fn parse_provider(text: &str) -> Option<DataSource> {
    let wanted = normalised(text);
    DataSource::ALL.into_iter().find(|source| {
        normalised(source.display_name()) == wanted
            || normalised(source.watchlist_slug()) == wanted
            // "Alpaca" for "Alpaca (IEX)".
            || normalised(source.display_name()).starts_with(&wanted) && wanted.len() >= 4
    })
}

fn parse_asset_class(text: &str) -> AssetClass {
    match normalised(text).as_str() {
        "stock" | "stocks" | "equity" | "equities" | "share" | "shares" => AssetClass::Stock,
        "crypto" | "cryptocurrency" | "coin" => AssetClass::Crypto,
        "forex" | "fx" | "currency" => AssetClass::Forex,
        "future" | "futures" => AssetClass::Futures,
        "prediction" | "predictionmarket" => AssetClass::Prediction,
        _ => AssetClass::Other,
    }
}

/// ISO-8601 with a zone (`Z`, `+02:00`), epoch seconds or milliseconds, or a clock time with no
/// zone (`2026-11-09 14:03:22`), which is read in `naive`.
pub fn parse_timestamp(text: &str, naive: NaiveTime, offset: Offset<'_>) -> Result<i64, String> {
    let text = text.trim();
    if let Some(ms) = parse_iso8601(text) {
        return Ok(ms);
    }
    if text.bytes().all(|b| b.is_ascii_digit()) {
        let value: i64 = text
            .parse()
            .map_err(|_| format!("time {text:?} is not a time"))?;
        return match text.len() {
            10 => Ok(value * 1_000),
            13 => Ok(value),
            _ => Err(format!(
                "time {text:?} is a number but not epoch seconds or milliseconds"
            )),
        };
    }
    // No zone: "2026-11-09 14:03:22", "2026-11-09T14:03:22", with optional fractions.
    let as_utc = text.replacen(' ', "T", 1);
    let utc = parse_iso8601(&format!("{as_utc}Z")).ok_or_else(|| {
        format!("time {text:?} is not an ISO-8601 time like 2026-11-09T14:03:22Z")
    })?;
    Ok(match naive {
        NaiveTime::Utc => utc,
        NaiveTime::Local => {
            // The clock reading is local: take off the offset in force at that instant.
            let first = utc - offset(utc) * 1_000;
            utc - offset(first) * 1_000
        }
    })
}

// MARK: Building executions

/// The uid of an imported account.
pub fn account_uid(name: &str) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    format!("import:{slug}")
}

fn execution_of(
    candidate: &Candidate,
    options: &ImportOptions,
    account_uid: &str,
    source_exec_id: &str,
) -> Execution {
    let (key, market_key, provider) = match candidate.provider {
        Some(source) => {
            let market = InstrumentId::new(source, &candidate.symbol).key();
            (
                market.clone(),
                market,
                Some(source.display_name().to_owned()),
            )
        }
        None => {
            let key = format!("IMPORT:{}", candidate.symbol.to_uppercase());
            (key.clone(), key, None)
        }
    };
    Execution {
        uid: format!("{account_uid}|{source_exec_id}"),
        source: SourceId(format!("import:{account_uid}")),
        account_uid: account_uid.to_owned(),
        source_exec_id: source_exec_id.to_owned(),
        order_ref: candidate.order_ref.clone(),
        instrument: InstrumentRef {
            key,
            market_key,
            symbol: candidate.symbol.clone(),
            display_name: candidate.symbol.clone(),
            provider,
            asset_class: candidate.asset_class,
            quote_currency: Some(options.currency.clone()),
        },
        side: candidate.side,
        quantity: candidate.quantity,
        price: candidate.price,
        ts_ms: candidate.ts_ms,
        source_tz: Some(match options.naive_time {
            NaiveTime::Utc => "UTC".to_owned(),
            NaiveTime::Local => "local".to_owned(),
        }),
        fee: candidate.fee,
        fee_currency: candidate.fee.map(|_| {
            candidate
                .fee_currency
                .clone()
                .unwrap_or_else(|| options.currency.clone())
        }),
        liquidity: None,
        multiplier: candidate.multiplier,
        settlement_currency: Some(options.currency.clone()),
        planned_stop: None,
        status: ExecutionStatus::Active,
        meta: serde_json::Value::Null,
    }
}

/// The identity of a fill that came with none: a hash of what it is plus how many identical
/// fills came before it in the file, so two real identical fills stay two and a re-import of the
/// same file lands on the same ids.
fn derived_id(candidate: &Candidate, occurrence: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        candidate.account,
        candidate.ts_ms,
        candidate.symbol.to_uppercase(),
        candidate.side.sign(),
        candidate.quantity.normalize(),
        candidate.price.normalize(),
        candidate
            .fee
            .map(|fee| fee.normalize().to_string())
            .unwrap_or_default(),
        occurrence,
    ));
    let digest = hasher.finalize();
    let hex: String = digest
        .iter()
        .take(10)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("csv-{hex}")
}

/// What an import will do, built from the rows that were understood.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub accounts: Vec<JournalAccount>,
    pub executions: Vec<Execution>,
    /// Rows refused, with why: from reading the row, and from duplicate ids inside the file.
    pub rejected: Vec<(usize, String)>,
}

/// Turns understood rows into accounts and executions. A file id used twice in one file with
/// different content is refused for the second row.
pub fn plan(outcomes: &[RowOutcome], options: &ImportOptions, now_ms: i64) -> Plan {
    let mut plan = Plan::default();
    let mut seen_ids: HashMap<(String, String), usize> = HashMap::new();
    let mut occurrences: HashMap<String, usize> = HashMap::new();
    let mut accounts: BTreeMap<String, JournalAccount> = BTreeMap::new();
    for outcome in outcomes {
        let candidate = match &outcome.result {
            Ok(candidate) => candidate,
            Err(reason) => {
                plan.rejected.push((outcome.row, reason.clone()));
                continue;
            }
        };
        let uid = account_uid(&candidate.account);
        let source_exec_id = match &candidate.source_exec_id {
            Some(id) => {
                if let Some(first) = seen_ids.insert((uid.clone(), id.clone()), outcome.row) {
                    plan.rejected.push((
                        outcome.row,
                        format!("execution id {id:?} was already used in row {first}"),
                    ));
                    continue;
                }
                id.clone()
            }
            None => {
                let base = derived_id(candidate, 0);
                let count = occurrences.entry(base).or_insert(0);
                let id = derived_id(candidate, *count);
                *count += 1;
                id
            }
        };
        accounts
            .entry(uid.clone())
            .or_insert_with(|| JournalAccount {
                uid: uid.clone(),
                source: SourceId(format!("import:{uid}")),
                kind: TradingSourceKind::Imported,
                external_id: candidate.account.clone(),
                name: candidate.account.clone(),
                currency: options.currency.clone(),
                initial_balance: options.initial_balance,
                cost_basis: options.cost_basis,
                archived: false,
                created_at_ms: now_ms,
            });
        plan.executions
            .push(execution_of(candidate, options, &uid, &source_exec_id));
    }
    plan.accounts = accounts.into_values().collect();
    plan
}

/// A planned import as a source, so it goes through the same ingest as any other. One source per
/// imported account: another import never archives this one.
pub struct ImportSource {
    pub account: JournalAccount,
    pub executions: Vec<Execution>,
}

impl ImportSource {
    /// One source per account in the plan.
    pub fn split(plan: Plan) -> Vec<ImportSource> {
        plan.accounts
            .into_iter()
            .map(|account| ImportSource {
                executions: plan
                    .executions
                    .iter()
                    .filter(|execution| execution.account_uid == account.uid)
                    .cloned()
                    .collect(),
                account,
            })
            .collect()
    }
}

impl TradingActivitySource for ImportSource {
    fn source_id(&self) -> SourceId {
        self.account.source.clone()
    }

    fn kind(&self) -> TradingSourceKind {
        TradingSourceKind::Imported
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            historical_executions: true,
            fees: true,
            account_currency: true,
            ..SourceCapabilities::default()
        }
    }

    fn accounts(&self) -> Vec<JournalAccount> {
        vec![self.account.clone()]
    }

    fn executions(
        &self,
        account_uid: &str,
        _cursor: Option<&super::source::Cursor>,
    ) -> Result<ExecutionBatch, SourceError> {
        if account_uid != self.account.uid {
            return Err(SourceError::UnknownAccount(account_uid.to_owned()));
        }
        Ok(ExecutionBatch {
            executions: self.executions.clone(),
            voided: Vec::new(),
            next: None,
            rejected: Vec::<RejectedExecution>::new(),
        })
    }
}

// MARK: Export

pub const EXECUTIONS_HEADER: &str = "account,source_exec_id,timestamp,symbol,provider,side,quantity,price,fee,fee_currency,multiplier,asset_class,currency,order_id";
pub const TRADES_HEADER: &str = "account,trade_id,symbol,provider,direction,status,opened_at,closed_at,quantity,avg_entry,avg_exit,gross_pnl,fees,fees_known,net_pnl,currency,hold_seconds,r_multiple,playbook,rating,reviewed,tags,mistakes,emotions,notes";

fn row(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| escape(field))
        .collect::<Vec<_>>()
        .join(",")
}

fn asset_class_name(class: AssetClass) -> &'static str {
    match class {
        AssetClass::Stock => "stock",
        AssetClass::Crypto => "crypto",
        AssetClass::Forex => "forex",
        AssetClass::Futures => "futures",
        AssetClass::Prediction => "prediction",
        AssetClass::Other => "other",
    }
}

/// Executions in the import format, oldest first. `account_name` says what to call an account;
/// voided executions are left out (the source cancelled them).
pub fn export_executions(
    executions: &[&Execution],
    account_name: &dyn Fn(&str) -> String,
) -> String {
    let mut ordered: Vec<&&Execution> = executions
        .iter()
        .filter(|execution| execution.status == ExecutionStatus::Active)
        .collect();
    ordered.sort_by_key(|execution| (execution.ts_ms, execution.uid.clone()));
    let mut out = String::from(EXECUTIONS_HEADER);
    out.push('\n');
    for execution in ordered {
        let fields = [
            account_name(&execution.account_uid),
            execution.source_exec_id.clone(),
            format_iso8601(execution.ts_ms),
            execution.instrument.symbol.clone(),
            execution.instrument.provider.clone().unwrap_or_default(),
            match execution.side {
                Side::Buy => "buy",
                Side::Sell => "sell",
            }
            .to_owned(),
            execution.quantity.normalize().to_string(),
            execution.price.normalize().to_string(),
            execution
                .fee
                .map(|fee| fee.normalize().to_string())
                .unwrap_or_default(),
            execution.fee_currency.clone().unwrap_or_default(),
            execution.multiplier.normalize().to_string(),
            asset_class_name(execution.instrument.asset_class).to_owned(),
            execution
                .settlement_currency
                .clone()
                .or_else(|| execution.instrument.quote_currency.clone())
                .unwrap_or_default(),
            execution.order_ref.clone().unwrap_or_default(),
        ];
        out.push_str(&row(&fields));
        out.push('\n');
    }
    out
}

/// Trades with their results and what the user wrote, oldest first.
pub fn export_trades(
    trades: &[&Trade],
    reviews: &HashMap<String, Review>,
    playbook_name: &dyn Fn(&str) -> String,
    account_name: &dyn Fn(&str) -> String,
) -> String {
    let mut ordered: Vec<&&Trade> = trades.iter().collect();
    ordered.sort_by_key(|trade| (trade.open_ts_ms, trade.key.clone()));
    let mut out = String::from(TRADES_HEADER);
    out.push('\n');
    for trade in ordered {
        let review = reviews.get(&trade.key);
        let r = match risk::r_multiple(trade, review.and_then(|review| review.stop)) {
            Metric::Value(value) => value.round_dp(4).normalize().to_string(),
            Metric::Unavailable(_) => String::new(),
        };
        let fields = [
            account_name(&trade.account_uid),
            trade.key.clone(),
            trade.instrument.symbol.clone(),
            trade.instrument.provider.clone().unwrap_or_default(),
            match trade.direction {
                super::model::Direction::Long => "long",
                super::model::Direction::Short => "short",
            }
            .to_owned(),
            if trade.is_open() { "open" } else { "closed" }.to_owned(),
            format_iso8601(trade.open_ts_ms),
            trade.close_ts_ms.map(format_iso8601).unwrap_or_default(),
            trade.entry_quantity.normalize().to_string(),
            trade.avg_entry.normalize().to_string(),
            trade
                .avg_exit
                .map(|price| price.normalize().to_string())
                .unwrap_or_default(),
            trade.gross_pnl.normalize().to_string(),
            trade.fees.normalize().to_string(),
            trade.fees_known.to_string(),
            trade.net_pnl().normalize().to_string(),
            trade.currency.clone().unwrap_or_default(),
            trade
                .duration_ms()
                .map(|ms| (ms / 1_000).to_string())
                .unwrap_or_default(),
            r,
            review
                .and_then(|review| review.playbook.as_deref())
                .map(playbook_name)
                .unwrap_or_default(),
            review
                .and_then(|review| review.rating)
                .map(|rating| rating.to_string())
                .unwrap_or_default(),
            review.is_some_and(|review| review.reviewed).to_string(),
            review
                .map(|review| review.tags.join("; "))
                .unwrap_or_default(),
            review
                .map(|review| review.mistakes.join("; "))
                .unwrap_or_default(),
            review
                .map(|review| review.emotions.join("; "))
                .unwrap_or_default(),
            review
                .map(|review| review.notes.clone())
                .unwrap_or_default(),
        ];
        out.push_str(&row(&fields));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::d;

    fn utc(_: i64) -> i64 {
        0
    }

    fn table(text: &str) -> Table {
        read_table(text).unwrap()
    }

    const FILE: &str = "\u{feff}Time,Symbol,Side,Qty,Price,Commission,Account\n\
        2026-11-09T10:00:00Z,BTCUSDT,Buy,1,100,0.5,Main\n\
        2026-11-09T11:00:00Z,BTCUSDT,Sell,1,110,0.5,Main\n";

    #[test]
    fn headers_are_recognised_by_their_usual_names() {
        let t = table(FILE);
        let map = ColumnMap::detect(&t.headers);
        assert_eq!(map.get(Field::Timestamp), Some(0));
        assert_eq!(map.get(Field::Symbol), Some(1));
        assert_eq!(map.get(Field::Side), Some(2));
        assert_eq!(map.get(Field::Quantity), Some(3));
        assert_eq!(map.get(Field::Price), Some(4));
        assert_eq!(map.get(Field::Fee), Some(5));
        assert_eq!(map.get(Field::Account), Some(6));
        assert!(map.missing().is_empty());
        let bare = ColumnMap::detect(&["a".into(), "b".into()]);
        assert_eq!(bare.missing().len(), 5);
    }

    #[test]
    fn a_column_serves_only_one_field() {
        // "Id" is the execution id; "Order" is the order, not a second id.
        let map = ColumnMap::detect(&["Id".into(), "Order".into(), "Date".into()]);
        assert_eq!(map.get(Field::SourceExecId), Some(0));
        assert_eq!(map.get(Field::OrderId), Some(1));
        assert_eq!(map.get(Field::Timestamp), Some(2));
    }

    #[test]
    fn a_good_file_becomes_executions_that_reconstruct_a_trade() {
        let t = table(FILE);
        let map = ColumnMap::detect(&t.headers);
        let options = ImportOptions::default();
        let outcomes = interpret(&t, &map, &options, &utc);
        assert!(outcomes.iter().all(|o| o.result.is_ok()), "{outcomes:?}");
        let plan = plan(&outcomes, &options, 1);
        assert_eq!(plan.accounts.len(), 1);
        assert_eq!(plan.accounts[0].name, "Main");
        assert_eq!(plan.accounts[0].uid, "import:main");
        assert_eq!(plan.executions.len(), 2);
        let trades = reconstruct(&plan.executions, |_: &str| CostBasis::AverageCost);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].gross_pnl, d("10"));
        assert_eq!(trades[0].fees, d("1.0"));
    }

    #[test]
    fn every_bad_row_says_why_with_its_row_number_and_good_rows_still_import() {
        let text = "time,symbol,side,qty,price,fee\n\
            2026-11-09T10:00:00Z,BTC,buy,1,100,\n\
            not-a-time,BTC,buy,1,100,\n\
            2026-11-09T10:00:00Z,BTC,hold,1,100,\n\
            2026-11-09T10:00:00Z,BTC,buy,0,100,\n\
            2026-11-09T10:00:00Z,BTC,buy,1,abc,\n\
            2026-11-09T10:00:00Z,,buy,1,100,\n\
            2026-11-09T10:00:00Z,BTC,sell,1,110,-2\n";
        let t = table(text);
        let map = ColumnMap::detect(&t.headers);
        let options = ImportOptions::default();
        let outcomes = interpret(&t, &map, &options, &utc);
        let reasons: Vec<_> = outcomes
            .iter()
            .map(|o| (o.row, o.result.as_ref().err().cloned()))
            .collect();
        assert_eq!(reasons[0].1, None);
        assert!(reasons[1].1.as_ref().unwrap().contains("not an ISO-8601"));
        assert!(reasons[2].1.as_ref().unwrap().contains("not buy or sell"));
        assert!(reasons[3].1.as_ref().unwrap().contains("not positive"));
        assert!(reasons[4].1.as_ref().unwrap().contains("not a number"));
        assert!(reasons[5].1.as_ref().unwrap().contains("symbol is empty"));
        // A negative fee is written as a charge: read as its size.
        assert_eq!(reasons[6].1, None);
        assert_eq!(reasons[1].0, 3, "row numbers count the header as row 1");
        let plan = plan(&outcomes, &options, 1);
        assert_eq!(plan.executions.len(), 2);
        assert_eq!(plan.rejected.len(), 5);
        // A blank fee stays unknown, not zero.
        assert_eq!(plan.executions[0].fee, None);
        assert_eq!(plan.executions[1].fee, Some(d("2")));
    }

    #[test]
    fn times_with_no_zone_need_the_users_zone_and_epochs_are_understood() {
        let two_hours = |_: i64| 7_200;
        let utc_ms = parse_iso8601("2026-11-09T10:00:00Z").unwrap();
        assert_eq!(
            parse_timestamp("2026-11-09 10:00:00", NaiveTime::Utc, &utc),
            Ok(utc_ms)
        );
        assert_eq!(
            parse_timestamp("2026-11-09 10:00:00", NaiveTime::Local, &two_hours),
            Ok(utc_ms - 7_200_000)
        );
        // A zone in the text wins over the option.
        assert_eq!(
            parse_timestamp("2026-11-09T12:00:00+02:00", NaiveTime::Utc, &utc),
            Ok(utc_ms)
        );
        assert_eq!(
            parse_timestamp("1762682400", NaiveTime::Utc, &utc),
            Ok(1_762_682_400_000)
        );
        assert_eq!(
            parse_timestamp("1762682400123", NaiveTime::Utc, &utc),
            Ok(1_762_682_400_123)
        );
        assert!(parse_timestamp("12345", NaiveTime::Utc, &utc).is_err());
        assert!(parse_timestamp("2026-11-09", NaiveTime::Utc, &utc).is_err());
    }

    #[test]
    fn numbers_accept_thousands_separators_and_currency_signs() {
        assert_eq!(parse_decimal("1,234.50", "p"), Ok(d("1234.50")));
        assert_eq!(parse_decimal("$99", "p"), Ok(d("99")));
        assert_eq!(parse_decimal("1e3", "p"), Ok(d("1000")));
        assert!(parse_decimal("", "p").is_err());
        assert!(parse_decimal("nan", "p").is_err());
        assert!(parse_decimal("inf", "p").is_err());
    }

    #[test]
    fn a_row_in_another_currency_is_refused_not_added() {
        let text = "time,symbol,side,qty,price,currency\n2026-11-09T10:00:00Z,AAPL,buy,1,100,EUR\n";
        let t = table(text);
        let outcomes = interpret(
            &t,
            &ColumnMap::detect(&t.headers),
            &ImportOptions::default(),
            &utc,
        );
        assert!(outcomes[0].result.as_ref().unwrap_err().contains("EUR"));
    }

    #[test]
    fn identical_fills_stay_two_and_a_reimport_gets_the_same_ids() {
        let text = "time,symbol,side,qty,price\n\
            2026-11-09T10:00:00Z,BTC,buy,1,100\n\
            2026-11-09T10:00:00Z,BTC,buy,1,100\n";
        let t = table(text);
        let map = ColumnMap::detect(&t.headers);
        let options = ImportOptions::default();
        let first = plan(&interpret(&t, &map, &options, &utc), &options, 1);
        let again = plan(&interpret(&t, &map, &options, &utc), &options, 99);
        assert_eq!(first.executions.len(), 2);
        assert_ne!(first.executions[0].uid, first.executions[1].uid);
        assert_eq!(
            first.executions.iter().map(|e| &e.uid).collect::<Vec<_>>(),
            again.executions.iter().map(|e| &e.uid).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_file_id_used_twice_in_a_file_refuses_the_second_row() {
        let text = "id,time,symbol,side,qty,price\n\
            a,2026-11-09T10:00:00Z,BTC,buy,1,100\n\
            a,2026-11-09T11:00:00Z,BTC,sell,1,110\n";
        let t = table(text);
        let options = ImportOptions::default();
        let plan = plan(
            &interpret(&t, &ColumnMap::detect(&t.headers), &options, &utc),
            &options,
            1,
        );
        assert_eq!(plan.executions.len(), 1);
        assert!(plan.rejected[0].1.contains("already used in row 2"));
    }

    #[test]
    fn a_chart_provider_links_the_trades_to_the_chart_and_none_leaves_them_unlinked() {
        let text =
            "time,symbol,side,qty,price,exchange\n2026-11-09T10:00:00Z,BTCUSDT,buy,1,100,binance\n";
        let t = table(text);
        let options = ImportOptions::default();
        let plan = plan(
            &interpret(&t, &ColumnMap::detect(&t.headers), &options, &utc),
            &options,
            1,
        );
        let instrument = &plan.executions[0].instrument;
        assert_eq!(instrument.provider.as_deref(), Some("Binance"));
        assert_eq!(
            instrument.market_key,
            InstrumentId::new(DataSource::Binance, "BTCUSDT").key()
        );
        let bare = "time,symbol,side,qty,price\n2026-11-09T10:00:00Z,BTCUSDT,buy,1,100\n";
        let t = table(bare);
        let plan2 = plan_of(&t, &options);
        assert_eq!(plan2.executions[0].instrument.provider, None);
        assert_eq!(plan2.executions[0].instrument.market_key, "IMPORT:BTCUSDT");
        let with_default = ImportOptions {
            default_provider: Some(DataSource::Coinbase),
            ..ImportOptions::default()
        };
        let plan3 = plan_of(&t, &with_default);
        assert_eq!(
            plan3.executions[0].instrument.provider.as_deref(),
            Some("Coinbase")
        );
        let bad = "time,symbol,side,qty,price,exchange\n2026-11-09T10:00:00Z,X,buy,1,100,nowhere\n";
        let t = table(bad);
        assert!(plan_of(&t, &options).rejected[0].1.contains("nowhere"));
    }

    fn plan_of(t: &Table, options: &ImportOptions) -> Plan {
        plan(
            &interpret(t, &ColumnMap::detect(&t.headers), options, &utc),
            options,
            1,
        )
    }

    #[test]
    fn files_without_a_usable_shape_are_refused_with_a_reason() {
        assert!(read_table("").unwrap_err().contains("empty"));
        assert!(read_table("a\n1\n").unwrap_err().contains("header"));
        assert!(read_table("a,b\n").unwrap_err().contains("no rows"));
        assert!(
            read_table(&"x".repeat(MAX_BYTES + 1))
                .unwrap_err()
                .contains("larger")
        );
    }

    #[test]
    fn exported_executions_import_back_to_the_same_trades() {
        let options = ImportOptions {
            account_name: "Swing".into(),
            ..ImportOptions::default()
        };
        let t = table(
            "time,symbol,side,qty,price,fee,provider\n\
             2026-11-09T10:00:00Z,BTCUSDT,buy,1.5,100.25,0.10,Binance\n\
             2026-11-09T11:00:00Z,BTCUSDT,sell,0.5,110,,Binance\n\
             2026-11-09T12:00:00Z,BTCUSDT,sell,1,120.5,0.20,Binance\n",
        );
        let first = plan_of(&t, &options);
        let refs: Vec<&Execution> = first.executions.iter().collect();
        let text = export_executions(&refs, &|_| "Swing".to_owned());
        assert!(text.starts_with(EXECUTIONS_HEADER));
        let again_table = read_table(&text).unwrap();
        let again = plan_of(&again_table, &options);
        assert_eq!(again.rejected, Vec::<(usize, String)>::new());
        assert_eq!(again.executions.len(), 3);
        let basis = |_: &str| CostBasis::AverageCost;
        let a = reconstruct(&first.executions, basis);
        let b = reconstruct(&again.executions, basis);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].gross_pnl, b[0].gross_pnl);
        assert_eq!(a[0].fees, b[0].fees);
        assert_eq!(a[0].avg_entry, b[0].avg_entry);
        // The unknown fee stayed unknown through the round trip.
        assert_eq!(again.executions[1].fee, None);
        // Same ids: importing the export again adds nothing.
        assert_eq!(
            first
                .executions
                .iter()
                .map(|e| e.uid.clone())
                .collect::<Vec<_>>(),
            again
                .executions
                .iter()
                .map(|e| e.uid.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn exported_trades_carry_results_and_what_the_user_wrote() {
        let t = table(FILE);
        let options = ImportOptions::default();
        let p = plan_of(&t, &options);
        let trades = reconstruct(&p.executions, |_: &str| CostBasis::AverageCost);
        let mut review = Review::new(&trades[0].key, &trades[0].account_uid);
        review.notes = "Took it \"early\", sadly.\nNext time wait.".into();
        review.tags = vec!["breakout".into(), "am".into()];
        review.rating = Some(4);
        review.reviewed = true;
        review.playbook = Some("pb1".into());
        review.stop = Some(d("95"));
        let reviews = HashMap::from([(trades[0].key.clone(), review)]);
        let refs: Vec<&Trade> = trades.iter().collect();
        let text = export_trades(&refs, &reviews, &|_| "Breakout".to_owned(), &|_| {
            "Main".to_owned()
        });
        let records = parse_records(&text);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].join(","), TRADES_HEADER);
        let record = &records[1];
        assert_eq!(record.len(), records[0].len());
        let get = |name: &str| {
            let index = records[0].iter().position(|h| h == name).unwrap();
            record[index].clone()
        };
        assert_eq!(get("account"), "Main");
        assert_eq!(get("direction"), "long");
        assert_eq!(get("status"), "closed");
        assert_eq!(get("net_pnl"), "9");
        assert_eq!(get("fees_known"), "true");
        assert_eq!(get("hold_seconds"), "3600");
        // Risk 5 (entry 100 - stop 95): R = net 9 / 5.
        assert_eq!(get("r_multiple"), "1.8");
        assert_eq!(get("playbook"), "Breakout");
        assert_eq!(get("tags"), "breakout; am");
        assert_eq!(get("notes"), "Took it \"early\", sadly.\nNext time wait.");
    }
}
