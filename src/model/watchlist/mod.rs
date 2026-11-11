//! Watchlist domain: a named, flat, ordered list of markets and section dividers.
//! Pure data and invariants — no persistence, no UI. `WatchlistStore` (Stage 6a)
//! wraps these and decides how they are saved.

pub mod layout;
pub mod quote;
pub mod text;

use crate::model::{DataSource, InstrumentId};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Identity of one row (instrument or section), so drag, selection and removal
/// never depend on array offsets.
pub type EntryId = u64;

/// A fresh id: wall-clock nanoseconds, bumped when two are requested in the same tick.
pub fn new_id() -> EntryId {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => last = actual,
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// The most markets and sections one list holds. iCloud Sync refuses a larger record, so a
/// list that grew past it here would silently stop syncing.
pub const MAX_ENTRIES_PER_LIST: usize = 5_000;
/// The most characters in a list or section name (and in an imported title).
pub const MAX_NAME_CHARS: usize = 200;
/// The most lists. Nothing syncs or draws well past this; it stops runaway imports.
pub const MAX_LISTS: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatchlistError {
    Duplicate(String),
    NotFound,
    EmptyName,
    NameTooLong,
    InvalidName,
    /// Two sections have the same title, so a title does not say which is meant.
    AmbiguousSection(String),
    /// The list is at `MAX_ENTRIES_PER_LIST`.
    ListFull,
    /// There are already `MAX_LISTS` lists.
    TooManyLists,
    PersistenceUnavailable,
    /// The change was refused by the database; the lists are as they were.
    SaveFailed,
}

impl std::fmt::Display for WatchlistError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(name) => write!(formatter, "\"{name}\" is already in this watchlist"),
            Self::NotFound => formatter.write_str("That watchlist item no longer exists"),
            Self::EmptyName => formatter.write_str("A name can't be empty"),
            Self::NameTooLong => write!(
                formatter,
                "A name can be at most {MAX_NAME_CHARS} characters"
            ),
            Self::InvalidName => formatter.write_str("A name can't contain control characters"),
            Self::AmbiguousSection(title) => write!(
                formatter,
                "More than one section is called \"{title}\": use its section_id"
            ),
            Self::ListFull => write!(
                formatter,
                "A watchlist holds at most {MAX_ENTRIES_PER_LIST} items"
            ),
            Self::TooManyLists => write!(formatter, "You can have at most {MAX_LISTS} watchlists"),
            Self::SaveFailed => formatter.write_str("Your watchlists couldn't be saved"),
            Self::PersistenceUnavailable => formatter.write_str(
                "Your saved watchlists couldn't be read, so changes are turned off to protect them",
            ),
        }
    }
}

impl std::error::Error for WatchlistError {}

/// A refusal as the sidebar shows it: a short title and the sentence under it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchlistNotice {
    pub title: &'static str,
    pub message: String,
}

impl WatchlistNotice {
    pub fn new(title: &'static str, message: impl Into<String>) -> Self {
        Self {
            title,
            message: message.into(),
        }
    }
}

impl WatchlistError {
    pub fn notice(&self) -> WatchlistNotice {
        let title = match self {
            Self::Duplicate(_) => "Already in Watchlist",
            Self::NotFound => "Item Not Found",
            Self::EmptyName | Self::NameTooLong | Self::InvalidName => "Invalid Name",
            Self::AmbiguousSection(_) => "Ambiguous Section",
            Self::ListFull => "Watchlist Full",
            Self::TooManyLists => "Too Many Watchlists",
            Self::PersistenceUnavailable => "Changes Turned Off",
            Self::SaveFailed => "Couldn\u{2019}t Save",
        };
        let message = match self {
            Self::Duplicate(name) => {
                format!("\u{201c}{name}\u{201d} is already in this watchlist.")
            }
            other => format!("{other}."),
        };
        WatchlistNotice::new(title, message)
    }
}

/// A column the sidebar can show next to the symbol, which is always present.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum WatchlistColumn {
    Last,
    Change,
    ChangePercent,
    Volume,
    Exchange,
    Status,
}

impl WatchlistColumn {
    pub const ALL: [Self; 6] = [
        Self::Last,
        Self::Change,
        Self::ChangePercent,
        Self::Volume,
        Self::Exchange,
        Self::Status,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Last => "Last",
            Self::Change => "Chg",
            Self::ChangePercent => "Chg%",
            Self::Volume => "Vol",
            Self::Exchange => "Exchange",
            Self::Status => "Status",
        }
    }

    /// Spoken name, since the header is abbreviated.
    pub fn accessibility_title(self) -> &'static str {
        match self {
            Self::Last => "Last price",
            Self::Change => "Change",
            Self::ChangePercent => "Change percent",
            Self::Volume => "Volume",
            Self::Exchange => "Exchange",
            Self::Status => "Market status",
        }
    }

    /// The sort a click on this column's heading selects, if it can sort.
    pub fn sort_key(self) -> Option<WatchlistSortKey> {
        match self {
            Self::Last => Some(WatchlistSortKey::Last),
            Self::Change => Some(WatchlistSortKey::Change),
            Self::ChangePercent => Some(WatchlistSortKey::ChangePercent),
            Self::Volume => Some(WatchlistSortKey::Volume),
            Self::Exchange | Self::Status => None,
        }
    }
}

/// Which value orders the instruments inside each section. `Manual` is the stored order.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum WatchlistSortKey {
    #[default]
    Manual,
    Symbol,
    Last,
    Change,
    ChangePercent,
    Volume,
}

impl WatchlistSortKey {
    pub const ALL: [Self; 6] = [
        Self::Manual,
        Self::Symbol,
        Self::Last,
        Self::Change,
        Self::ChangePercent,
        Self::Volume,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Symbol => "Symbol",
            Self::Last => "Last",
            Self::Change => "Change",
            Self::ChangePercent => "Change %",
            Self::Volume => "Volume",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WatchlistSort {
    pub key: WatchlistSortKey,
    pub ascending: bool,
}

impl Default for WatchlistSort {
    fn default() -> Self {
        Self::MANUAL
    }
}

impl WatchlistSort {
    pub const MANUAL: Self = Self {
        key: WatchlistSortKey::Manual,
        ascending: true,
    };

    /// A click on a column heading: first click sorts (largest first for numbers,
    /// A to Z for symbols), the next flips, the next returns to Manual.
    pub fn after_clicking(self, key: WatchlistSortKey) -> Self {
        if self.key != key {
            return Self {
                key,
                ascending: key == WatchlistSortKey::Symbol,
            };
        }
        let first_ascending = key == WatchlistSortKey::Symbol;
        if self.ascending == first_ascending {
            Self {
                key,
                ascending: !first_ascending,
            }
        } else {
            Self::MANUAL
        }
    }
}

/// A colour mark on a market. Flags belong to the market, not to a list.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum WatchlistFlag {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl WatchlistFlag {
    pub const ALL: [Self; 6] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
        }
    }
}

/// How one watchlist looks. Sorting lives here but never rewrites the manual order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WatchlistDisplaySettings {
    pub columns: Vec<WatchlistColumn>,
    pub sort: WatchlistSort,
    pub shows_description: bool,
}

impl Default for WatchlistDisplaySettings {
    fn default() -> Self {
        Self {
            columns: vec![WatchlistColumn::Last, WatchlistColumn::ChangePercent],
            sort: WatchlistSort::MANUAL,
            shows_description: true,
        }
    }
}

impl WatchlistDisplaySettings {
    /// Shows or hides a column, keeping the canonical column order.
    pub fn set_column(&mut self, column: WatchlistColumn, visible: bool) {
        let shown = self.columns.contains(&column);
        if visible == shown {
            return;
        }
        self.columns = WatchlistColumn::ALL
            .into_iter()
            .filter(|candidate| {
                if *candidate == column {
                    visible
                } else {
                    self.columns.contains(candidate)
                }
            })
            .collect();
    }
}

/// A labelled divider. The instruments after it, up to the next section, belong to it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WatchlistSection {
    pub id: EntryId,
    pub title: String,
    #[serde(default)]
    pub is_collapsed: bool,
}

impl WatchlistSection {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            title: title.into(),
            is_collapsed: false,
        }
    }
}

/// One market in a watchlist. Carries only what is needed to find the market again
/// and to label it before a quote arrives; prices are transient and never stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WatchlistInstrument {
    pub id: EntryId,
    pub instrument: InstrumentId,
    /// Title shown first: "Bitcoin", "BTC/USDT", a prediction-market question.
    pub name: String,
    /// Short secondary label: "BTC", the symbol.
    pub label: String,
    /// Set for prediction markets, whose symbol is an opaque token id.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Provider artwork captured with the market metadata (prediction events in particular).
    #[serde(default)]
    pub image_url: Option<String>,
    /// Set for a portfolio card kept as a favorite; `instrument` is then its placeholder
    /// identity (`PortfolioCard::instrument_id`) and nothing is quoted.
    #[serde(default)]
    pub card: Option<crate::model::portfolio::card::PortfolioCard>,
}

impl WatchlistInstrument {
    /// A row for a portfolio card: the card's name, and the portfolio it covers beneath.
    pub fn for_card(card: crate::model::portfolio::card::PortfolioCard, scope_name: &str) -> Self {
        Self {
            card: Some(card),
            ..Self::new(card.instrument_id(), card.kind.title(), scope_name)
        }
    }

    pub fn new(
        instrument: InstrumentId,
        name: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            id: new_id(),
            instrument,
            name: name.into(),
            label: label.into(),
            display_name: None,
            image_url: None,
            card: None,
        }
    }

    /// A row for a search hit or a chart's market. `symbol` is what the hit displays
    /// ("BTCUSDT", "AAPL — Apple Inc.", a market's question); `full_symbol` is the
    /// provider's own id and becomes the identity.
    #[allow(dead_code)] // for the chart-card star and search-result menus
    pub fn from_market(source: DataSource, symbol: &str, full_symbol: &str) -> Self {
        let (name, label, display_name) = if source.is_prediction_market() {
            // The provider id is an opaque token, not a useful row label. Keep it solely as the
            // instrument identity and let the subtitle identify the provider.
            (symbol.to_owned(), String::new(), Some(symbol.to_owned()))
        } else {
            match source {
                DataSource::Alpaca => {
                    let parts: Vec<&str> = symbol.split(" — ").collect();
                    let name = if parts.len() > 1 {
                        parts[1..].join(" — ")
                    } else {
                        symbol.to_owned()
                    };
                    (name, full_symbol.to_uppercase(), None)
                }
                DataSource::CoinGecko => {
                    let name = full_symbol
                        .split('-')
                        .filter(|word| !word.is_empty())
                        .map(|word| {
                            let mut chars = word.chars();
                            chars.next().map_or_else(String::new, |first| {
                                first
                                    .to_uppercase()
                                    .chain(chars.flat_map(char::to_lowercase))
                                    .collect()
                            })
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    (name, symbol.to_uppercase(), None)
                }
                _ => (symbol.to_owned(), symbol.to_uppercase(), None),
            }
        };
        Self {
            display_name,
            ..Self::new(InstrumentId::new(source, full_symbol), name, label)
        }
    }

    /// A copy under a new identity, for duplicated lists.
    pub fn copy(&self) -> Self {
        Self {
            id: new_id(),
            ..self.clone()
        }
    }
}

/// One row of a watchlist's flat, ordered list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WatchlistEntry {
    Instrument(WatchlistInstrument),
    Section(WatchlistSection),
}

impl WatchlistEntry {
    pub fn id(&self) -> EntryId {
        match self {
            Self::Instrument(item) => item.id,
            Self::Section(section) => section.id,
        }
    }

    pub fn instrument(&self) -> Option<&WatchlistInstrument> {
        match self {
            Self::Instrument(item) => Some(item),
            Self::Section(_) => None,
        }
    }

    pub fn section(&self) -> Option<&WatchlistSection> {
        match self {
            Self::Section(section) => Some(section),
            Self::Instrument(_) => None,
        }
    }
}

/// A named, ordered list of markets and section dividers.
///
/// `entries` is one flat vector. A section owns the instruments after it until the
/// next section, so reordering anything is a single move and deleting a section only
/// removes its header: its instruments fall into the section above, or the root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Watchlist {
    pub id: EntryId,
    pub name: String,
    #[serde(default)]
    pub entries: Vec<WatchlistEntry>,
    #[serde(default)]
    pub display: WatchlistDisplaySettings,
    /// The list the chart-card star adds to. Exactly one list has this; it can be
    /// renamed but not deleted.
    #[serde(default)]
    pub is_favorites: bool,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

type Result<T> = std::result::Result<T, WatchlistError>;

impl Watchlist {
    pub fn new(name: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            name: name.into(),
            entries: Vec::new(),
            display: WatchlistDisplaySettings::default(),
            is_favorites: false,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    // MARK: Reading

    pub fn instruments(&self) -> impl Iterator<Item = &WatchlistInstrument> {
        self.entries.iter().filter_map(WatchlistEntry::instrument)
    }

    pub fn sections(&self) -> impl Iterator<Item = &WatchlistSection> {
        self.entries.iter().filter_map(WatchlistEntry::section)
    }

    pub fn contains(&self, instrument: &InstrumentId) -> bool {
        self.entry_for(instrument).is_some()
    }

    pub fn entry_for(&self, instrument: &InstrumentId) -> Option<&WatchlistInstrument> {
        let key = instrument.key();
        self.instruments().find(|item| item.instrument.key() == key)
    }

    pub fn index_of(&self, entry_id: EntryId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id() == entry_id)
    }

    /// The section an entry sits under, or `None` at the root.
    pub fn owning_section(&self, entry_id: EntryId) -> Option<&WatchlistSection> {
        let index = self.index_of(entry_id)?;
        self.entries[..index]
            .iter()
            .rev()
            .find_map(WatchlistEntry::section)
    }

    /// Appends a row without the duplicate check, as old stored data can hold one.
    #[cfg(test)]
    pub fn push_unchecked(&mut self, item: WatchlistInstrument) {
        self.entries.push(WatchlistEntry::Instrument(item));
    }

    /// Puts the rows right where it can without losing one: a repeated market goes (the first
    /// stays) and a repeated row id gets a fresh one. Returns whether anything changed.
    pub fn repair(&mut self) -> bool {
        let mut changed = self.collapse_duplicate_markets();
        let mut ids = std::collections::HashSet::new();
        for entry in &mut self.entries {
            if !ids.insert(entry.id()) {
                let fresh = new_id();
                match entry {
                    WatchlistEntry::Instrument(item) => item.id = fresh,
                    WatchlistEntry::Section(section) => section.id = fresh,
                }
                ids.insert(fresh);
                changed = true;
            }
        }
        changed
    }

    /// Drops a market's second and later rows (the same `InstrumentId::key`, which now counts
    /// `BTC` and `BTCUSDT` as one market). The first row stays, with its place and name.
    /// Returns whether any row went.
    fn collapse_duplicate_markets(&mut self) -> bool {
        let mut seen = std::collections::HashSet::new();
        let before = self.entries.len();
        self.entries.retain(|entry| match entry {
            WatchlistEntry::Instrument(item) => seen.insert(item.instrument.key()),
            WatchlistEntry::Section(_) => true,
        });
        self.entries.len() != before
    }

    fn is_section(&self, index: usize) -> bool {
        self.entries[index].section().is_some()
    }

    fn is_instrument(&self, index: usize) -> bool {
        self.entries[index].instrument().is_some()
    }

    // MARK: Instruments

    /// Adds at the end of a section's block, or at the end of the list when
    /// `section_id` is `None`.
    pub fn add(
        &mut self,
        instrument: WatchlistInstrument,
        section_id: Option<EntryId>,
    ) -> Result<()> {
        if self.contains(&instrument.instrument)
            || self.entries.iter().any(|entry| entry.id() == instrument.id)
        {
            return Err(WatchlistError::Duplicate(instrument.name));
        }
        self.add_checked(instrument, section_id)
    }

    /// `add` for a caller that has already ruled out a repeated market (an import keeps a set
    /// of the keys it has seen, which beats comparing every row of a long list per line).
    pub(crate) fn add_checked(
        &mut self,
        instrument: WatchlistInstrument,
        section_id: Option<EntryId>,
    ) -> Result<()> {
        if self.entries.len() >= MAX_ENTRIES_PER_LIST {
            return Err(WatchlistError::ListFull);
        }
        let index = match section_id {
            Some(section_id) => {
                let section_index = self
                    .index_of(section_id)
                    .filter(|index| self.is_section(*index))
                    .ok_or(WatchlistError::NotFound)?;
                self.block_end(section_index)
            }
            None => self.entries.len(),
        };
        self.entries
            .insert(index, WatchlistEntry::Instrument(instrument));
        Ok(())
    }

    pub fn remove_instrument(&mut self, instrument: &InstrumentId) -> bool {
        let key = instrument.key();
        let Some(index) = self.entries.iter().position(|entry| {
            entry
                .instrument()
                .is_some_and(|item| item.instrument.key() == key)
        }) else {
            return false;
        };
        self.entries.remove(index);
        true
    }

    pub fn remove_entry(&mut self, entry_id: EntryId) -> Result<()> {
        let index = self
            .index_of(entry_id)
            .filter(|index| self.is_instrument(*index))
            .ok_or(WatchlistError::NotFound)?;
        self.entries.remove(index);
        Ok(())
    }

    // MARK: Ordering

    /// Moves an instrument or a whole section (header plus its block) to sit before
    /// `target_id`, or to the end when `None`. Dropping onto itself, or a section into
    /// its own block, changes nothing.
    pub fn move_entry(&mut self, entry_id: EntryId, before: Option<EntryId>) -> Result<()> {
        let from = self.index_of(entry_id).ok_or(WatchlistError::NotFound)?;
        let target = match before {
            Some(target_id) => Some(self.index_of(target_id).ok_or(WatchlistError::NotFound)?),
            None => None,
        };
        if before == Some(entry_id) {
            return Ok(());
        }

        if self.is_section(from) {
            let range = from..self.block_end(from);
            let mut destination =
                target.map_or(self.entries.len(), |index| self.block_start(index));
            if range.contains(&destination) || destination == range.end {
                return Ok(());
            }
            let block: Vec<WatchlistEntry> = self.entries.drain(range.clone()).collect();
            if destination > range.start {
                destination -= block.len();
            }
            self.entries.splice(destination..destination, block);
        } else {
            let entry = self.entries.remove(from);
            // The target's index may have shifted after the removal.
            let destination = match before {
                Some(target_id) => self.index_of(target_id).unwrap_or(self.entries.len()),
                None => self.entries.len(),
            };
            self.entries.insert(destination, entry);
        }
        Ok(())
    }

    /// Moves several entries together to sit before `before` (or at the end), keeping
    /// their relative order. Nothing moves if any id is unknown.
    pub fn move_entries(&mut self, ids: &[EntryId], before: Option<EntryId>) -> Result<()> {
        if ids.iter().any(|id| self.index_of(*id).is_none()) {
            return Err(WatchlistError::NotFound);
        }
        if before.is_some_and(|target| ids.contains(&target)) {
            return Ok(());
        }
        for id in ids {
            self.move_entry(*id, before)?;
        }
        Ok(())
    }

    /// Moves several instruments to the end of a section's block (or the root block),
    /// in the order given. Nothing moves if any id is not an instrument here, or the
    /// section does not exist.
    pub fn move_instruments(&mut self, ids: &[EntryId], section_id: Option<EntryId>) -> Result<()> {
        let all_instruments = ids.iter().all(|id| {
            self.index_of(*id)
                .is_some_and(|index| self.is_instrument(index))
        });
        if !all_instruments {
            return Err(WatchlistError::NotFound);
        }
        if let Some(section_id) = section_id
            && !self
                .index_of(section_id)
                .is_some_and(|index| self.is_section(index))
        {
            return Err(WatchlistError::NotFound);
        }
        for id in ids {
            self.move_instrument(*id, section_id)?;
        }
        Ok(())
    }

    /// Moves an instrument to the end of a section's block, or to the end of the root block.
    pub fn move_instrument(
        &mut self,
        entry_id: EntryId,
        section_id: Option<EntryId>,
    ) -> Result<()> {
        let from = self
            .index_of(entry_id)
            .filter(|index| self.is_instrument(*index))
            .ok_or(WatchlistError::NotFound)?;
        if let Some(section_id) = section_id
            && !self
                .index_of(section_id)
                .is_some_and(|index| self.is_section(index))
        {
            return Err(WatchlistError::NotFound);
        }
        let entry = self.entries.remove(from);
        let destination = match section_id {
            Some(section_id) => {
                let section_index = self.index_of(section_id).ok_or(WatchlistError::NotFound)?;
                self.block_end(section_index)
            }
            None => self
                .entries
                .iter()
                .position(|entry| entry.section().is_some())
                .unwrap_or(self.entries.len()),
        };
        self.entries.insert(destination, entry);
        Ok(())
    }

    // MARK: Sections

    pub fn add_section(
        &mut self,
        title: &str,
        before: Option<EntryId>,
    ) -> Result<WatchlistSection> {
        let section = WatchlistSection::new(Self::validated(title)?);
        if self.entries.len() >= MAX_ENTRIES_PER_LIST {
            return Err(WatchlistError::ListFull);
        }
        let destination = before
            .and_then(|id| self.index_of(id))
            .unwrap_or(self.entries.len());
        self.entries
            .insert(destination, WatchlistEntry::Section(section.clone()));
        Ok(section)
    }

    fn section_mut(&mut self, id: EntryId) -> Result<&mut WatchlistSection> {
        self.entries
            .iter_mut()
            .find_map(|entry| match entry {
                WatchlistEntry::Section(section) if section.id == id => Some(section),
                _ => None,
            })
            .ok_or(WatchlistError::NotFound)
    }

    /// The section a caller means by `reference`: its id, or failing that its title (case
    /// ignored). A title two sections share is refused rather than guessed.
    pub fn resolve_section(&self, reference: &str) -> Result<EntryId> {
        let reference = reference.trim();
        if let Ok(id) = reference.parse::<EntryId>()
            && self.sections().any(|section| section.id == id)
        {
            return Ok(id);
        }
        let mut found = self
            .sections()
            .filter(|section| section.title.to_lowercase() == reference.to_lowercase());
        match (found.next(), found.next()) {
            (Some(section), None) => Ok(section.id),
            (Some(_), Some(_)) => Err(WatchlistError::AmbiguousSection(reference.to_owned())),
            (None, _) => Err(WatchlistError::NotFound),
        }
    }

    pub fn rename_section(&mut self, id: EntryId, title: &str) -> Result<()> {
        let clean = Self::validated(title)?;
        self.section_mut(id)?.title = clean;
        Ok(())
    }

    pub fn set_section_collapsed(&mut self, id: EntryId, collapsed: bool) -> Result<()> {
        self.section_mut(id)?.is_collapsed = collapsed;
        Ok(())
    }

    /// Removes the header only; its instruments join the section above, or the root.
    pub fn delete_section(&mut self, id: EntryId) -> Result<()> {
        let index = self
            .index_of(id)
            .filter(|index| self.is_section(*index))
            .ok_or(WatchlistError::NotFound)?;
        self.entries.remove(index);
        Ok(())
    }

    // MARK: Whole list

    /// A copy under new identities throughout, so it never shares a row id with the original.
    pub fn duplicated(&self, name: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            name: name.into(),
            entries: self
                .entries
                .iter()
                .map(|entry| match entry {
                    WatchlistEntry::Instrument(item) => WatchlistEntry::Instrument(item.copy()),
                    WatchlistEntry::Section(section) => WatchlistEntry::Section(WatchlistSection {
                        id: new_id(),
                        ..section.clone()
                    }),
                })
                .collect(),
            display: self.display.clone(),
            is_favorites: false,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    /// Reasons this list breaks an invariant. Empty for a healthy list.
    #[cfg(test)]
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut ids = std::collections::HashSet::new();
        let mut markets = std::collections::HashSet::new();
        for entry in &self.entries {
            if !ids.insert(entry.id()) {
                problems.push(format!("duplicate entry id {}", entry.id()));
            }
            if let Some(item) = entry.instrument()
                && !markets.insert(item.instrument.key())
            {
                problems.push(format!("duplicate market {}", item.instrument.key()));
            }
        }
        problems
    }

    /// A list or section name as it is stored: runs of whitespace (a pasted newline, a tab)
    /// become one space, and an empty, over-long or control-character name is refused.
    pub fn validated(name: &str) -> Result<String> {
        let clean = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.is_empty() {
            Err(WatchlistError::EmptyName)
        } else if clean.chars().any(char::is_control) {
            Err(WatchlistError::InvalidName)
        } else if clean.chars().count() > MAX_NAME_CHARS {
            Err(WatchlistError::NameTooLong)
        } else {
            Ok(clean)
        }
    }

    /// Imported or pasted text made fit for a name: control characters dropped, whitespace
    /// collapsed, cut at `MAX_NAME_CHARS`. Empty if nothing is left.
    pub fn sanitized_name(text: &str) -> String {
        let kept: String = text
            .chars()
            .filter(|c| !c.is_control() || c.is_whitespace())
            .collect();
        kept.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_NAME_CHARS)
            .collect()
    }

    /// Records that the list's content was edited (rows or names, not how it is shown).
    pub fn touch(&mut self) {
        self.updated_at_ms = now_ms().max(self.updated_at_ms);
    }

    // MARK: Block geometry

    /// Index just past the last instrument that belongs to the section at `section_index`.
    fn block_end(&self, section_index: usize) -> usize {
        let mut index = section_index + 1;
        while index < self.entries.len() && !self.is_section(index) {
            index += 1;
        }
        index
    }

    /// Where a section dropped "before" this entry should land: the header of the block
    /// the entry sits in, so a block is never split.
    fn block_start(&self, index: usize) -> usize {
        let mut cursor = index;
        while cursor > 0 && !self.is_section(cursor) {
            cursor -= 1;
        }
        if self.is_section(cursor) { cursor } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_refusal_reads_as_a_titled_sentence() {
        let notice = WatchlistError::Duplicate("BTC".to_owned()).notice();
        assert_eq!(notice.title, "Already in Watchlist");
        assert_eq!(
            notice.message,
            "\u{201c}BTC\u{201d} is already in this watchlist."
        );
        let full = WatchlistError::ListFull.notice();
        assert_eq!(full.title, "Watchlist Full");
        assert!(full.message.ends_with("items."), "{}", full.message);
    }

    use super::*;
    use crate::model::DataSource;

    fn item(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument::new(
            InstrumentId::new(DataSource::Binance, symbol),
            symbol,
            symbol.trim_end_matches("USDT"),
        )
    }

    #[test]
    fn a_portfolio_card_is_a_favorite_by_its_kind_and_portfolio_not_its_range() {
        use crate::model::portfolio::card::{CardKind, PortfolioCard};
        use crate::model::portfolio::statistics::PortfolioHistoryRange;
        let mut list = Watchlist::new("Favorites");
        let mut card = PortfolioCard::new(CardKind::TopPerformers, Some(4));
        list.add(WatchlistInstrument::for_card(card, "Swing"), None)
            .unwrap();
        assert!(list.contains(&card.instrument_id()));

        // Another range of the same card is the same favorite; another portfolio is not.
        card.range = PortfolioHistoryRange::OneYear;
        assert!(list.contains(&card.instrument_id()));
        assert!(
            list.add(WatchlistInstrument::for_card(card, "Swing"), None)
                .is_err()
        );
        let other = PortfolioCard::new(CardKind::TopPerformers, None);
        assert!(!list.contains(&other.instrument_id()));

        let row = list.instruments().next().unwrap();
        assert_eq!(row.name, "Top Performers");
        assert_eq!(row.label, "Swing");
        let stored = serde_json::to_string(&list).unwrap();
        let decoded: Watchlist = serde_json::from_str(&stored).unwrap();
        assert_eq!(decoded, list);
    }

    #[test]
    fn a_market_row_saved_before_cards_decodes_without_one() {
        let mut row = serde_json::to_value(item("BTCUSDT")).unwrap();
        row.as_object_mut().unwrap().remove("card");
        let decoded: WatchlistInstrument = serde_json::from_value(row).unwrap();
        assert_eq!(decoded.card, None);
    }

    /// Titles/labels in stored order; sections in brackets.
    fn shape(list: &Watchlist) -> Vec<String> {
        list.entries
            .iter()
            .map(|entry| match entry {
                WatchlistEntry::Section(section) => format!("[{}]", section.title),
                WatchlistEntry::Instrument(item) => item.label.clone(),
            })
            .collect()
    }

    fn sample() -> (Watchlist, Vec<EntryId>) {
        // BTC ETH [Majors] SOL XRP [Watching] DOGE
        let mut list = Watchlist::new("Crypto");
        let mut ids = Vec::new();
        for symbol in ["BTCUSDT", "ETHUSDT"] {
            let entry = item(symbol);
            ids.push(entry.id);
            list.add(entry, None).unwrap();
        }
        ids.push(list.add_section("Majors", None).unwrap().id);
        for symbol in ["SOLUSDT", "XRPUSDT"] {
            let entry = item(symbol);
            ids.push(entry.id);
            list.add(entry, None).unwrap();
        }
        ids.push(list.add_section("Watching", None).unwrap().id);
        let doge = item("DOGEUSDT");
        ids.push(doge.id);
        list.add(doge, None).unwrap();
        (list, ids)
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let ids: Vec<EntryId> = (0..1_000).map(|_| new_id()).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn add_rejects_duplicates_case_insensitively() {
        let mut list = Watchlist::new("A");
        list.add(item("BTCUSDT"), None).unwrap();
        let again = WatchlistInstrument::new(
            InstrumentId::new(DataSource::Binance, "btcusdt"),
            "BTC",
            "BTC",
        );
        assert!(matches!(
            list.add(again, None),
            Err(WatchlistError::Duplicate(_))
        ));
        // The same symbol on another provider is a different market.
        let coinbase = WatchlistInstrument::new(
            InstrumentId::new(DataSource::Coinbase, "BTC-USD"),
            "BTC/USD",
            "BTC",
        );
        assert!(list.add(coinbase, None).is_ok());
    }

    #[test]
    fn add_to_section_lands_at_the_end_of_its_block() {
        let (mut list, ids) = sample();
        list.add(item("ADAUSDT"), Some(ids[2])).unwrap();
        assert_eq!(
            shape(&list),
            [
                "BTC",
                "ETH",
                "[Majors]",
                "SOL",
                "XRP",
                "ADA",
                "[Watching]",
                "DOGE"
            ]
        );
        assert_eq!(
            list.add(item("LTCUSDT"), Some(ids[0])),
            Err(WatchlistError::NotFound)
        );
    }

    #[test]
    fn spellings_of_one_market_are_one_row() {
        let mut list = Watchlist::new("L");
        list.add(item("BTCUSDT"), None).unwrap();
        assert!(matches!(
            list.add(item("btc"), None),
            Err(WatchlistError::Duplicate(_))
        ));
        assert!(list.contains(&InstrumentId::new(DataSource::Binance, "BTC/USDT")));
        assert!(list.remove_instrument(&InstrumentId::new(DataSource::Binance, "BTC")));
        assert_eq!(list.instruments().count(), 0);
    }

    #[test]
    fn collapsing_keeps_the_first_row_of_a_market_and_every_section() {
        let mut list = Watchlist::new("L");
        // Rows saved before aliases shared a key can sit side by side.
        list.entries.push(WatchlistEntry::Instrument(item("BTC")));
        list.add_section("Alts", None).unwrap();
        list.entries
            .push(WatchlistEntry::Instrument(item("BTCUSDT")));
        list.entries
            .push(WatchlistEntry::Instrument(item("ETHUSDT")));
        assert!(list.repair());
        let labels: Vec<String> = list.instruments().map(|item| item.label.clone()).collect();
        assert_eq!(labels, ["BTC", "ETH"]);
        assert_eq!(list.sections().count(), 1);
        assert!(!list.repair(), "nothing left to drop");
    }

    #[test]
    fn names_are_cleaned_then_checked() {
        assert_eq!(Watchlist::validated("  a \n b\t"), Ok("a b".to_owned()));
        assert_eq!(Watchlist::validated(" \n "), Err(WatchlistError::EmptyName));
        assert_eq!(
            Watchlist::validated("a\u{1b}b"),
            Err(WatchlistError::InvalidName)
        );
        let at_limit = "é".repeat(MAX_NAME_CHARS);
        assert!(Watchlist::validated(&at_limit).is_ok());
        assert_eq!(
            Watchlist::validated(&format!("{at_limit}é")),
            Err(WatchlistError::NameTooLong)
        );
        assert_eq!(Watchlist::sanitized_name("a\u{7} b\n c"), "a b c");
        assert_eq!(
            Watchlist::sanitized_name(&"x".repeat(MAX_NAME_CHARS + 9))
                .chars()
                .count(),
            MAX_NAME_CHARS
        );
    }

    #[test]
    fn a_list_stops_at_its_entry_limit() {
        let mut list = Watchlist::new("Big");
        for index in 0..MAX_ENTRIES_PER_LIST {
            list.push_unchecked(item(&format!("S{index:04}USDT")));
        }
        assert_eq!(
            list.add(item("LASTUSDT"), None),
            Err(WatchlistError::ListFull)
        );
        assert_eq!(
            list.add_section("More", None),
            Err(WatchlistError::ListFull)
        );
        list.remove_instrument(&InstrumentId::new(DataSource::Binance, "S0000USDT"));
        assert!(list.add(item("LASTUSDT"), None).is_ok());
    }

    #[test]
    fn a_repeated_row_id_gets_a_fresh_one_and_no_row_is_lost() {
        let mut list = Watchlist::new("L");
        let (a, mut b) = (item("BTCUSDT"), item("ETHUSDT"));
        b.id = a.id;
        let section = WatchlistSection::new("S".to_owned());
        list.push_unchecked(a);
        list.push_unchecked(b);
        list.entries.push(WatchlistEntry::Section(WatchlistSection {
            id: section.id,
            ..section.clone()
        }));
        list.entries.push(WatchlistEntry::Section(section));
        assert_eq!(list.validate().len(), 2);
        assert!(list.repair());
        assert!(list.validate().is_empty());
        assert_eq!(list.entries.len(), 4);
        assert!(!list.repair());
    }

    #[test]
    fn owning_section_of_an_entry() {
        let (list, ids) = sample();
        assert!(list.owning_section(ids[0]).is_none());
        assert_eq!(list.owning_section(ids[3]).unwrap().title, "Majors");
    }

    #[test]
    fn deleting_a_section_keeps_its_instruments() {
        let (mut list, ids) = sample();
        list.delete_section(ids[2]).unwrap();
        assert_eq!(
            shape(&list),
            ["BTC", "ETH", "SOL", "XRP", "[Watching]", "DOGE"]
        );
        assert_eq!(list.delete_section(ids[0]), Err(WatchlistError::NotFound));
    }

    #[test]
    fn move_instrument_before_target_and_to_end() {
        let (mut list, ids) = sample();
        list.move_entry(ids[0], Some(ids[4])).unwrap(); // BTC before XRP
        assert_eq!(
            shape(&list),
            ["ETH", "[Majors]", "SOL", "BTC", "XRP", "[Watching]", "DOGE"]
        );
        list.move_entry(ids[0], None).unwrap();
        assert_eq!(*shape(&list).last().unwrap(), "BTC");
        assert_eq!(list.move_entry(999, None), Err(WatchlistError::NotFound));
    }

    #[test]
    fn moving_a_section_carries_its_block_and_never_splits_one() {
        let (mut list, ids) = sample();
        // Majors block (+SOL, XRP) to the end.
        list.move_entry(ids[2], None).unwrap();
        assert_eq!(
            shape(&list),
            ["BTC", "ETH", "[Watching]", "DOGE", "[Majors]", "SOL", "XRP"]
        );
        // Drop Watching "before XRP": lands before the Majors header, not inside it.
        list.move_entry(ids[5], Some(ids[4])).unwrap();
        assert_eq!(
            shape(&list),
            ["BTC", "ETH", "[Watching]", "DOGE", "[Majors]", "SOL", "XRP"],
            "dropping a section inside the block that follows it is a no-op"
        );
        // Majors into the middle of Watching's block lands before the Watching header.
        list.move_entry(ids[2], Some(ids[6])).unwrap();
        assert_eq!(
            shape(&list),
            ["BTC", "ETH", "[Majors]", "SOL", "XRP", "[Watching]", "DOGE"]
        );
    }

    #[test]
    fn move_instruments_to_section_and_root() {
        let (mut list, ids) = sample();
        list.move_instruments(&[ids[0], ids[1]], Some(ids[5]))
            .unwrap();
        assert_eq!(
            shape(&list),
            ["[Majors]", "SOL", "XRP", "[Watching]", "DOGE", "BTC", "ETH"]
        );
        list.move_instruments(&[ids[1]], None).unwrap();
        assert_eq!(
            shape(&list),
            ["ETH", "[Majors]", "SOL", "XRP", "[Watching]", "DOGE", "BTC"]
        );
        assert_eq!(
            list.move_instruments(&[ids[2]], None),
            Err(WatchlistError::NotFound),
            "a section is not an instrument"
        );
        assert_eq!(
            list.move_instruments(&[ids[1]], Some(ids[1])),
            Err(WatchlistError::NotFound)
        );
    }

    #[test]
    fn move_entries_keeps_relative_order_and_is_atomic_on_unknown_ids() {
        let (mut list, ids) = sample();
        let before = shape(&list);
        assert_eq!(
            list.move_entries(&[ids[0], 12345], None),
            Err(WatchlistError::NotFound)
        );
        assert_eq!(shape(&list), before);
        // Before the Watching header: the end of the Majors block, order kept.
        list.move_entries(&[ids[0], ids[1]], Some(ids[5])).unwrap();
        assert_eq!(
            shape(&list),
            ["[Majors]", "SOL", "XRP", "BTC", "ETH", "[Watching]", "DOGE"]
        );
        // Before DOGE: inside the Watching block.
        list.move_entries(&[ids[0]], Some(ids[6])).unwrap();
        assert_eq!(
            shape(&list),
            ["[Majors]", "SOL", "XRP", "ETH", "[Watching]", "BTC", "DOGE"]
        );
    }

    #[test]
    fn sections_validate_and_rename() {
        let (mut list, ids) = sample();
        assert_eq!(list.add_section("  ", None), Err(WatchlistError::EmptyName));
        list.rename_section(ids[2], "  Core ").unwrap();
        assert_eq!(list.owning_section(ids[3]).unwrap().title, "Core");
        assert_eq!(
            list.rename_section(ids[0], "x"),
            Err(WatchlistError::NotFound)
        );
    }

    #[test]
    fn duplicate_shares_no_ids_and_is_not_favorites() {
        let (mut list, _) = sample();
        list.is_favorites = true;
        let copy = list.duplicated("Copy");
        assert!(!copy.is_favorites);
        assert_eq!(shape(&copy), shape(&list));
        let original: std::collections::HashSet<EntryId> =
            list.entries.iter().map(WatchlistEntry::id).collect();
        assert!(
            copy.entries
                .iter()
                .all(|entry| !original.contains(&entry.id()))
        );
        assert!(copy.validate().is_empty());
    }

    #[test]
    fn validate_reports_broken_invariants() {
        let (mut list, _) = sample();
        let duplicate = list.entries[0].clone();
        list.entries.push(duplicate);
        assert_eq!(list.validate().len(), 2);
    }

    #[test]
    fn remove_by_market_or_row() {
        let (mut list, ids) = sample();
        assert!(list.remove_instrument(&InstrumentId::new(DataSource::Binance, "ethusdt")));
        assert!(!list.remove_instrument(&InstrumentId::new(DataSource::Binance, "ethusdt")));
        list.remove_entry(ids[0]).unwrap();
        assert_eq!(list.remove_entry(ids[2]), Err(WatchlistError::NotFound));
    }

    #[test]
    fn sort_click_cycles_number_columns_and_symbol_differently() {
        let mut sort = WatchlistSort::MANUAL;
        sort = sort.after_clicking(WatchlistSortKey::ChangePercent);
        assert_eq!(
            (sort.key, sort.ascending),
            (WatchlistSortKey::ChangePercent, false)
        );
        sort = sort.after_clicking(WatchlistSortKey::ChangePercent);
        assert!(sort.ascending);
        sort = sort.after_clicking(WatchlistSortKey::ChangePercent);
        assert_eq!(sort, WatchlistSort::MANUAL);

        sort = sort.after_clicking(WatchlistSortKey::Symbol);
        assert_eq!((sort.key, sort.ascending), (WatchlistSortKey::Symbol, true));
        sort = sort.after_clicking(WatchlistSortKey::Symbol);
        assert!(!sort.ascending);
        sort = sort.after_clicking(WatchlistSortKey::Symbol);
        assert_eq!(sort, WatchlistSort::MANUAL);
    }

    #[test]
    fn rows_from_markets_follow_each_providers_labelling() {
        let btc = WatchlistInstrument::from_market(DataSource::Binance, "BTCUSDT", "BTCUSDT");
        assert_eq!(
            (btc.name.as_str(), btc.label.as_str()),
            ("BTCUSDT", "BTCUSDT")
        );

        let gecko = WatchlistInstrument::from_market(DataSource::CoinGecko, "btc", "bitcoin-cash");
        assert_eq!(
            (gecko.name.as_str(), gecko.label.as_str()),
            ("Bitcoin Cash", "BTC")
        );
        assert_eq!(gecko.instrument.symbol, "bitcoin-cash");

        let stock =
            WatchlistInstrument::from_market(DataSource::Alpaca, "AAPL — Apple Inc.", "aapl");
        assert_eq!(
            (stock.name.as_str(), stock.label.as_str()),
            ("Apple Inc.", "AAPL")
        );
        assert_eq!(stock.instrument.symbol, "aapl");

        let market =
            WatchlistInstrument::from_market(DataSource::Polymarket, "Will it rain?", "9876");
        assert_eq!(market.name, "Will it rain?");
        assert!(market.label.is_empty());
        assert_eq!(market.display_name.as_deref(), Some("Will it rain?"));
        assert_eq!(market.instrument.symbol, "9876");
    }

    #[test]
    fn watchlist_artwork_round_trips_and_old_rows_default_to_none() {
        let mut market =
            WatchlistInstrument::from_market(DataSource::Polymarket, "Will it rain?", "9876");
        market.image_url = Some("https://img/rain.png".into());
        let encoded = serde_json::to_value(&market).expect("serialize");
        let decoded: WatchlistInstrument =
            serde_json::from_value(encoded.clone()).expect("round trip");
        assert_eq!(decoded.image_url, market.image_url);

        let mut old = encoded;
        old.as_object_mut().unwrap().remove("image_url");
        let decoded: WatchlistInstrument = serde_json::from_value(old).expect("old row");
        assert_eq!(decoded.image_url, None);
    }

    #[test]
    fn columns_toggle_in_canonical_order() {
        let mut display = WatchlistDisplaySettings::default();
        display.set_column(WatchlistColumn::Volume, true);
        display.set_column(WatchlistColumn::Change, true);
        assert_eq!(
            display.columns,
            [
                WatchlistColumn::Last,
                WatchlistColumn::Change,
                WatchlistColumn::ChangePercent,
                WatchlistColumn::Volume
            ]
        );
        display.set_column(WatchlistColumn::Last, false);
        display.set_column(WatchlistColumn::Last, false);
        assert_eq!(display.columns.len(), 3);
        assert!(!display.columns.contains(&WatchlistColumn::Last));
    }

    #[test]
    fn serde_round_trip_and_old_rows_decode() {
        let (list, _) = sample();
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(serde_json::from_str::<Watchlist>(&json).unwrap(), list);
        // Older rows lack everything but id and name.
        let minimal: Watchlist = serde_json::from_str(r#"{"id":1,"name":"Old"}"#).unwrap();
        assert!(minimal.entries.is_empty());
        assert_eq!(minimal.display, WatchlistDisplaySettings::default());
        assert!(!minimal.is_favorites);
    }
}
