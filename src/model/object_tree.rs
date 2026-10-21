//! The Object Tree's rows: the chart's symbol, its indicators and scripts, and its drawings.
//!
//! A view over state the chart already owns (`IndicatorConfig`, the Pine instances, the drawing
//! list), rebuilt from it; nothing here is stored. Drawings are listed topmost first, the order
//! the chart paints them back to front reversed, so the row on top is the one drawn on top.
//! Pure: the panel draws the rows and the workspace applies what the user does to them.

use super::drawings::Drawing;
use super::pine_instance::PineInstance;
use super::{BuiltIn, IndicatorConfig, Tool};

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
    Indicator(IndicatorKey),
    Drawing(u64),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Section {
    Symbol,
    Indicators,
    Drawings,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::Symbol => "Symbol",
            Self::Indicators => "Indicators",
            Self::Drawings => "Drawings",
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

    let mut indicators = Vec::new();
    for indicator in BuiltIn::ALL
        .into_iter()
        .filter(|indicator| indicator.applied(inputs.indicators))
    {
        indicators.push(Row {
            key: Key::Indicator(IndicatorKey::BuiltIn(indicator)),
            label: indicator.chip_title(inputs.indicators),
            detail: indicator.menu_title().to_owned(),
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
            detail: "Script".to_owned(),
            shown: Some(script.visible),
            locked: None,
            tool: None,
            count: 0,
        });
    }
    sections.push((Section::Indicators, indicators));

    // Topmost first, numbered per tool from the bottom so a name stays with its drawing.
    let mut counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    let mut drawings: Vec<Row> = inputs
        .drawings
        .iter()
        .filter(|drawing| drawing.tool().is_some() && !drawing.is_ruler())
        .map(|drawing| {
            let name = drawing.tool().map_or("Drawing", |tool| tool.label());
            let number = counts.entry(name).or_insert(0);
            *number += 1;
            Row {
                key: Key::Drawing(drawing.id()),
                label: format!("{name} {number}"),
                detail: drawing.text().to_owned(),
                shown: Some(!drawing.hidden()),
                locked: Some(drawing.locked()),
                tool: drawing.tool(),
                count: 0,
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Anchor;
    use crate::model::drawing_style::Visibility;

    fn line(id: u64) -> Drawing {
        Drawing::TrendLine {
            id,
            a: Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: Anchor {
                time_ms: 1,
                price: 2.0,
            },
            locked: false,
            hidden: false,
            visibility: Visibility::default(),
            style: crate::model::drawing_style::DrawingStyle::trend_line(),
        }
    }

    fn inputs<'a>(
        config: &'a IndicatorConfig,
        scripts: &'a [PineInstance],
        drawings: &'a [Drawing],
    ) -> Inputs<'a> {
        Inputs {
            title: "BTCUSDT",
            source: "Binance",
            indicators: config,
            scripts,
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
        BuiltIn::Rsi.set_applied(&mut config, true);
        BuiltIn::Rsi.toggle_visible(&mut config);
        let scripts = [PineInstance::new(3, None, "My script", "//@version=6")];
        let drawings = [line(1), line(2)];
        let rows = rows(&inputs(&config, &scripts, &drawings), "");
        assert_eq!(
            labels(&rows),
            [
                "Symbol",
                "BTCUSDT",
                "Indicators",
                "RSI 14",
                "EMA 20",
                "My script",
                "Drawings",
                "Trend line 2",
                "Trend line 1",
            ]
        );
        let rsi = rows.iter().find(|row| row.label == "RSI 14").unwrap();
        assert_eq!(rsi.shown, Some(false), "hidden by its eye");
        let script = rows.iter().find(|row| row.label == "My script").unwrap();
        assert_eq!(script.key, Key::Indicator(IndicatorKey::Script(3)));
        assert_eq!(script.shown, Some(true));
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
    fn rulers_and_unreadable_rows_are_not_objects() {
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
        let unreadable = Drawing::Unreadable(serde_json::json!({"Spiral": {"id": 2}}));
        let drawings = [ruler, unreadable];
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
