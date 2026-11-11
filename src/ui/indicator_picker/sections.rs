//! What the picker lists for a sidebar choice. Pure: entries in, blocks of entries out.

use crate::model::indicators::catalog::{CatalogEntry, EntryKey, Family, Origin};
use crate::ui::icons::Icon;
use std::collections::HashSet;

/// One sidebar row that lists indicators.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Section {
    Favorites,
    MyScripts,
    /// Built-in indicators that draw on the price pane.
    Overlays,
    /// Built-in indicators with a pane of their own.
    Panes,
    Discover,
    All,
    Family(Family),
}

/// The sidebar's groups, in order. "Open Pine Studio" is an action row after Yours' sections.
pub const YOURS: [Section; 2] = [Section::Favorites, Section::MyScripts];
pub const BUILT_IN: [Section; 2] = [Section::Overlays, Section::Panes];
pub const LIBRARY: [Section; 2] = [Section::Discover, Section::All];

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::Favorites => "Favorites",
            Self::MyScripts => "My scripts",
            Self::Overlays => "Overlays",
            Self::Panes => "Separate pane",
            Self::Discover => "Discover",
            Self::All => "All indicators",
            Self::Family(family) => family.title(),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Favorites => Icon::Star,
            Self::MyScripts => Icon::PersonCircle,
            Self::Overlays => Icon::ChartUptrend,
            Self::Panes => Icon::Waveform,
            Self::Discover => Icon::Sparkles,
            Self::All => Icon::Layers,
            Self::Family(Family::Trend) => Icon::LineTrend,
            Self::Family(Family::Momentum) => Icon::Gauge,
            Self::Family(Family::Volatility) => Icon::Bands,
            Self::Family(Family::VolumeFlow) => Icon::BarChart,
            Self::Family(Family::Structure) => Icon::Diamond,
        }
    }

    /// What the grid says when this section has nothing in it.
    pub fn empty_message(self) -> &'static str {
        match self {
            Self::Favorites => "No favorites yet. Star an indicator to keep it here.",
            Self::MyScripts => "No saved scripts yet. Open Pine Studio to write one.",
            _ => "Nothing here.",
        }
    }

    /// Whether `entry` belongs in this section (Discover is handled by [`blocks`]).
    fn contains(self, entry: &CatalogEntry, favorites: &HashSet<String>) -> bool {
        let built_in = entry.origin != Origin::Script;
        match self {
            Self::Favorites => favorites.contains(&entry.key.stored()),
            Self::MyScripts => entry.origin == Origin::Script,
            Self::Overlays => built_in && entry.overlay,
            Self::Panes => built_in && !entry.overlay,
            Self::Discover => built_in,
            Self::All => true,
            Self::Family(family) => entry.family == Some(family),
        }
    }
}

/// A run of cards under an optional heading.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub heading: Option<&'static str>,
    pub entries: Vec<CatalogEntry>,
}

/// The stored keys of the indicators Discover leads with, when they exist.
const POPULAR: [&str; 8] = [
    "native:rsi",
    "native:ema",
    "native:bollinger",
    "bundled:macd",
    "native:volume",
    "bundled:ichimoku-cloud",
    "bundled:vwap",
    "bundled:atr",
];

/// A stable pseudo-random order for a seed (FNV-1a over the seed and the key), so a shuffle is
/// repeatable and needs no random source.
fn scramble(seed: u64, key: &EntryKey) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for byte in key.stored().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// What the grid lists for `section`. Discover is a "Popular" row, then the rest in the order
/// `shuffle` gives; every other section is one block without a heading.
pub fn blocks(
    section: Section,
    all: &[CatalogEntry],
    favorites: &HashSet<String>,
    shuffle: u64,
) -> Vec<Block> {
    if section != Section::Discover {
        return vec![Block {
            heading: None,
            entries: all
                .iter()
                .filter(|entry| section.contains(entry, favorites))
                .cloned()
                .collect(),
        }];
    }
    let popular: Vec<CatalogEntry> = POPULAR
        .iter()
        .filter_map(|stored| all.iter().find(|entry| entry.key.stored() == *stored))
        .cloned()
        .collect();
    let mut rest: Vec<CatalogEntry> = all
        .iter()
        .filter(|entry| entry.origin != Origin::Script && !popular.contains(entry))
        .cloned()
        .collect();
    rest.sort_by_key(|entry| scramble(shuffle, &entry.key));
    vec![
        Block {
            heading: Some("Popular"),
            entries: popular,
        },
        Block {
            heading: Some("More to explore"),
            entries: rest,
        },
    ]
}

/// The results of a search across everything, as one block.
pub fn search_blocks(all: &[CatalogEntry], query: &str) -> Vec<Block> {
    vec![Block {
        heading: Some("Results"),
        entries: crate::model::indicators::catalog::search(all, query),
    }]
}

/// The number beside a sidebar row (none for Discover).
pub fn count(section: Section, all: &[CatalogEntry], favorites: &HashSet<String>) -> Option<usize> {
    (section != Section::Discover).then(|| {
        all.iter()
            .filter(|entry| section.contains(entry, favorites))
            .count()
    })
}

/// Every entry of the blocks, in the order the keyboard walks them.
pub fn flatten(blocks: &[Block]) -> Vec<&CatalogEntry> {
    blocks.iter().flat_map(|block| &block.entries).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::indicators::catalog::all;
    use crate::pine_studio::model::{LocalScript, ScriptType};
    use crate::ui::script_catalog::sample;
    use std::sync::Arc;

    fn catalog() -> Vec<CatalogEntry> {
        let scripts: Vec<Arc<LocalScript>> =
            vec![Arc::new(sample("s1", "My Script", ScriptType::Indicator))];
        all(&scripts)
    }

    fn titles(blocks: &[Block]) -> Vec<&str> {
        flatten(blocks)
            .into_iter()
            .map(|entry| entry.title.as_str())
            .collect()
    }

    #[test]
    fn overlays_and_panes_split_the_built_ins_and_leave_scripts_out() {
        let all = catalog();
        let none = HashSet::new();
        let overlays = blocks(Section::Overlays, &all, &none, 1);
        let panes = blocks(Section::Panes, &all, &none, 1);
        let built_ins = all
            .iter()
            .filter(|entry| entry.origin != Origin::Script)
            .count();
        assert_eq!(
            overlays[0].entries.len() + panes[0].entries.len(),
            built_ins
        );
        assert!(titles(&overlays).contains(&"Ichimoku Cloud"));
        assert!(titles(&panes).contains(&"MACD"));
        assert!(!titles(&panes).contains(&"My Script"));
        assert_eq!(
            titles(&blocks(Section::MyScripts, &all, &none, 1)),
            ["My Script"]
        );
    }

    #[test]
    fn a_family_section_lists_only_that_family() {
        let all = catalog();
        let momentum = blocks(Section::Family(Family::Momentum), &all, &HashSet::new(), 1);
        assert!(titles(&momentum).contains(&"RSI"));
        assert!(!titles(&momentum).contains(&"ATR"));
        for entry in flatten(&momentum) {
            assert_eq!(entry.family, Some(Family::Momentum));
        }
    }

    #[test]
    fn favorites_list_what_was_starred_and_ignore_the_rest() {
        let all = catalog();
        let favorites: HashSet<String> = ["bundled:macd", "script:s1", "bundled:gone"]
            .iter()
            .map(|key| (*key).to_owned())
            .collect();
        let found = blocks(Section::Favorites, &all, &favorites, 1);
        assert_eq!(titles(&found), ["MACD", "My Script"]);
        assert_eq!(count(Section::Favorites, &all, &favorites), Some(2));
        assert!(
            blocks(Section::Favorites, &all, &HashSet::new(), 1)[0]
                .entries
                .is_empty()
        );
    }

    #[test]
    fn discover_leads_with_popular_and_shuffles_the_rest_repeatably() {
        let all = catalog();
        let none = HashSet::new();
        let first = blocks(Section::Discover, &all, &none, 1);
        assert_eq!(first[0].heading, Some("Popular"));
        assert_eq!(first[0].entries.len(), POPULAR.len());
        assert_eq!(first, blocks(Section::Discover, &all, &none, 1));
        let other = blocks(Section::Discover, &all, &none, 2);
        assert_ne!(
            titles(&first[1..]),
            titles(&other[1..]),
            "a new seed reorders"
        );
        // Nothing is listed twice, and saved scripts are not "discovered".
        let listed = titles(&first);
        let unique: HashSet<_> = listed.iter().collect();
        assert_eq!(unique.len(), listed.len());
        assert!(!listed.contains(&"My Script"));
        assert_eq!(count(Section::Discover, &all, &none), None);
    }

    #[test]
    fn the_all_count_is_every_entry_and_search_spans_every_section() {
        let all = catalog();
        assert_eq!(count(Section::All, &all, &HashSet::new()), Some(all.len()));
        let found = search_blocks(&all, "script");
        assert_eq!(titles(&found), ["My Script"]);
    }
}
