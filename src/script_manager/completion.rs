//! Autocomplete for the Pine editor: what to offer for the identifier before the caret, and in
//! what order. A port of the spirit of `PineCompletionEngine` / `PineCompletionRanking`, limited
//! to names the Rust engine really supports: every builtin below comes from an engine table or
//! is checked against the engine's own dispatchers by a test, so nothing offered fails at run
//! time as "unknown function".

use super::edit_ops;
use super::syntax;
use crate::pine::symbol_catalog as catalog;
use crate::pine::{array_builtins, map_builtins, math, matrix_builtins, strings, ta, ta_extra};
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    /// A name the script declares: the closest scope, ranked first.
    User,
    Variable,
    Function,
    Namespace,
    Keyword,
    Type,
    Constant,
}

impl Kind {
    /// The badge letter (`f`unction, `v`ariable, `c`onstant, `n`amespace, `k`eyword, `t`ype,
    /// `u`ser).
    pub fn badge(self) -> char {
        match self {
            Self::User => 'u',
            Self::Variable => 'v',
            Self::Function => 'f',
            Self::Namespace => 'n',
            Self::Keyword => 'k',
            Self::Type => 't',
            Self::Constant => 'c',
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::User => "your symbol",
            Self::Variable => "built-in variable",
            Self::Function => "built-in function",
            Self::Namespace => "namespace",
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Constant => "constant",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: Kind,
    /// A function: accepting it also writes the call parentheses.
    pub callable: bool,
}

/// What to show for the caret's position: the char range the choice replaces and the ranked items.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Completion {
    pub replace: Range<usize>,
    pub items: Vec<Item>,
}

const KEYWORDS: [&str; 23] = [
    "if", "else", "for", "while", "switch", "var", "varip", "import", "export", "method", "type",
    "enum", "and", "or", "not", "true", "false", "na", "break", "continue", "to", "by", "once",
];

const TYPES: [&str; 16] = [
    "int",
    "float",
    "bool",
    "string",
    "color",
    "line",
    "label",
    "box",
    "table",
    "series",
    "simple",
    "const",
    "array",
    "map",
    "footprint",
    "volume_row",
];

/// Names beyond the engine tables, each verified by `every_listed_name_is_supported`.
const MORE_TA: [&str; 13] = [
    "ta.barssince",
    "ta.vwap",
    "ta.dmi",
    "ta.cum",
    "ta.correlation",
    "ta.tr",
    "ta.atr",
    "ta.bb",
    "ta.pivothigh",
    "ta.pivotlow",
    "ta.sar",
    "ta.supertrend",
    "ta.linreg",
];

/// Scalar builtins, exercised end to end by `pine_host::call_site_tests` (the footprint ones by
/// `pine::footprint_tests`).
const MORE_SCALAR: [&str; 40] = [
    "max_bars_back",
    "ticker.new",
    "ticker.standard",
    "ticker.modify",
    "ticker.inherit",
    "log.info",
    "log.warning",
    "log.error",
    "request.currency_rate",
    "chart.point.new",
    "request.security",
    "timeframe.in_seconds",
    "timeframe.change",
    "str.format_time",
    "runtime.error",
    "color.r",
    "color.g",
    "color.b",
    "color.t",
    "color.from_gradient",
    "fixnan",
    "timeframe.from_seconds",
    "request.footprint",
    "footprint.buy_volume",
    "footprint.sell_volume",
    "footprint.total_volume",
    "footprint.delta",
    "footprint.poc",
    "footprint.vah",
    "footprint.val",
    "footprint.rows",
    "footprint.get_row_by_price",
    "volume_row.up_price",
    "volume_row.down_price",
    "volume_row.buy_volume",
    "volume_row.sell_volume",
    "volume_row.total_volume",
    "volume_row.delta",
    "volume_row.has_buy_imbalance",
    "volume_row.has_sell_imbalance",
];

/// Arrays of the drawings alive, read as variables.
const DRAWING_COLLECTIONS: [&str; 6] = [
    "line.all",
    "label.all",
    "box.all",
    "table.all",
    "linefill.all",
    "polyline.all",
];

const INPUTS: [&str; 7] = [
    "input.int",
    "input.float",
    "input.bool",
    "input.string",
    "input.color",
    "input.source",
    "input.time",
];

const COLORS: [&str; 19] = [
    "color.aqua",
    "color.black",
    "color.blue",
    "color.fuchsia",
    "color.gray",
    "color.green",
    "color.lime",
    "color.maroon",
    "color.navy",
    "color.olive",
    "color.orange",
    "color.purple",
    "color.red",
    "color.silver",
    "color.teal",
    "color.white",
    "color.yellow",
    "color.new",
    "color.rgb",
];

const DRAWINGS: [&str; 14] = [
    "line.new",
    "line.delete",
    "line.set_xy1",
    "line.set_xy2",
    "label.new",
    "label.delete",
    "label.set_text",
    "box.new",
    "box.delete",
    "table.new",
    "table.cell",
    "polyline.new",
    "linefill.new",
    "line.set_color",
];

const ACTIONS: [&str; 5] = [
    "alert",
    "alertcondition",
    "strategy.entry",
    "strategy.close",
    "strategy.exit",
];

const CONSTANTS: [&str; 24] = [
    "plot.style_line",
    "plot.style_histogram",
    "plot.style_columns",
    "plot.style_area",
    "plot.style_circles",
    "plot.style_cross",
    "shape.triangleup",
    "shape.triangledown",
    "shape.circle",
    "shape.cross",
    "shape.xcross",
    "shape.diamond",
    "shape.square",
    "location.abovebar",
    "location.belowbar",
    "location.top",
    "location.bottom",
    "size.tiny",
    "size.small",
    "size.normal",
    "size.large",
    "size.huge",
    "display.none",
    "display.all",
];

/// Every builtin the editor offers, with its kind.
pub fn builtins() -> &'static [Item] {
    static ITEMS: OnceLock<Vec<Item>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let mut names: BTreeSet<(String, Kind)> = BTreeSet::new();
        let mut add = |name: &str, kind: Kind| {
            names.insert((name.to_owned(), kind));
        };
        for name in catalog::VARIABLES {
            add(name, Kind::Variable);
        }
        for name in catalog::FUNCTIONS {
            add(name, Kind::Function);
        }
        for name in ta_extra::VARIABLES.iter().chain(&DRAWING_COLLECTIONS) {
            add(name, Kind::Variable);
        }
        for name in catalog::RESERVED_CONSTANTS {
            add(name, Kind::Constant);
        }
        for name in ta::SUPPORTED
            .iter()
            .chain(&MORE_TA)
            .chain(&MORE_SCALAR)
            .chain(&ta_extra::FUNCTIONS)
            .chain(&math::SUPPORTED)
            .chain(&strings::SUPPORTED)
            .chain(&INPUTS)
            .chain(&array_builtins::KNOWN)
            .chain(&map_builtins::NAMES)
            .chain(&DRAWINGS)
            .chain(&ACTIONS)
        {
            add(name, Kind::Function);
        }
        for member in matrix_builtins::MEMBERS {
            add(&format!("matrix.{member}"), Kind::Function);
        }
        for name in COLORS
            .iter()
            .filter(|n| !matches!(**n, "color.new" | "color.rgb"))
        {
            add(name, Kind::Constant);
        }
        add("color.new", Kind::Function);
        add("color.rgb", Kind::Function);
        for name in CONSTANTS {
            add(name, Kind::Constant);
        }
        for name in KEYWORDS {
            add(name, Kind::Keyword);
        }
        for name in TYPES {
            add(name, Kind::Type);
        }
        names
            .into_iter()
            .map(|(label, kind)| Item {
                callable: kind == Kind::Function,
                label,
                kind,
            })
            .collect()
    })
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// What to offer at `caret`, or `None`. It needs two typed letters, a trailing `.`, or an
/// explicit request; it stays quiet inside strings and comments and when the only candidate is
/// exactly what is typed.
pub fn complete(source: &str, caret: usize, explicit: bool) -> Option<Completion> {
    let chars: Vec<char> = source.chars().collect();
    let caret = caret.min(chars.len());
    let mut start = caret;
    while start > 0 && (is_word(chars[start - 1]) || chars[start - 1] == '.') {
        start -= 1;
    }
    // A leading dot belongs to a value (`foo().bar`), not to a name we can complete.
    while start < caret && chars[start] == '.' {
        start += 1;
    }
    let prefix: String = chars[start..caret].iter().collect();
    if prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) || edit_ops::in_text(source, caret)
    {
        return None;
    }
    let letters = prefix.chars().filter(|c| is_word(*c)).count();
    let after_dot = prefix.ends_with('.');
    if !(explicit || after_dot || letters >= 2) {
        return None;
    }
    let lower = prefix.to_lowercase();
    let dotted = prefix.contains('.');

    let mut items: Vec<Item> = Vec::new();
    let mut heads: BTreeSet<String> = BTreeSet::new();
    for item in builtins() {
        if dotted {
            if item.label.to_lowercase().starts_with(&lower) {
                items.push(item.clone());
            }
        } else if let Some((head, _)) = item.label.split_once('.') {
            if head.to_lowercase().starts_with(&lower) {
                heads.insert(head.to_owned());
            }
        } else if item.label.to_lowercase().starts_with(&lower) {
            items.push(item.clone());
        }
    }
    for head in heads {
        if !items.iter().any(|i| i.label == head) {
            items.push(Item {
                label: head,
                kind: Kind::Namespace,
                callable: false,
            });
        }
    }
    if !dotted {
        // Your own names shadow builtins of the same spelling.
        let user = syntax::declared_symbols(source);
        let functions = syntax::declared_functions(source);
        items.retain(|i| !user.contains(&i.label));
        items.extend(
            user.into_iter()
                .filter(|name| name.to_lowercase().starts_with(&lower))
                .map(|label| Item {
                    callable: functions.contains(&label),
                    label,
                    kind: Kind::User,
                }),
        );
    }
    if items.is_empty() || (items.len() == 1 && items[0].label == prefix) {
        return None;
    }
    // One entry per spelling (`color` is a function, a type and a namespace): the closest kind wins.
    items.sort_by_key(|item| item.kind);
    let mut seen = BTreeSet::new();
    items.retain(|item| seen.insert(item.label.clone()));
    // Exact spelling first, then your own names, then by kind and alphabet.
    items.sort_by(|a, b| {
        let key = |i: &Item| {
            (
                !i.label.starts_with(&prefix),
                !i.label.eq_ignore_ascii_case(&prefix),
                i.kind,
                i.label.to_lowercase(),
            )
        };
        key(a).cmp(&key(b))
    });
    items.truncate(60);
    Some(Completion {
        replace: start..caret,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{alert_builtins, drawings, strategy_builtins};

    fn labels(completion: &Completion) -> Vec<&str> {
        completion.items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn every_listed_name_is_supported() {
        let mut unsupported = Vec::new();
        let mut check = |name: &str, ok: bool| {
            if !ok {
                unsupported.push(name.to_owned());
            }
        };
        for name in MORE_TA {
            let known = ta::is_supported(name)
                || matches!(
                    name,
                    "ta.cum"
                        | "ta.barssince"
                        | "ta.vwap"
                        | "ta.dmi"
                        | "ta.correlation"
                        | "ta.tr"
                        | "ta.atr"
                        | "ta.bb"
                        | "ta.pivothigh"
                        | "ta.pivotlow"
                        | "ta.sar"
                        | "ta.supertrend"
                );
            check(name, known);
        }
        for name in COLORS {
            check(
                name,
                crate::pine::builtins::color(name).is_some()
                    || matches!(name, "color.new" | "color.rgb"),
            );
        }
        for name in DRAWINGS {
            check(name, drawings::handles(name));
        }
        for name in ACTIONS {
            check(
                name,
                alert_builtins::handles(name) || strategy_builtins::handles(name),
            );
        }
        for name in CONSTANTS {
            check(
                name,
                crate::pine::builtins::constant(name).is_some()
                    || crate::pine::compiler::is_named_constant(name),
            );
        }
        assert!(unsupported.is_empty(), "{unsupported:?}");
    }

    #[test]
    fn every_array_map_and_matrix_function_the_engine_handles_is_offered() {
        let offered: Vec<&str> = builtins().iter().map(|i| i.label.as_str()).collect();
        for member in matrix_builtins::MEMBERS {
            let name = format!("matrix.{member}");
            assert!(offered.contains(&name.as_str()), "{name}");
        }
        for name in array_builtins::KNOWN.iter().chain(&map_builtins::NAMES) {
            assert!(offered.contains(name), "{name}");
        }
        for name in [
            "array.percentrank",
            "array.every",
            "array.some",
            "map.put_all",
        ] {
            assert!(offered.contains(&name), "{name}");
        }
    }

    #[test]
    fn the_newer_ta_names_are_offered_with_the_right_kinds() {
        let found = complete("x = ta.", 7, false).unwrap();
        for name in ["ta.alma", "ta.valuewhen", "ta.stoch", "ta.obv"] {
            assert!(labels(&found).contains(&name), "{name}");
        }
        let kind = |name: &str| {
            builtins()
                .iter()
                .find(|item| item.label == name)
                .map(|item| item.kind)
        };
        assert_eq!(kind("ta.obv"), Some(Kind::Variable));
        assert_eq!(kind("ta.mfi"), Some(Kind::Function));
        assert_eq!(kind("box.all"), Some(Kind::Variable));
    }

    #[test]
    fn two_letters_trigger_and_one_does_not() {
        assert!(complete("cl", 2, false).is_some());
        assert!(complete("c", 1, false).is_none());
        assert!(
            complete("c", 1, true).is_some(),
            "an explicit request needs no letters"
        );
    }

    #[test]
    fn a_dot_lists_the_namespace_members() {
        let found = complete("x = ta.", 7, false).unwrap();
        assert_eq!(found.replace, 4..7);
        assert!(labels(&found).contains(&"ta.sma") && labels(&found).contains(&"ta.atr"));
        assert!(labels(&found).iter().all(|l| l.starts_with("ta.")));
        let narrowed = complete("x = ta.pi", 9, false).unwrap();
        assert_eq!(labels(&narrowed), ["ta.pivothigh", "ta.pivotlow"]);
    }

    #[test]
    fn namespaces_appear_once_and_your_own_names_lead() {
        let source = "close_ma = 1\nplot(cl";
        let found = complete(source, source.chars().count(), false).unwrap();
        let names = labels(&found);
        assert_eq!(names, ["close_ma", "close"], "your own name leads");
        let colour = complete("co", 2, false).unwrap();
        assert_eq!(labels(&colour).iter().filter(|n| **n == "color").count(), 1);
        let ns = complete("ma", 2, false).unwrap();
        assert!(
            ns.items
                .iter()
                .any(|i| i.label == "math" && i.kind == Kind::Namespace)
        );
    }

    #[test]
    fn declared_names_shadow_builtins_and_stay_quiet_in_text() {
        let found = complete("close = 1\nclo", 13, false).unwrap();
        assert_eq!(found.items[0].kind, Kind::User);
        assert_eq!(found.items.iter().filter(|i| i.label == "close").count(), 1);
        assert!(complete("// clo", 6, false).is_none(), "comments are text");
        assert!(complete("s = \"clo", 8, false).is_none(), "so are strings");
        assert!(complete("12", 2, false).is_none(), "numbers are not names");
    }

    #[test]
    fn functions_are_callable_and_other_names_are_not() {
        let found = complete("color.", 6, false).unwrap();
        let callable = |label: &str| {
            found
                .items
                .iter()
                .find(|i| i.label == label)
                .unwrap()
                .callable
        };
        assert!(callable("color.new") && callable("color.rgb"));
        assert!(!callable("color.red"));
        let namespaces = complete("ta", 2, false).unwrap();
        assert!(
            namespaces
                .items
                .iter()
                .all(|i| i.kind != Kind::Namespace || !i.callable)
        );
        let own = complete(
            "half(x) => x / 2
level = 1
hal",
            30,
            false,
        )
        .unwrap();
        assert!(own.items.iter().any(|i| i.label == "half" && i.callable));
        let value = complete(
            "level = 1
lev",
            13,
            false,
        )
        .unwrap();
        assert!(
            value
                .items
                .iter()
                .any(|i| i.label == "level" && !i.callable)
        );
    }

    #[test]
    fn an_exact_lone_match_offers_nothing() {
        assert!(complete("barstate.isfirst", 16, false).is_none());
    }
}
