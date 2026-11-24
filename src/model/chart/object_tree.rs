//! The Object Tree's rows: the chart's symbol, the markets compared on it, its indicators and
//! scripts, and its drawings.
//!
//! A view over state the chart already owns (`IndicatorConfig`, the Pine instances, the compared
//! markets, the drawing list), rebuilt from it; nothing here is stored. Drawings are listed topmost first, the order
//! the chart paints them back to front reversed, so the row on top is the one drawn on top.
//! Pure: the panel draws the rows and the workspace applies what the user does to them.

use crate::i18n::{tr, tr_c};
use crate::model::drawings::Drawing;
use crate::model::overlay::{OverlayId, OverlayList};
use crate::model::pine::instance::PineInstance;
use crate::model::{BuiltIn, IndicatorConfig, Tool};

/// An indicator row's identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorKey {
    BuiltIn(BuiltIn),
    /// A script instance, by its id on the chart.
    Script(u64),
}

/// What a row stands for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Heading(Section),
    Symbol,
    /// A market compared on the chart, by its id on the chart.
    Overlay(OverlayId),
    Indicator(IndicatorKey),
    Drawing(u64),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Section {
    Symbol,
    Overlays,
    Indicators,
    Drawings,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::Symbol => tr!("Symbol"),
            Self::Overlays => tr!("Compared symbols"),
            Self::Indicators => tr!("Indicators"),
            Self::Drawings => tr!("Drawings"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: Key,
    pub label: String,
    pub detail: String,
    /// `Some` when the row can be hidden: `true` while it shows.
    pub shown: Option<bool>,
    /// `Some` when the row can be locked: `true` while it is.
    pub locked: Option<bool>,
    /// The tool that drew a drawing, for its icon.
    pub tool: Option<Tool>,
    /// On a heading, how many rows its section holds (before any filter).
    pub count: usize,
}

/// What the tree lists.
pub struct Inputs<'a> {
    pub title: &'a str,
    pub source: &'a str,
    pub indicators: &'a IndicatorConfig,
    pub scripts: &'a [PineInstance],
    /// The markets compared on the chart, in the order they are drawn.
    pub overlays: &'a OverlayList,
    pub drawings: &'a [Drawing],
}

/// The rows for `filter` (empty lists everything; otherwise a case-insensitive match on the
/// label or detail, with only the headings that still have rows).
pub fn rows(inputs: &Inputs, filter: &str) -> Vec<Row> {
    let needle = filter.trim().to_lowercase();
    let matches = |row: &Row| {
        needle.is_empty()
            || row.label.to_lowercase().contains(&needle)
            || row.detail.to_lowercase().contains(&needle)
    };
    let heading = |section: Section, count: usize| Row {
        key: Key::Heading(section),
        label: section.title().to_owned(),
        detail: String::new(),
        shown: None,
        locked: None,
        tool: None,
        count,
    };

    let mut sections: Vec<(Section, Vec<Row>)> = Vec::new();
    sections.push((
        Section::Symbol,
        vec![Row {
            key: Key::Symbol,
            label: inputs.title.to_owned(),
            detail: inputs.source.to_owned(),
            shown: None,
            locked: None,
            tool: None,
            count: 0,
        }],
    ));

    // The compared markets, in the order the list holds them (the legend's order too).
    let overlays: Vec<Row> = inputs
        .overlays
        .iter()
        .map(|overlay| Row {
            key: Key::Overlay(overlay.id),
            label: overlay.name(),
            detail: overlay.instrument.source.display_name().to_owned(),
            shown: Some(overlay.visible),
            locked: None,
            tool: None,
            count: 0,
        })
        .collect();
    sections.push((Section::Overlays, overlays));

    let mut indicators = Vec::new();
    for indicator in BuiltIn::ALL
        .into_iter()
        .filter(|indicator| indicator.applied(inputs.indicators))
    {
        indicators.push(Row {
            key: Key::Indicator(IndicatorKey::BuiltIn(indicator)),
            label: indicator.chip_title(inputs.indicators),
            detail: indicator.display_title().to_owned(),
            shown: Some(!indicator.hidden(inputs.indicators)),
            locked: None,
            tool: None,
            count: 0,
        });
    }
    for script in inputs.scripts {
        indicators.push(Row {
            key: Key::Indicator(IndicatorKey::Script(script.id)),
            label: script.name.clone(),
            detail: tr!("Script").to_owned(),
            shown: Some(script.visible),
            locked: None,
            tool: None,
            count: 0,
        });
    }
    sections.push((Section::Indicators, indicators));

    // Topmost first, numbered per tool from the bottom so a name stays with its drawing. A drawing
    // this version cannot read (a tool, or a field, from a newer build) is listed too, so it is not
    // just invisible: it can be deleted, nothing else.
    let mut counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    let mut unreadable = 0;
    let mut drawings: Vec<Row> = inputs
        .drawings
        .iter()
        .filter_map(|drawing| {
            if let Drawing::Unreadable(_) = drawing {
                // A row that is not even a drawing (no id) is not an object.
                let id = drawing.stored_id();
                unreadable += 1;
                return (id != 0).then(|| Row {
                    key: Key::Drawing(id),
                    label: tr!("From a newer version {number}", number = unreadable),
                    detail: tr!("Update DegenView to see it").to_owned(),
                    shown: None,
                    locked: None,
                    tool: None,
                    count: 0,
                });
            }
            if drawing.tool().is_none() || drawing.is_ruler() {
                return None;
            }
            let name = drawing.tool().map_or(tr!("Drawing"), |tool| tool.label());
            let number = counts.entry(name).or_insert(0);
            *number += 1;
            Some(Row {
                key: Key::Drawing(drawing.id()),
                label: tr_c!(
                    "a numbered drawing, such as Trend Line 3",
                    "{name} {number}",
                    name = name,
                    number = number
                ),
                detail: drawing
                    .summary()
                    .unwrap_or_else(|| drawing.text().to_owned()),
                shown: Some(!drawing.hidden()),
                locked: Some(drawing.locked()),
                tool: drawing.tool(),
                count: 0,
            })
        })
        .collect();
    drawings.reverse();
    sections.push((Section::Drawings, drawings));

    let mut out = Vec::new();
    for (section, rows) in sections {
        let total = rows.len();
        let rows: Vec<Row> = rows.into_iter().filter(|row| matches(row)).collect();
        if rows.is_empty() {
            continue;
        }
        out.push(heading(section, total));
        out.extend(rows);
    }
    out
}

// MARK: Reordering

/// What a row can be moved among: rows of one kind are one stack. Built-in indicators have a fixed
/// order, so they are in none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stack {
    Overlays,
    Scripts,
    Drawings,
}

fn stack_of(key: Key) -> Option<Stack> {
    match key {
        Key::Overlay(_) => Some(Stack::Overlays),
        Key::Indicator(IndicatorKey::Script(_)) => Some(Stack::Scripts),
        Key::Drawing(_) => Some(Stack::Drawings),
        Key::Heading(_) | Key::Symbol | Key::Indicator(IndicatorKey::BuiltIn(_)) => None,
    }
}

/// Whether the user can pick the row up to reorder it. A drawing from a newer version (it has no
/// tool) stays where it is, as in the row menu.
pub fn draggable(row: &Row) -> bool {
    stack_of(row.key).is_some() && (!matches!(row.key, Key::Drawing(_)) || row.tool.is_some())
}

/// The rows `key` can be dropped among, in the order the tree shows them (itself included).
pub fn siblings(rows: &[Row], key: Key) -> Vec<Key> {
    let Some(stack) = stack_of(key) else {
        return Vec::new();
    };
    rows.iter()
        .map(|row| row.key)
        .filter(|candidate| stack_of(*candidate) == Some(stack))
        .collect()
}

/// The row `from` takes the place of when it is dropped into insertion gap `gap` (0 is above the
/// first sibling, `siblings.len()` below the last; a larger gap counts as the last). `None` when
/// the drop leaves it where it is: its own two gaps, or a row that is not among `siblings`.
pub fn drop_over(siblings: &[Key], from: Key, gap: usize) -> Option<Key> {
    let from_index = siblings.iter().position(|key| *key == from)?;
    let gap = gap.min(siblings.len());
    let to = if gap > from_index { gap - 1 } else { gap };
    (to != from_index).then(|| siblings[to])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Anchor;

    fn line(id: u64) -> Drawing {
        Drawing::build(
            crate::model::Tool::TrendLine,
            id,
            &[
                Anchor {
                    time_ms: 0,
                    price: 1.0,
                },
                Anchor {
                    time_ms: 1,
                    price: 2.0,
                },
            ],
            crate::model::Tool::TrendLine.default_style(),
        )
        .expect("a trend line")
    }

    fn inputs<'a>(
        config: &'a IndicatorConfig,
        scripts: &'a [PineInstance],
        drawings: &'a [Drawing],
    ) -> Inputs<'a> {
        static NO_OVERLAYS: std::sync::OnceLock<OverlayList> = std::sync::OnceLock::new();
        Inputs {
            title: "BTCUSDT",
            source: "Binance",
            indicators: config,
            scripts,
            overlays: NO_OVERLAYS.get_or_init(OverlayList::new),
            drawings,
        }
    }

    fn labels(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn an_empty_chart_lists_only_its_symbol() {
        let config = IndicatorConfig::default();
        let rows = rows(&inputs(&config, &[], &[]), "");
        assert_eq!(labels(&rows), ["Symbol", "BTCUSDT"]);
        assert!(matches!(rows[0].key, Key::Heading(Section::Symbol)));
        assert_eq!(rows[1].key, Key::Symbol);
    }

    #[test]
    fn indicators_scripts_and_drawings_are_listed_in_sections() {
        let mut config = IndicatorConfig::default();
        BuiltIn::Ema.set_applied(&mut config, true);
        BuiltIn::Bollinger.set_applied(&mut config, true);
        BuiltIn::Bollinger.toggle_visible(&mut config);
        let scripts = [PineInstance::new(3, None, "My script", "//@version=6")];
        let drawings = [line(1), line(2)];
        let rows = rows(&inputs(&config, &scripts, &drawings), "");
        assert_eq!(
            labels(&rows),
            [
                "Symbol",
                "BTCUSDT",
                "Indicators",
                "EMA 20",
                "BB",
                "My script",
                "Drawings",
                "Trend line 2",
                "Trend line 1",
            ]
        );
        let bands = rows.iter().find(|row| row.label == "BB").unwrap();
        assert_eq!(bands.shown, Some(false), "hidden by its eye");
        let script = rows.iter().find(|row| row.label == "My script").unwrap();
        assert_eq!(script.key, Key::Indicator(IndicatorKey::Script(3)));
        assert_eq!(script.shown, Some(true));
    }

    #[test]
    fn compared_markets_are_listed_after_the_symbol_in_the_order_the_list_holds_them() {
        use crate::model::overlay::OverlayCommand;
        use crate::model::{DataSource, InstrumentId};
        let config = IndicatorConfig::default();
        let main = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let mut overlays = OverlayList::new();
        let eth = overlays.add(
            &main,
            InstrumentId::new(DataSource::Binance, "ETHUSDT"),
            None,
        );
        let sol = overlays.add(
            &main,
            InstrumentId::new(DataSource::Binance, "SOLUSDT"),
            Some("Solana".into()),
        );
        let (
            crate::model::overlay::AddOutcome::Added(eth),
            crate::model::overlay::AddOutcome::Added(sol),
        ) = (eth, sol)
        else {
            panic!("both added");
        };
        overlays.apply(OverlayCommand::SetVisible(sol, false));
        let drawings = [line(1)];
        let mut input = inputs(&config, &[], &drawings);
        input.overlays = &overlays;
        let rows = rows(&input, "");
        assert_eq!(
            labels(&rows),
            [
                "Symbol",
                "BTCUSDT",
                "Compared symbols",
                "ETHUSDT",
                "Solana",
                "Drawings",
                "Trend line 1",
            ]
        );
        let row = |key| rows.iter().find(|row| row.key == key).unwrap();
        // Hidden follows the overlay's eye; there is no lock and no tool.
        assert_eq!(row(Key::Overlay(eth)).shown, Some(true));
        assert_eq!(row(Key::Overlay(sol)).shown, Some(false));
        assert_eq!(row(Key::Overlay(sol)).locked, None);
        assert_eq!(row(Key::Overlay(sol)).detail, "Binance");
        assert_eq!(row(Key::Heading(Section::Overlays)).count, 2);
        // The filter finds one by its title or its market, and drops the other sections.
        let found = super::rows(&input, "solana");
        assert_eq!(labels(&found), ["Compared symbols", "Solana"]);
    }

    #[test]
    fn drawings_run_topmost_first_and_keep_their_numbers() {
        let config = IndicatorConfig::default();
        let drawings = [line(10), line(20), line(30)];
        let rows = rows(&inputs(&config, &[], &drawings), "");
        let keys: Vec<Key> = rows
            .iter()
            .filter(|row| matches!(row.key, Key::Drawing(_)))
            .map(|row| row.key)
            .collect();
        assert_eq!(keys, [Key::Drawing(30), Key::Drawing(20), Key::Drawing(10)]);
        // The bottom drawing is number 1 whatever the order shown.
        let bottom = rows.iter().find(|row| row.key == Key::Drawing(10)).unwrap();
        assert_eq!(bottom.label, "Trend line 1");
    }

    #[test]
    fn a_hidden_or_locked_drawing_says_so() {
        let config = IndicatorConfig::default();
        let mut hidden = line(1);
        hidden.set_hidden(true);
        let mut locked = line(2);
        locked.set_locked(true);
        let drawings = [hidden, locked];
        let rows = rows(&inputs(&config, &[], &drawings), "");
        let first = rows.iter().find(|r| r.key == Key::Drawing(1)).unwrap();
        let second = rows.iter().find(|r| r.key == Key::Drawing(2)).unwrap();
        assert_eq!((first.shown, first.locked), (Some(false), Some(false)));
        assert_eq!((second.shown, second.locked), (Some(true), Some(true)));
    }

    #[test]
    fn a_drawing_from_a_newer_version_is_listed_by_the_id_it_was_saved_with_and_nothing_else() {
        let config = IndicatorConfig::default();
        let future = Drawing::Unreadable(serde_json::json!({"Spiral": {"id": 9}}));
        let drawings = [line(1), future, line(2)];
        let rows = rows(&inputs(&config, &[], &drawings), "");
        let row = rows
            .iter()
            .find(|row| row.key == Key::Drawing(9))
            .expect("listed");
        assert_eq!(row.label, "From a newer version 1");
        assert_eq!(row.detail, "Update DegenView to see it");
        // It cannot be hidden, locked, restyled or given a tool's icon: only deleted.
        assert_eq!((row.shown, row.locked, row.tool), (None, None, None));
        // It sits where it was stacked, and the drawings around it are numbered as before.
        let order: Vec<Key> = rows
            .iter()
            .filter(|row| matches!(row.key, Key::Drawing(_)))
            .map(|row| row.key)
            .collect();
        assert_eq!(order, [Key::Drawing(2), Key::Drawing(9), Key::Drawing(1)]);
        let heading = rows
            .iter()
            .find(|row| row.key == Key::Heading(Section::Drawings))
            .unwrap();
        assert_eq!(heading.count, 3);
        // The filter finds it by what it says.
        let found = super::rows(&inputs(&config, &[], &drawings), "newer version");
        assert_eq!(labels(&found), ["Drawings", "From a newer version 1"]);
    }

    #[test]
    fn rulers_and_rows_that_are_not_drawings_are_not_objects() {
        let config = IndicatorConfig::default();
        let ruler = Drawing::Ruler {
            id: 1,
            a: Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: Anchor {
                time_ms: 1,
                price: 2.0,
            },
        };
        // No id: not a drawing a newer version made, just something stored.
        let junk = Drawing::Unreadable(serde_json::json!("garbage"));
        let drawings = [ruler, junk];
        let rows = rows(&inputs(&config, &[], &drawings), "");
        assert_eq!(labels(&rows), ["Symbol", "BTCUSDT"]);
    }

    #[test]
    fn headings_count_their_section_and_drawings_carry_their_tool() {
        let mut config = IndicatorConfig::default();
        BuiltIn::Ema.set_applied(&mut config, true);
        let drawings = [line(1), line(2)];
        let rows = rows(&inputs(&config, &[], &drawings), "");
        let count = |section| {
            rows.iter()
                .find(|row| row.key == Key::Heading(section))
                .map(|row| row.count)
        };
        assert_eq!(count(Section::Symbol), Some(1));
        assert_eq!(count(Section::Indicators), Some(1));
        assert_eq!(count(Section::Drawings), Some(2));
        let drawing = rows.iter().find(|r| r.key == Key::Drawing(1)).unwrap();
        assert_eq!(drawing.tool, Some(Tool::TrendLine));
        // A filter narrows the rows but a heading still reports what the section holds.
        let filtered = super::rows(&inputs(&config, &[], &drawings), "line 1");
        let heading = filtered
            .iter()
            .find(|row| row.key == Key::Heading(Section::Drawings))
            .unwrap();
        assert_eq!(heading.count, 2);
    }

    #[test]
    fn a_drop_takes_the_place_of_the_row_it_lands_on() {
        let keys: Vec<Key> = (1..=4).map(Key::Drawing).collect();
        // Gaps are numbered 0 (above the first) to 4 (below the last).
        assert_eq!(drop_over(&keys, Key::Drawing(3), 0), Some(Key::Drawing(1)));
        assert_eq!(drop_over(&keys, Key::Drawing(3), 1), Some(Key::Drawing(2)));
        assert_eq!(drop_over(&keys, Key::Drawing(1), 4), Some(Key::Drawing(4)));
        assert_eq!(drop_over(&keys, Key::Drawing(1), 3), Some(Key::Drawing(3)));
        // The two gaps beside a row leave it where it is, as does a row that is not listed.
        assert_eq!(drop_over(&keys, Key::Drawing(3), 2), None);
        assert_eq!(drop_over(&keys, Key::Drawing(3), 3), None);
        assert_eq!(drop_over(&keys, Key::Drawing(9), 0), None);
        // Past the end is the end.
        assert_eq!(drop_over(&keys, Key::Drawing(1), 99), Some(Key::Drawing(4)));
        assert_eq!(drop_over(&[], Key::Drawing(1), 0), None);
    }

    #[test]
    fn only_overlays_scripts_and_readable_drawings_move_among_their_own_kind() {
        let config = IndicatorConfig::default();
        let mut with_ema = config;
        BuiltIn::Ema.set_applied(&mut with_ema, true);
        let scripts = [
            PineInstance::new(3, None, "One", "//@version=6"),
            PineInstance::new(4, None, "Two", "//@version=6"),
        ];
        let future = Drawing::Unreadable(serde_json::json!({"Spiral": {"id": 9}}));
        let drawings = [line(1), future, line(2)];
        let rows = rows(&inputs(&with_ema, &scripts, &drawings), "");
        let row = |key| rows.iter().find(|row| row.key == key).unwrap();

        assert_eq!(
            siblings(&rows, Key::Indicator(IndicatorKey::Script(4))),
            [
                Key::Indicator(IndicatorKey::Script(3)),
                Key::Indicator(IndicatorKey::Script(4))
            ],
            "the built-in indicator is not among them"
        );
        assert_eq!(
            siblings(&rows, Key::Drawing(1)),
            [Key::Drawing(2), Key::Drawing(9), Key::Drawing(1)],
            "an unreadable drawing holds its place in the stack"
        );
        assert!(draggable(row(Key::Drawing(1))));
        assert!(!draggable(row(Key::Drawing(9))), "from a newer version");
        assert!(draggable(row(Key::Indicator(IndicatorKey::Script(3)))));
        assert!(!draggable(row(Key::Indicator(IndicatorKey::BuiltIn(
            BuiltIn::Ema
        )))));
        assert!(!draggable(row(Key::Symbol)));
        assert!(!draggable(row(Key::Heading(Section::Drawings))));
        assert!(siblings(&rows, Key::Symbol).is_empty());
    }

    #[test]
    fn the_filter_keeps_matches_and_drops_empty_sections() {
        let mut config = IndicatorConfig::default();
        BuiltIn::Ema.set_applied(&mut config, true);
        let drawings = [line(1)];
        let rows = rows(&inputs(&config, &[], &drawings), "ema");
        assert_eq!(labels(&rows), ["Indicators", "EMA 20"]);
        let rows = super::rows(&inputs(&config, &[], &drawings), "TREND");
        assert_eq!(labels(&rows), ["Drawings", "Trend line 1"]);
        let none = super::rows(&inputs(&config, &[], &drawings), "zzz");
        assert!(none.is_empty());
        // Spaces around the filter do not matter.
        let spaced = super::rows(&inputs(&config, &[], &drawings), "  ema ");
        assert_eq!(labels(&spaced), ["Indicators", "EMA 20"]);
    }
}
