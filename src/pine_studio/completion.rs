//! Autocomplete for the Pine editor: what to offer for the identifier before the caret, and in
//! what order. A port of the spirit of `PineCompletionEngine` / `PineCompletionRanking`, limited
//! to names the Rust engine really supports: every builtin below comes from an engine table or
//! is checked against the engine's own dispatchers by a test, so nothing offered fails at run
//! time as "unknown function".

use super::edit_ops;
use super::symbols::{self, SymbolTable};
use super::syntax;
use crate::pine::symbol_catalog as catalog;
use crate::pine::{misc_builtins, registry, ta_extra};
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    /// A name the script declares: the closest scope, ranked first.
    User,
    /// A field of a user type, after `value.`.
    Field,
    /// A method of the value's type, after `value.`.
    Method,
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
            Self::Field => 'p',
            Self::Method => 'm',
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
            Self::Field => "field",
            Self::Method => "method",
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

/// The most names one request offers: enough for a whole namespace (`math.` and `array.` run to
/// about a hundred), which the list scrolls through, so none is cut off the end.
const MAX_SUGGESTIONS: usize = 200;

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
        for name in ta_extra::VARIABLES
            .iter()
            .chain(&registry::DRAWING_COLLECTIONS)
        {
            add(name, Kind::Variable);
        }
        for name in catalog::RESERVED_CONSTANTS {
            add(name, Kind::Constant);
        }
        // Every function the engine runs. Not the ones that answer `na` because the app has no such
        // feed, which are registered so the compiler knows them but are not worth suggesting.
        for name in registry::all_functions().iter().filter(|name| {
            let name = name.as_str();
            (!misc_builtins::UNAVAILABLE_FEEDS.contains(&name) || name == "request.currency_rate")
                && !misc_builtins::CURRENCY_CONVERSIONS.contains(&name)
                && name != "chart.point"
        }) {
            add(name, Kind::Function);
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

/// [`complete_with`] for a caller that has no symbol table of the text.
pub fn complete(source: &str, caret: usize, explicit: bool) -> Option<Completion> {
    complete_with(source, caret, explicit, &symbols::analyze(source))
}

/// What to offer at `caret`, or `None`. It needs two typed letters, a trailing `.`, or an
/// explicit request; it stays quiet inside strings and comments and when the only candidate is
/// exactly what is typed. `table` is the symbols of `source`: after `value.` it says what `value`
/// is, so the fields and methods of its type are offered.
pub fn complete_with(
    source: &str,
    caret: usize,
    explicit: bool,
    table: &SymbolTable,
) -> Option<Completion> {
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
    // A variable the script declares answers for itself, builtin namespaces of the same spelling or not.
    if dotted && let Some(items) = member_items(table, caret, &prefix) {
        return finish(items, &prefix, start..caret);
    }

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
    finish(items, &prefix, start..caret)
}

/// Ranks `items` for the typed `prefix` and wraps them, unless there is nothing to offer or the
/// only candidate is exactly what is typed.
fn finish(mut items: Vec<Item>, prefix: &str, replace: Range<usize>) -> Option<Completion> {
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
                !i.label.starts_with(prefix),
                !i.label.eq_ignore_ascii_case(prefix),
                i.kind,
                i.label.to_lowercase(),
            )
        };
        key(a).cmp(&key(b))
    });
    items.truncate(MAX_SUGGESTIONS);
    Some(Completion { replace, items })
}

/// The types whose members are the builtin functions of a namespace of the same name
/// (`array.push` is `values.push`), minus the namespace's constructors.
const COLLECTION_TYPES: [&str; 9] = [
    "array", "map", "matrix", "line", "label", "box", "table", "linefill", "polyline",
];

/// `value.partial` where the head of `value` is a variable the script declares: what can follow the
/// dot. `None` when the head is no such variable (`ta.`, `array.`), so the builtin catalogue
/// answers; an empty list when it is one whose type is not known, so nothing is guessed.
fn member_items(table: &SymbolTable, caret: usize, prefix: &str) -> Option<Vec<Item>> {
    let (path, partial) = prefix.rsplit_once('.')?;
    let declared = table.type_of_path(path, caret)?;
    let mut items = Vec::new();
    let Some(type_name) = declared else {
        return Some(items);
    };
    let lower = partial.to_lowercase();
    let mut offer = |member: &str, kind: Kind, callable: bool| {
        if member.to_lowercase().starts_with(&lower) {
            items.push(Item {
                label: format!("{path}.{member}"),
                kind,
                callable,
            });
        }
    };
    // `float[]` is an `array<float>`.
    let base = if type_name.ends_with("[]") {
        "array"
    } else {
        type_name.split('<').next().unwrap_or(&type_name)
    };
    if COLLECTION_TYPES.contains(&base) {
        for item in builtins() {
            if item.kind != Kind::Function {
                continue;
            }
            let Some(member) = item
                .label
                .strip_prefix(base)
                .and_then(|rest| rest.strip_prefix('.'))
            else {
                continue;
            };
            if member != "new" && member != "from" && !member.starts_with("new_") {
                offer(member, Kind::Method, true);
            }
        }
    } else if let Some(info) = table.type_info(&type_name) {
        for (field, _) in &info.fields {
            offer(field, Kind::Field, false);
        }
        for method in table.methods_of(&type_name) {
            offer(method, Kind::Method, true);
        }
        offer("copy", Kind::Method, true);
    }
    Some(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{array_builtins, drawings, map_builtins, matrix_builtins};

    fn labels(completion: &Completion) -> Vec<&str> {
        completion.items.iter().map(|i| i.label.as_str()).collect()
    }

    /// The editor offers a name, so the compiler must not call it unknown: `str.format_time` was
    /// computed by the session and missing from the `str` table the check reads.
    #[test]
    fn no_offered_function_is_reported_as_unknown() {
        use crate::pine::compiler::{NoLibraries, compile};
        use crate::pine::name_check::UNKNOWN_MEMBER;
        let prefixes = ["array.", "map.", "matrix.", "math.", "str.", "ta."];
        let mut wrongly_unknown = Vec::new();
        for item in builtins() {
            if item.kind != Kind::Function || !prefixes.iter().any(|p| item.label.starts_with(p)) {
                continue;
            }
            let source = format!("//@version=6\nindicator(\"T\")\nx = {}()\n", item.label);
            let program = compile(&source, crate::pine::Limits::default(), &NoLibraries);
            if program.diagnostics.iter().any(|d| d.code == UNKNOWN_MEMBER) {
                wrongly_unknown.push(item.label.clone());
            }
        }
        assert!(wrongly_unknown.is_empty(), "{wrongly_unknown:?}");
    }

    #[test]
    fn every_listed_name_is_supported() {
        let mut unsupported = Vec::new();
        let mut check = |name: &str, ok: bool| {
            if !ok {
                unsupported.push(name.to_owned());
            }
        };
        for name in COLORS {
            check(
                name,
                crate::pine::builtins::color(name).is_some()
                    || matches!(name, "color.new" | "color.rgb"),
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
    fn every_array_map_matrix_and_drawing_function_the_engine_handles_is_offered() {
        let offered: Vec<&str> = builtins().iter().map(|i| i.label.as_str()).collect();
        for (namespace, members) in drawings::NAMESPACES {
            for member in members {
                let name = format!("{namespace}.{member}");
                assert!(offered.contains(&name.as_str()), "{name}");
                assert!(drawings::handles(&name), "{name}");
            }
        }
        for name in drawings::CHART_POINT_NAMES {
            assert!(offered.contains(name), "{name}");
        }
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
        assert_eq!(
            labels(&narrowed),
            ["ta.pivot_point_levels", "ta.pivothigh", "ta.pivotlow"]
        );
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

    // MARK: Members after `value.`

    const STATE: &str = "type ReactionState
    float priorCore = na
    float priorTop = na
    float priorBottom = na
    int reclaimDirection = 0
    int breakBar = na
    int acceptanceDirection = 0
    int outsideCloses = 0

method forget(ReactionState self) =>
    self.reclaimDirection := 0
    self.breakBar := na
";

    fn after(typed: &str) -> Option<Completion> {
        let source = format!("{STATE}{typed}");
        complete(&source, source.chars().count(), false)
    }

    fn labels_of(found: &Option<Completion>) -> Vec<String> {
        found
            .as_ref()
            .map(|found| found.items.iter().map(|item| item.label.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn self_offers_the_fields_and_methods_of_its_type() {
        let found = after("    self.");
        let names = labels_of(&found);
        for field in [
            "priorCore",
            "priorTop",
            "priorBottom",
            "reclaimDirection",
            "breakBar",
            "acceptanceDirection",
            "outsideCloses",
        ] {
            let label = format!("self.{field}");
            let item = found
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|item| item.label == label)
                .unwrap_or_else(|| panic!("{label} in {names:?}"));
            assert_eq!(item.kind, Kind::Field);
            assert!(!item.callable);
        }
        // The method itself and the builtin `copy`, both callable; nothing else.
        for method in ["self.forget", "self.copy"] {
            let item = found
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|item| item.label == method)
                .unwrap_or_else(|| panic!("{method} in {names:?}"));
            assert_eq!(item.kind, Kind::Method);
            assert!(item.callable);
        }
        assert_eq!(names.len(), 9, "{names:?}");
        // The whole dotted word is replaced, as for `ta.`.
        let source = format!("{STATE}    self.");
        let end = source.chars().count();
        assert_eq!(found.unwrap().replace, end - 5..end);
    }

    #[test]
    fn typing_after_the_dot_narrows_the_members() {
        assert_eq!(
            labels_of(&after("    self.re")),
            ["self.reclaimDirection"],
            "case-insensitive prefix"
        );
        assert_eq!(
            labels_of(&after("    self.PRIOR")),
            ["self.priorBottom", "self.priorCore", "self.priorTop"]
        );
        assert!(after("    self.zzz").is_none());
    }

    #[test]
    fn the_dot_is_completed_in_the_middle_of_a_method_too() {
        let source = format!("{STATE}    self.\n    self.outsideCloses := 0\n");
        let caret = format!("{STATE}    self.").chars().count();
        let found = complete(&source, caret, false);
        assert_eq!(labels_of(&found).len(), 9);
    }

    #[test]
    fn a_name_that_is_not_a_declared_variable_offers_no_members() {
        // Outside the method `self` is nothing.
        assert!(after("self.").is_none());
        // An untyped variable: nothing is guessed, and no builtin name matches.
        let source = "x = 1\nx.";
        assert!(complete(source, source.chars().count(), false).is_none());
        // Builtin namespaces still answer for themselves.
        assert!(complete("ta.", 3, false).is_some());
    }

    #[test]
    fn a_typed_or_constructed_local_has_its_types_members() {
        let zone = "type Zone\n    float top\n    float bottom\n";
        for declaration in [
            "zone = Zone.new(1.0, 2.0)",
            "Zone zone = na",
            "var Zone zone = Zone.new()",
            "varip Zone zone = na",
        ] {
            let source = format!("{zone}{declaration}\nzone.");
            let found = complete(&source, source.chars().count(), false);
            assert_eq!(
                labels_of(&found),
                ["zone.bottom", "zone.top", "zone.copy"],
                "{declaration}"
            );
        }
    }

    #[test]
    fn collections_offer_their_namespace_without_constructors() {
        for declaration in [
            "window = array.new<float>()",
            "window = array.new_float()",
            "array<float> window = na",
            "float[] window = na",
        ] {
            let source = format!("{declaration}\nwindow.pu");
            let found = complete(&source, source.chars().count(), false);
            let names = labels_of(&found);
            assert!(
                names.contains(&"window.push".to_owned()),
                "{declaration}: {names:?}"
            );
            assert!(names.iter().all(|name| name.starts_with("window.pu")));
        }
        let source = "window = array.new_float()\nwindow.";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert!(names.contains(&"window.size".to_owned()));
        assert!(!names.iter().any(|name| name.contains("new")), "{names:?}");
        assert!(!names.contains(&"window.from".to_owned()));
        let source = "m = map.new<string, int>()\nm.";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert!(names.contains(&"m.put".to_owned()) && names.contains(&"m.get".to_owned()));
        let source = "ln = line.new(0, 1, 2, 3)\nln.set";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert!(names.contains(&"ln.set_x1".to_owned()), "{names:?}");
    }

    #[test]
    fn a_field_of_collection_type_is_followed_through_the_path() {
        let source = "type Book\n    array<float> items\n    Book next\n\
                      method top(Book this) =>\n    this.items.pu";
        let found = complete(source, source.chars().count(), false);
        assert_eq!(
            labels_of(&found),
            ["this.items.push"].map(str::to_owned).to_vec(),
        );
        // A field whose type is another user type.
        let source = "type Book\n    array<float> items\n    Book next\n\
                      method top(Book this) =>\n    this.next.";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert!(names.contains(&"this.next.items".to_owned()));
        assert!(names.contains(&"this.next.next".to_owned()));
        assert!(names.contains(&"this.next.top".to_owned()));
        // A field of unknown type ends the path.
        let source = "type Book\n    items\nmethod top(Book this) =>\n    this.items.";
        assert!(complete(source, source.chars().count(), false).is_none());
    }

    #[test]
    fn only_the_methods_of_the_receivers_type_are_offered() {
        let source = "type Dog\n    int legs\ntype Bird\n    int legs\n\
                      method bark(Dog this) => 1\n\
                      method chirp(Bird this) => 2\n\
                      method describe(Dog this) => 3\n\
                      method describe(Bird this) => 4\n\
                      method twice(float this) => this * 2.0\n\
                      d = Dog.new(4)\nd.";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert_eq!(
            names,
            ["d.legs", "d.bark", "d.copy", "d.describe"],
            "fields, then methods by name"
        );
    }

    #[test]
    fn a_library_types_methods_match_by_its_short_name() {
        let source = "type Zone\n    float top\nmethod grow(Zone this) => this\n\
                      method shrink(lib.Zone this) => this\nz = Zone.new(1.0)\nz.";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert!(names.contains(&"z.grow".to_owned()) && names.contains(&"z.shrink".to_owned()));
    }

    #[test]
    fn members_survive_crlf_and_astral_characters() {
        let source = "// 🙂🙂\r\ntype Zone\r\n    float top\r\n    float bottom\r\n\r\n\
                      method flip(Zone this) =>\r\n    this.";
        let caret = source.chars().count();
        let found = complete(source, caret, false).expect("members");
        assert_eq!(
            found.replace,
            caret - 5..caret,
            "a character range, not bytes"
        );
        assert_eq!(
            found
                .items
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["this.bottom", "this.top", "this.copy", "this.flip"]
        );
    }

    #[test]
    fn a_parameter_shadows_a_global_of_the_same_name() {
        let source = "type Dog\n    int legs\ntype Bird\n    int wings\n\
                      self = Bird.new(2)\n\
                      method go(Dog self) =>\n    self.";
        let names = labels_of(&complete(source, source.chars().count(), false));
        assert_eq!(names, ["self.legs", "self.copy", "self.go"]);
    }

    #[test]
    fn accepting_a_method_writes_the_call_and_a_field_does_not() {
        let found = after("    self.forg").expect("a method");
        assert_eq!(found.items[0].label, "self.forget");
        assert!(found.items[0].callable);
        let found = after("    self.breakB").expect("a field");
        assert!(!found.items[0].callable);
    }
}
