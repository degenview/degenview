//! Turns a watchlist plus the viewer's filter and sort into the rows the sidebar
//! draws, and turns a drop on those rows back into a move of stored entries.
//!
//! Pure: it never changes the stored order. Sorting happens inside each section, so
//! section headers stay where the user put them, and filtering hides sections left
//! empty.

use crate::model::InstrumentId;
use crate::model::watchlist::quote::WatchlistQuote;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistEntry, WatchlistFlag, WatchlistInstrument, WatchlistSection,
    WatchlistSort, WatchlistSortKey,
};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Section {
        section: WatchlistSection,
        count: usize,
    },
    Instrument {
        item: WatchlistInstrument,
        section_id: Option<EntryId>,
    },
    /// Placeholder under an expanded section that holds nothing.
    EmptySection { section_id: EntryId },
}

impl Row {
    /// Stable key for the row, including placeholders (which stand for no entry).
    #[cfg(test)]
    pub fn key(&self) -> String {
        match self {
            Self::Section { section, .. } => section.id.to_string(),
            Self::Instrument { item, .. } => item.id.to_string(),
            Self::EmptySection { section_id } => format!("empty-{section_id}"),
        }
    }

    /// The stored entry this row stands for, if any.
    pub fn entry_id(&self) -> Option<EntryId> {
        match self {
            Self::Section { section, .. } => Some(section.id),
            Self::Instrument { item, .. } => Some(item.id),
            Self::EmptySection { .. } => None,
        }
    }

    /// The section this row belongs to or heads; `None` for a symbol at the root.
    pub fn owning_section_id(&self) -> Option<EntryId> {
        match self {
            Self::Instrument { section_id, .. } => *section_id,
            Self::Section { section, .. } => Some(section.id),
            Self::EmptySection { section_id } => Some(*section_id),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub text: String,
    pub flag: Option<WatchlistFlag>,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        !self.trimmed_text().is_empty() || self.flag.is_some()
    }

    pub fn trimmed_text(&self) -> &str {
        self.text.trim()
    }
}

/// Market flags by `InstrumentId::key()`.
pub type Flags = HashMap<String, WatchlistFlag>;

pub fn rows(
    list: &Watchlist,
    filter: &Filter,
    flags: &Flags,
    sort: WatchlistSort,
    quote: &dyn Fn(&InstrumentId) -> Option<WatchlistQuote>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for block in blocks(list) {
        let mut items: Vec<&WatchlistInstrument> = block
            .items
            .iter()
            .copied()
            .filter(|item| matches(item, filter, flags))
            .collect();
        if sort.key != WatchlistSortKey::Manual {
            items = sorted(items, sort, quote);
        }

        let Some(section) = block.section else {
            rows.extend(items.into_iter().map(|item| Row::Instrument {
                item: item.clone(),
                section_id: None,
            }));
            continue;
        };
        if filter.is_active() && items.is_empty() {
            continue;
        }
        rows.push(Row::Section {
            section: section.clone(),
            count: items.len(),
        });
        if section.is_collapsed {
            continue;
        }
        if items.is_empty() {
            rows.push(Row::EmptySection {
                section_id: section.id,
            });
        } else {
            rows.extend(items.into_iter().map(|item| Row::Instrument {
                item: item.clone(),
                section_id: Some(section.id),
            }));
        }
    }
    rows
}

struct Block<'a> {
    section: Option<&'a WatchlistSection>,
    items: Vec<&'a WatchlistInstrument>,
}

/// The root block, then one block per section, in stored order.
fn blocks(list: &Watchlist) -> Vec<Block<'_>> {
    let mut blocks = vec![Block {
        section: None,
        items: Vec::new(),
    }];
    for entry in &list.entries {
        match entry {
            WatchlistEntry::Section(section) => blocks.push(Block {
                section: Some(section),
                items: Vec::new(),
            }),
            WatchlistEntry::Instrument(item) => {
                if let Some(block) = blocks.last_mut() {
                    block.items.push(item);
                }
            }
        }
    }
    blocks
}

/// Filtering matches name, symbol, label, description and provider, ignoring case.
pub fn matches(item: &WatchlistInstrument, filter: &Filter, flags: &Flags) -> bool {
    if let Some(flag) = filter.flag
        && flags.get(&item.instrument.key()) != Some(&flag)
    {
        return false;
    }
    let needle = filter.trimmed_text().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    [
        item.name.as_str(),
        item.label.as_str(),
        item.instrument.symbol.as_str(),
        item.display_name.as_deref().unwrap_or(""),
        item.instrument.source.display_name(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(&needle))
}

#[derive(Clone, Debug, PartialEq)]
enum SortValue {
    Number(f64),
    Text(String),
}

fn compare(a: &SortValue, b: &SortValue) -> Ordering {
    match (a, b) {
        (SortValue::Number(a), SortValue::Number(b)) => a.total_cmp(b),
        (SortValue::Text(a), SortValue::Text(b)) => natural_cmp(a, b),
        (SortValue::Number(_), SortValue::Text(_)) => Ordering::Less,
        (SortValue::Text(_), SortValue::Number(_)) => Ordering::Greater,
    }
}

/// Case-insensitive, with runs of digits compared as numbers ("ETH2" < "ETH10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (mut left, mut right) = (String::new(), String::new());
                while let Some(digit) = a.next_if(char::is_ascii_digit) {
                    left.push(digit);
                }
                while let Some(digit) = b.next_if(char::is_ascii_digit) {
                    right.push(digit);
                }
                let (left, right) = (left.trim_start_matches('0'), right.trim_start_matches('0'));
                let order = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn value_of(
    item: &WatchlistInstrument,
    key: WatchlistSortKey,
    quote: &dyn Fn(&InstrumentId) -> Option<WatchlistQuote>,
) -> Option<SortValue> {
    let number = |pick: fn(&WatchlistQuote) -> Option<f64>| {
        quote(&item.instrument)
            .and_then(|quote| pick(&quote))
            .map(SortValue::Number)
    };
    match key {
        WatchlistSortKey::Manual => None,
        WatchlistSortKey::Symbol => Some(SortValue::Text(item.label.clone())),
        WatchlistSortKey::Last => number(|quote| quote.last),
        WatchlistSortKey::Change => number(|quote| quote.change),
        WatchlistSortKey::ChangePercent => number(|quote| quote.change_percent),
        WatchlistSortKey::Volume => number(|quote| quote.volume),
    }
}

/// Stable, with markets that have no value for the key always last.
fn sorted<'a>(
    items: Vec<&'a WatchlistInstrument>,
    sort: WatchlistSort,
    quote: &dyn Fn(&InstrumentId) -> Option<WatchlistQuote>,
) -> Vec<&'a WatchlistInstrument> {
    let mut keyed: Vec<(usize, &WatchlistInstrument, Option<SortValue>)> = items
        .into_iter()
        .enumerate()
        .map(|(offset, item)| (offset, item, value_of(item, sort.key, quote)))
        .collect();
    keyed.sort_by(|lhs, rhs| match (&lhs.2, &rhs.2) {
        (None, None) => lhs.0.cmp(&rhs.0),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => match compare(a, b) {
            Ordering::Equal => lhs.0.cmp(&rhs.0),
            order if sort.ascending => order,
            order => order.reverse(),
        },
    });
    keyed.into_iter().map(|(_, item, _)| item).collect()
}

// MARK: Drops

/// A drop in terms of stored entries: move `ids`, in this order, to sit before
/// `before` (`None` = the end).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Move {
    pub ids: Vec<EntryId>,
    pub before: Option<EntryId>,
}

/// Turns a native row move (source rows and destination index, both in the rows as
/// shown) into a move of stored entries.
///
/// The stored list is flat, so the destination means exactly what the insertion line
/// showed: before the row at `destination`, whatever section that row is in. Dropped
/// between a section's last symbol and the next heading, a symbol lands before that
/// heading, which is the end of its section; dropped under a heading it lands first in
/// the section. Placeholder rows stand for no entry, so a drop on one resolves to the
/// next real entry. Returns `None` when the drop would change nothing.
pub fn resolve_move(rows: &[Row], sources: &[usize], destination: usize) -> Option<Move> {
    let mut sorted_sources = sources.to_vec();
    sorted_sources.sort_unstable();
    let moved: Vec<EntryId> = sorted_sources
        .iter()
        .filter_map(|index| rows.get(*index).and_then(Row::entry_id))
        .collect();
    if moved.is_empty() {
        return None;
    }
    let moved_set: HashSet<EntryId> = moved.iter().copied().collect();

    let before = rows
        .iter()
        .skip(destination)
        .find_map(|row| row.entry_id().filter(|id| !moved_set.contains(id)));

    let current: Vec<EntryId> = rows.iter().filter_map(Row::entry_id).collect();
    let mut result: Vec<EntryId> = current
        .iter()
        .copied()
        .filter(|id| !moved_set.contains(id))
        .collect();
    let insertion = before
        .and_then(|target| result.iter().position(|id| *id == target))
        .unwrap_or(result.len());
    result.splice(insertion..insertion, moved.iter().copied());
    (result != current).then_some(Move { ids: moved, before })
}

/// A drop while the visible order is derived (a sort or a filter): symbols change
/// section, not position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionMove {
    /// The symbols that are not already in the target section, in displayed order.
    pub ids: Vec<EntryId>,
    /// The target section; `None` is the root, above the first heading.
    pub section: Option<EntryId>,
}

/// Resolves a native row move of *symbols* when the rows on screen are not in stored
/// order. A symbol's place inside a section is decided by the sort, so only its section
/// can change: the section that owns the row just above the insertion line (a symbol's
/// section, a heading's own, an empty-section placeholder's, or the root when the drop
/// is above everything). Symbols already in that section stay put. Headings are not
/// handled here; they move through `resolve_move`, since section order is never sorted.
pub fn resolve_section_move(
    rows: &[Row],
    sources: &[usize],
    destination: usize,
) -> Option<SectionMove> {
    let mut sorted_sources = sources.to_vec();
    sorted_sources.sort_unstable();
    let moved: Vec<(EntryId, Option<EntryId>)> = sorted_sources
        .iter()
        .filter_map(|index| match rows.get(*index) {
            Some(Row::Instrument { item, section_id }) => Some((item.id, *section_id)),
            _ => None,
        })
        .collect();
    if moved.is_empty() {
        return None;
    }
    let above = destination.min(rows.len());
    let target = if above > 0 {
        rows[above - 1].owning_section_id()
    } else {
        None
    };
    let ids: Vec<EntryId> = moved
        .iter()
        .filter(|(_, section)| *section != target)
        .map(|(id, _)| *id)
        .collect();
    (!ids.is_empty()).then_some(SectionMove {
        ids,
        section: target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::watchlist::quote::Freshness;

    fn item(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument::new(
            InstrumentId::new(DataSource::Binance, symbol),
            format!("{symbol} coin"),
            symbol.trim_end_matches("USDT"),
        )
    }

    /// BTC ETH [Majors] SOL XRP [Watching] DOGE [Empty]
    fn sample() -> (Watchlist, Vec<EntryId>) {
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
        ids.push(list.add_section("Empty", None).unwrap().id);
        (list, ids)
    }

    fn no_quotes(_: &InstrumentId) -> Option<WatchlistQuote> {
        None
    }

    fn manual(list: &Watchlist) -> Vec<Row> {
        rows(
            list,
            &Filter::default(),
            &Flags::new(),
            WatchlistSort::MANUAL,
            &no_quotes,
        )
    }

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Section { section, count } => format!("[{} {count}]", section.title),
                Row::Instrument { item, .. } => item.label.clone(),
                Row::EmptySection { .. } => "(empty)".to_owned(),
            })
            .collect()
    }

    fn quote_for(
        prices: &'static [(&'static str, f64, f64)],
    ) -> impl Fn(&InstrumentId) -> Option<WatchlistQuote> {
        move |id: &InstrumentId| {
            prices
                .iter()
                .find(|(symbol, ..)| *symbol == id.symbol)
                .map(|(_, last, percent)| WatchlistQuote {
                    last: Some(*last),
                    change_percent: Some(*percent),
                    freshness: Freshness::Live,
                    ..WatchlistQuote::default()
                })
        }
    }

    #[test]
    fn manual_rows_follow_stored_order_with_counts_and_placeholders() {
        let (list, _) = sample();
        assert_eq!(
            labels(&manual(&list)),
            [
                "BTC",
                "ETH",
                "[Majors 2]",
                "SOL",
                "XRP",
                "[Watching 1]",
                "DOGE",
                "[Empty 0]",
                "(empty)"
            ]
        );
    }

    #[test]
    fn collapsed_sections_hide_rows_but_keep_the_heading_count() {
        let (mut list, ids) = sample();
        list.set_section_collapsed(ids[2], true).unwrap();
        assert_eq!(
            labels(&manual(&list)),
            [
                "BTC",
                "ETH",
                "[Majors 2]",
                "[Watching 1]",
                "DOGE",
                "[Empty 0]",
                "(empty)"
            ]
        );
    }

    #[test]
    fn sorting_orders_inside_sections_and_puts_missing_values_last() {
        let (list, _) = sample();
        let quotes = quote_for(&[
            ("BTCUSDT", 100.0, 1.0),
            ("ETHUSDT", 50.0, 3.0),
            ("SOLUSDT", 10.0, -2.0),
            // XRP has no quote.
        ]);
        let by_percent = |ascending| {
            labels(&rows(
                &list,
                &Filter::default(),
                &Flags::new(),
                WatchlistSort {
                    key: WatchlistSortKey::ChangePercent,
                    ascending,
                },
                &quotes,
            ))
        };
        assert_eq!(
            by_percent(false),
            [
                "ETH",
                "BTC",
                "[Majors 2]",
                "SOL",
                "XRP",
                "[Watching 1]",
                "DOGE",
                "[Empty 0]",
                "(empty)"
            ]
        );
        // Ascending flips the values but the unquoted market is still last.
        assert_eq!(
            by_percent(true),
            [
                "BTC",
                "ETH",
                "[Majors 2]",
                "SOL",
                "XRP",
                "[Watching 1]",
                "DOGE",
                "[Empty 0]",
                "(empty)"
            ]
        );
        // Sorting never touches the stored order.
        assert_eq!(list.entries.len(), 8);
    }

    #[test]
    fn symbol_sort_is_natural_and_case_insensitive() {
        assert_eq!(natural_cmp("eth2", "ETH10"), Ordering::Less);
        assert_eq!(natural_cmp("BTC", "btc"), Ordering::Equal);
        assert_eq!(natural_cmp("BTC", "BTCB"), Ordering::Less);
        assert_eq!(natural_cmp("a007", "a7"), Ordering::Equal);
        let mut list = Watchlist::new("X");
        for symbol in ["ZEC", "ada", "ETH10", "ETH2"] {
            let entry = WatchlistInstrument::new(
                InstrumentId::new(DataSource::Binance, format!("{symbol}USDT")),
                symbol,
                symbol,
            );
            list.add(entry, None).unwrap();
        }
        let sorted = labels(&rows(
            &list,
            &Filter::default(),
            &Flags::new(),
            WatchlistSort {
                key: WatchlistSortKey::Symbol,
                ascending: true,
            },
            &no_quotes,
        ));
        assert_eq!(sorted, ["ada", "ETH2", "ETH10", "ZEC"]);
    }

    #[test]
    fn filter_matches_name_symbol_provider_and_hides_emptied_sections() {
        let (list, _) = sample();
        let by_text = |text: &str| {
            labels(&rows(
                &list,
                &Filter {
                    text: text.to_owned(),
                    flag: None,
                },
                &Flags::new(),
                WatchlistSort::MANUAL,
                &no_quotes,
            ))
        };
        assert_eq!(by_text("btc"), ["BTC"]);
        assert_eq!(by_text("  SOLUSDT COIN "), ["[Majors 1]", "SOL"]);
        assert_eq!(
            by_text("binance").len(),
            5 + 2,
            "all five markets and the two sections that still hold one"
        );
        assert!(by_text("nothing").is_empty());
    }

    #[test]
    fn flag_filter_reads_the_global_flag_map() {
        let (list, _) = sample();
        let mut flags = Flags::new();
        flags.insert(
            InstrumentId::new(DataSource::Binance, "xrpusdt").key(),
            WatchlistFlag::Red,
        );
        let filtered = rows(
            &list,
            &Filter {
                text: String::new(),
                flag: Some(WatchlistFlag::Red),
            },
            &flags,
            WatchlistSort::MANUAL,
            &no_quotes,
        );
        assert_eq!(labels(&filtered), ["[Majors 1]", "XRP"]);
    }

    #[test]
    fn dropping_a_symbol_resolves_to_a_stored_move() {
        let (list, ids) = sample();
        let rows = manual(&list);
        // rows: 0 BTC 1 ETH 2 [Majors] 3 SOL 4 XRP 5 [Watching] 6 DOGE 7 [Empty] 8 (empty)
        // BTC dropped between SOL and XRP.
        assert_eq!(
            resolve_move(&rows, &[0], 4),
            Some(Move {
                ids: vec![ids[0]],
                before: Some(ids[4])
            })
        );
        // Dropped just before the next heading: end of Majors.
        assert_eq!(
            resolve_move(&rows, &[0], 5),
            Some(Move {
                ids: vec![ids[0]],
                before: Some(ids[5])
            })
        );
        // Dropped under a heading: first in the section.
        assert_eq!(
            resolve_move(&rows, &[0], 3).map(|m| m.before),
            Some(Some(ids[3]))
        );
        // Dropped on the empty placeholder resolves to "the end", nothing follows it.
        assert_eq!(
            resolve_move(&rows, &[0], 8),
            Some(Move {
                ids: vec![ids[0]],
                before: None
            })
        );
        // Dropped where it already is: nothing to do.
        assert_eq!(resolve_move(&rows, &[0], 0), None);
        assert_eq!(resolve_move(&rows, &[0], 1), None);
        assert_eq!(resolve_move(&rows, &[], 3), None);
        // A placeholder alone is not movable.
        assert_eq!(resolve_move(&rows, &[8], 0), None);
    }

    #[test]
    fn resolved_moves_apply_to_the_stored_list() {
        let (mut list, _) = sample();
        let rows = manual(&list);
        let drop = resolve_move(&rows, &[0, 1], 5).unwrap();
        list.move_entries(&drop.ids, drop.before).unwrap();
        assert_eq!(
            labels(&manual(&list)),
            [
                "[Majors 4]",
                "SOL",
                "XRP",
                "BTC",
                "ETH",
                "[Watching 1]",
                "DOGE",
                "[Empty 0]",
                "(empty)"
            ]
        );
    }

    #[test]
    fn sorted_or_filtered_drops_only_change_section() {
        let (list, ids) = sample();
        let rows = manual(&list);
        // BTC (root) dropped under the Watching heading (above row 6) -> Watching.
        assert_eq!(
            resolve_section_move(&rows, &[0], 6),
            Some(SectionMove {
                ids: vec![ids[0]],
                section: Some(ids[5])
            })
        );
        // Dropped above everything: root. BTC and ETH are already there; SOL is not.
        assert_eq!(
            resolve_section_move(&rows, &[0, 1, 3], 0),
            Some(SectionMove {
                ids: vec![ids[3]],
                section: None
            })
        );
        // Dropped onto the empty section's placeholder.
        assert_eq!(
            resolve_section_move(&rows, &[0], 9).map(|m| m.section),
            Some(Some(ids[7]))
        );
        // Same section: nothing.
        assert_eq!(resolve_section_move(&rows, &[3], 5), None);
        // Headings are not handled here.
        assert_eq!(resolve_section_move(&rows, &[2], 0), None);
    }

    #[test]
    fn row_keys_are_distinct_including_placeholders() {
        let (list, _) = sample();
        let keys: HashSet<String> = manual(&list).iter().map(Row::key).collect();
        assert_eq!(keys.len(), 9);
    }
}
