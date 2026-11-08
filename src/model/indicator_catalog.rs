//! The one list of indicators the add-indicator UIs draw from: the native built-ins, the Pine
//! indicators bundled with the app, and the user's saved scripts. Pure data and search; nothing
//! here draws or touches storage.

use super::BuiltIn;
use crate::pine_studio::indicators::{self, BundledIndicator};
use crate::pine_studio::model::{LocalScript, ScriptType};
use std::sync::Arc;

/// What an indicator measures. The picker's "Families" sidebar and its counts.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Family {
    Trend,
    Momentum,
    Volatility,
    VolumeFlow,
    Structure,
}

impl Family {
    #[cfg(test)]
    pub const ALL: [Self; 5] = [
        Self::Trend,
        Self::Momentum,
        Self::Volatility,
        Self::VolumeFlow,
        Self::Structure,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Trend => "Trend",
            Self::Momentum => "Momentum",
            Self::Volatility => "Volatility",
            Self::VolumeFlow => "Volume & Flow",
            Self::Structure => "Structure",
        }
    }
}

/// Where an indicator comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// Computed natively and drawn by the chart (`BuiltIn`).
    Native,
    /// A Pine script compiled into the app, always available.
    Bundled,
    /// A script the user saved.
    Script,
}

/// A stable identity for an indicator: tells one row from another.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum EntryKey {
    Native(BuiltIn),
    Bundled(&'static str),
    Script(String),
}

/// One row the add-indicator UIs can show and add.
#[derive(Clone, Debug)]
#[expect(
    dead_code,
    reason = "the indicator picker (next change) reads the detail fields"
)]
pub struct CatalogEntry {
    pub key: EntryKey,
    pub title: String,
    pub summary: String,
    pub description: String,
    pub family: Option<Family>,
    pub aliases: Vec<String>,
    /// Draws on the price pane (`true`) or in a pane of its own.
    pub overlay: bool,
    pub origin: Origin,
    pub author: &'static str,
    pub license: &'static str,
    pub script: Option<Arc<LocalScript>>,
}

impl PartialEq for CatalogEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl CatalogEntry {
    /// Scripts and strategies keep to market charts; only a native one runs on a line chart.
    pub fn runs_on_line_charts(&self) -> bool {
        match &self.key {
            EntryKey::Native(kind) => kind.supports_line_charts(),
            _ => false,
        }
    }

    fn native(kind: BuiltIn) -> Self {
        let (family, summary, description, aliases): (_, _, _, &[&str]) = match kind {
            BuiltIn::Volume => (
                Family::VolumeFlow,
                "Traded volume under the candles.",
                "Bars of the volume traded in each candle, coloured by whether the candle closed up or down. Not available on line charts, which carry no volume.",
                &["vol"],
            ),
            BuiltIn::Rsi => (
                Family::Momentum,
                "Relative Strength Index, 14 periods.",
                "Wilder's momentum oscillator, from 0 to 100, with the 70 and 30 levels drawn. Above 70 is conventionally overbought and below 30 oversold.",
                &["relative strength index", "rsi 14"],
            ),
            BuiltIn::Ema => (
                Family::Trend,
                "Exponential moving average of the close.",
                "A moving average that weights recent closes more. Choose its period from the chip on the chart (9, 20, 50, 100 or 200).",
                &["exponential moving average", "moving average", "ma"],
            ),
            BuiltIn::Bollinger => (
                Family::Volatility,
                "Bollinger Bands, 20 periods and 2 deviations.",
                "A 20-period average with bands two standard deviations above and below. The bands widen when price is volatile and narrow when it is quiet.",
                &["bbands", "bb", "bollinger bands"],
            ),
            BuiltIn::TrendFlips => (
                Family::Trend,
                "Marks where a Supertrend (10, 3) changes direction.",
                "Computes a Supertrend with an ATR length of 10 and a multiplier of 3 and marks each bar where its direction flips, up or down.",
                &["supertrend", "trend flip", "reversal"],
            ),
        };
        Self {
            key: EntryKey::Native(kind),
            title: kind.menu_title().to_owned(),
            summary: summary.to_owned(),
            description: description.to_owned(),
            family: Some(family),
            aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            overlay: kind.is_overlay(),
            origin: Origin::Native,
            author: "DegenView",
            license: "GPL-3.0-only",
            script: None,
        }
    }

    fn bundled(indicator: &'static BundledIndicator) -> Self {
        Self {
            key: EntryKey::Bundled(indicator.id),
            title: indicator.name.to_owned(),
            summary: indicator.summary.to_owned(),
            description: indicator.description.to_owned(),
            family: Some(indicator.family),
            aliases: indicator
                .aliases
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect(),
            overlay: indicator.overlay,
            origin: Origin::Bundled,
            author: indicator.author,
            license: indicator.license,
            script: None,
        }
    }

    fn script(script: &Arc<LocalScript>) -> Self {
        let kind = match script.script_type {
            ScriptType::Strategy => "strategy",
            _ => "indicator",
        };
        Self {
            key: EntryKey::Script(script.id.clone()),
            title: script.name.clone(),
            summary: format!("Your saved Pine {kind}."),
            description: format!("A Pine {kind} from your Scripts folder."),
            family: None,
            aliases: Vec::new(),
            overlay: false,
            origin: Origin::Script,
            author: "You",
            license: "",
            script: Some(script.clone()),
        }
    }

    /// 0 when `needle` (already lower case, non-empty) is the start of the title, 1 when it is
    /// inside it or an alias, `None` when it matches nothing.
    fn rank(&self, needle: &str) -> Option<u8> {
        let title = self.title.to_lowercase();
        if title.starts_with(needle) {
            Some(0)
        } else if title.contains(needle)
            || self
                .aliases
                .iter()
                .any(|alias| alias.to_lowercase().contains(needle))
        {
            Some(1)
        } else {
            None
        }
    }
}

/// Every native built-in, then every bundled Pine indicator, then the saved scripts, each in its
/// own order.
pub fn all(scripts: &[Arc<LocalScript>]) -> Vec<CatalogEntry> {
    let mut found: Vec<CatalogEntry> = BuiltIn::ALL.into_iter().map(CatalogEntry::native).collect();
    found.extend(indicators::ALL.iter().map(CatalogEntry::bundled));
    found.extend(scripts.iter().map(CatalogEntry::script));
    found
}

/// The entries matching `query` (case-insensitive, surrounding spaces ignored): titles that start
/// with it first, then titles or aliases that contain it. An empty query keeps everything.
pub fn search(entries: &[CatalogEntry], query: &str) -> Vec<CatalogEntry> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return entries.to_vec();
    }
    let mut ranked: Vec<(u8, &CatalogEntry)> = entries
        .iter()
        .filter_map(|entry| entry.rank(&needle).map(|rank| (rank, entry)))
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, entry)| entry.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::script_catalog::sample;

    fn scripts() -> Vec<Arc<LocalScript>> {
        vec![Arc::new(sample(
            "a",
            "Squeeze Momentum",
            ScriptType::Indicator,
        ))]
    }

    #[test]
    fn natives_come_first_then_bundled_then_scripts() {
        let found = all(&scripts());
        let origins: Vec<Origin> = found.iter().map(|entry| entry.origin).collect();
        let first_bundled = origins.iter().position(|o| *o == Origin::Bundled).unwrap();
        let first_script = origins.iter().position(|o| *o == Origin::Script).unwrap();
        assert_eq!(first_bundled, BuiltIn::ALL.len());
        assert_eq!(first_script, BuiltIn::ALL.len() + indicators::ALL.len());
        assert_eq!(found.len(), first_script + 1);
    }

    #[test]
    fn every_family_has_an_indicator() {
        let found = all(&[]);
        for family in Family::ALL {
            assert!(
                found.iter().any(|entry| entry.family == Some(family)),
                "{} is empty",
                family.title()
            );
        }
    }

    #[test]
    fn search_ranks_prefixes_before_aliases_and_finds_by_alias() {
        let found = all(&[]);
        let titles = |query: &str| -> Vec<String> {
            search(&found, query).into_iter().map(|e| e.title).collect()
        };
        // "macd" is a prefix of a title; "supertrend" is only an alias of Trend Flips.
        assert_eq!(titles("macd"), ["MACD"]);
        assert!(titles("supertrend").contains(&"Trend Flips".to_owned()));
        assert!(titles("zzzz").is_empty());
        let rsi = titles("rsi");
        assert_eq!(rsi[0], "RSI", "{rsi:?}");
        assert!(rsi.contains(&"Stochastic RSI".to_owned()), "{rsi:?}");
        assert_eq!(search(&found, "   ").len(), found.len());
    }

    #[test]
    fn only_native_entries_run_on_line_charts_and_not_volume() {
        let found = all(&scripts());
        for entry in &found {
            let expected = matches!(&entry.key, EntryKey::Native(kind) if *kind != BuiltIn::Volume);
            assert_eq!(entry.runs_on_line_charts(), expected, "{}", entry.title);
        }
    }
}
