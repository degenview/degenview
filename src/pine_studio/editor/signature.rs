//! Signature help: for the call the caret is in, `ta.sma(source, length) → series float` with the
//! argument under the caret marked. A port of the spirit of `PineSignatureResolver`; the
//! parameters come from `pine::compiler::signatures`, whose table is checked against the
//! completion catalogue here.

use super::edit_ops::{self, CallAt};
use super::symbols::{self, Kind, SymbolTable};
use crate::pine::compiler::{signatures::signature_of, symbol_catalog};

/// What to show: the call, its parameters, and which one the caret is on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Help {
    pub name: String,
    /// Each parameter as it is shown: a builtin's name, or a user parameter as the script writes it
    /// (`float factor = 2.0`).
    pub parameters: Vec<String>,
    /// The name of each parameter, which a `name =` keyword in the call matches.
    pub names: Vec<String>,
    /// Empty for a function the script declares: its type needs the checker, which does not run
    /// while typing.
    pub returns: String,
    pub active: Option<usize>,
    /// How many other declarations of this name there are (a function declared for several types).
    pub overloads: usize,
}

fn active_parameter(names: &[String], call: &CallAt) -> Option<usize> {
    if let Some(keyword) = &call.keyword
        && let Some(index) = names.iter().position(|p| p == keyword)
    {
        return Some(index);
    }
    if call.argument < names.len() {
        Some(call.argument)
    } else if names.last().is_some_and(|last| last.ends_with('…')) {
        Some(names.len() - 1)
    } else {
        None
    }
}

/// [`help_with`] for a caller that has no symbol table of the text.
#[cfg(test)]
pub fn help(source: &str, caret: usize) -> Option<Help> {
    help_with(source, caret, &symbols::analyze(source))
}

/// Signature help for the call around `caret`: a function or method the script declares, or a
/// builtin. `table` is the symbols of `source`.
pub fn help_with(source: &str, caret: usize, table: &SymbolTable) -> Option<Help> {
    let call = edit_ops::enclosing_call(source, caret)?;
    if let Some(help) = user_help(&call, caret, table) {
        return Some(help);
    }
    let (parameters, returns) = signature_of(&call.name)?;
    let active = active_parameter(&parameters, &call);
    Some(Help {
        name: call.name,
        names: parameters.clone(),
        parameters,
        returns: returns.to_owned(),
        active,
        overloads: 0,
    })
}

/// The declarations a method call `path.member(…)` can mean, or None when it is not a call to a
/// method the script declares (a builtin namespace, a library, a value of unknown kind).
fn receiver_methods<'a>(
    table: &'a SymbolTable,
    caret: usize,
    path: &str,
    member: &str,
) -> Option<Vec<&'a symbols::Symbol>> {
    // Without a known receiver type, the methods of that name stand for the call only when they all
    // belong to one type: guessing between types would show the wrong parameters.
    let by_name = || {
        let all = table.methods_named(member, None);
        let one_type = all.windows(2).all(|pair| {
            pair[0].receiver.as_deref().map(symbols::short_name)
                == pair[1].receiver.as_deref().map(symbols::short_name)
        });
        (!all.is_empty() && one_type).then_some(all)
    };
    if path.is_empty() {
        // A method called on something that is not a plain path: `f().member(…)`.
        return by_name();
    }
    let head = path.split('.').next().unwrap_or(path);
    match table.value_at(head, caret) {
        Some(symbol) if symbol.kind == Kind::ImportAlias => None,
        Some(_) => match table.type_of_path(path, caret)? {
            Some(type_name) => {
                let methods = table.methods_named(member, Some(&type_name));
                (!methods.is_empty()).then_some(methods)
            }
            None => by_name(),
        },
        // A builtin series (`close.scaled(…)`); a namespace (`ta.sma(…)`) is a builtin's.
        None if !path.contains('.') && symbol_catalog::is_variable(head) => by_name(),
        None => None,
    }
}

/// The parameters a call shows: all of them, or without the receiver for `value.name(…)`.
fn shown(declaration: &symbols::Symbol, method_syntax: bool) -> Vec<&symbols::ParamSig> {
    declaration
        .params
        .iter()
        .skip(usize::from(method_syntax))
        .collect()
}

/// Help for a function or method the script declares. A method called as `value.name(…)` is shown
/// without its receiver, the first parameter, which is the value before the dot.
fn user_help(call: &CallAt, caret: usize, table: &SymbolTable) -> Option<Help> {
    let method_syntax = call.name.contains('.');
    let candidates = match call.name.rsplit_once('.') {
        Some((path, member)) => receiver_methods(table, caret, path, member)?,
        None => table.callables_named(&call.name, caret),
    };
    // The first declaration that can take the argument being typed; the first one otherwise.
    let takes = |declaration: &symbols::Symbol| {
        let parameters = shown(declaration, method_syntax);
        call.argument < parameters.len()
            || call
                .keyword
                .as_ref()
                .is_some_and(|keyword| parameters.iter().any(|p| &p.name == keyword))
    };
    let chosen = candidates
        .iter()
        .find(|declaration| takes(declaration))
        .or(candidates.first())?;
    let parameters = shown(chosen, method_syntax);
    let names: Vec<String> = parameters.iter().map(|p| p.name.clone()).collect();
    let active = active_parameter(&names, call);
    Some(Help {
        name: call.name.trim_start_matches('.').to_owned(),
        parameters: parameters.iter().map(|p| p.text.clone()).collect(),
        names,
        returns: String::new(),
        active,
        overloads: candidates.len() - 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::signatures;
    use crate::pine_studio::editor::completion;

    #[test]
    fn every_signature_names_something_the_editor_offers() {
        let offered: Vec<&str> = completion::builtins()
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        let strangers: Vec<&str> = signatures::tabled_names()
            .filter(|name| !offered.contains(name))
            .collect();
        assert!(strangers.is_empty(), "{strangers:?}");
        // No duplicates either.
        let mut names: Vec<&str> = signatures::tabled_names().collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    #[test]
    fn newer_builtins_have_help_from_the_engine_tables() {
        let at = |text: &str| help(text, text.chars().count()).unwrap();
        let stoch = at("ta.stoch(close, high, ");
        assert_eq!(stoch.parameters, ["source", "high", "low", "length"]);
        assert_eq!(
            (stoch.returns.as_str(), stoch.active),
            ("series float", Some(2))
        );
        assert_eq!(
            at("ta.kc(close, 20, ").returns,
            "[series float, series float, series float]"
        );
        assert_eq!(at("str.replace(s, ").active, Some(1));
        assert_eq!(at("array.fill(a, 0, ").active, Some(2));
    }

    #[test]
    fn the_active_parameter_follows_the_caret_and_keywords() {
        let at = |text: &str| help(text, text.chars().count());
        let sma = at("ta.sma(close, ").unwrap();
        assert_eq!(sma.parameters, ["source", "length"]);
        assert_eq!(
            (sma.returns.as_str(), sma.active),
            ("series float", Some(1))
        );
        assert_eq!(at("ta.sma(").unwrap().active, Some(0));
        assert_eq!(
            at("ta.sma(close, 14, ").unwrap().active,
            None,
            "past the last parameter"
        );
        assert_eq!(at("plot(close, title = ").unwrap().active, Some(1));
        assert_eq!(
            at("plot(close, linewidth = 2, color = ").unwrap().active,
            Some(2),
            "a keyword wins over position"
        );
        assert_eq!(
            at("math.max(1, 2, 3, ").unwrap().active,
            Some(2),
            "a variadic tail repeats"
        );
        assert!(at("myFunc(").is_none(), "not a builtin");
        assert!(at("(1 + ").is_none());
    }

    #[test]
    fn a_caret_inside_a_keyword_marks_that_parameter() {
        // `plot(series, title, color, linewidth, ...)`: the caret is in the name of an argument
        // that skips parameters, so position 3 would point at the wrong one.
        let text = "plot(close, color = red, linewidth = 2)";
        let caret = |needle: &str, offset: usize| {
            let start = text.find(needle).unwrap();
            text[..start].chars().count() + offset
        };
        let active = |caret: usize| help(text, caret).unwrap().active;
        let linewidth = Some(3);
        assert_eq!(active(caret("linewidth", 0)), linewidth, "before the name");
        assert_eq!(active(caret("linewidth", 4)), linewidth, "inside the name");
        assert_eq!(active(caret("linewidth", 9)), linewidth, "after the name");
        assert_eq!(active(caret("color", 2)), Some(2), "an earlier keyword");
        // A user function with named arguments out of order.
        let source = "f(a, b, c, d) => a\nx = f(1, 2, d = 4, c = 3)";
        let at_name = |name: &str| {
            let start = source.rfind(name).unwrap();
            help(source, source[..start].chars().count() + 1)
                .unwrap()
                .active
        };
        assert_eq!((at_name("d ="), at_name("c =")), (Some(3), Some(2)));
    }

    #[test]
    fn collections_use_the_engines_own_parameter_names() {
        let push = help("array.push(", 11).unwrap();
        assert_eq!(push.parameters.first().map(String::as_str), Some("id"));
        assert!(push.parameters.len() >= 2);
    }

    // MARK: Functions and methods the script declares

    fn at_end(text: &str) -> Option<Help> {
        help(text, text.chars().count())
    }

    #[test]
    fn a_function_the_script_declares_has_help() {
        let found = at_end("half(x) => x / 2\nhalf(").unwrap();
        assert_eq!(found.name, "half");
        assert_eq!(found.parameters, ["x"]);
        assert_eq!(found.names, ["x"]);
        assert_eq!((found.active, found.overloads), (Some(0), 0));
        assert!(found.returns.is_empty(), "the type needs the checker");
    }

    #[test]
    fn parameters_are_shown_as_written_with_types_qualifiers_and_defaults() {
        let found =
            at_end("scale(float x, simple int length = 14, array<float> v) => 1\nscale(1, ")
                .unwrap();
        assert_eq!(
            found.parameters,
            ["float x", "simple int length = 14", "array<float> v"]
        );
        assert_eq!(found.names, ["x", "length", "v"]);
        assert_eq!(found.active, Some(1));
        // A parameter list wrapped over lines still reads as one line each.
        let wrapped = at_end("scale(float x,\n      float k = 2.0) => 1\nscale(").unwrap();
        assert_eq!(wrapped.parameters, ["float x", "float k = 2.0"]);
    }

    #[test]
    fn keywords_match_parameter_names_not_their_text() {
        let found = at_end("scale(float x, float k = 2.0) => 1\nscale(1, k = ").unwrap();
        assert_eq!(found.active, Some(1));
        let found = at_end("scale(float x, float k = 2.0) => 1\nscale(k = 3, ").unwrap();
        assert_eq!(found.active, Some(1), "a keyword wins over position");
    }

    #[test]
    fn a_method_called_with_a_dot_hides_its_receiver() {
        let zone = "type Zone\n    float top\n\
                    method grow(Zone this, float by, int times = 1) => this\n";
        for declaration in [
            "z = Zone.new(1.0)",
            "Zone z = na",
            "var Zone z = Zone.new(1.0)",
        ] {
            let text = format!("{zone}{declaration}\nz.grow(");
            let found = at_end(&text).unwrap_or_else(|| panic!("{declaration}"));
            assert_eq!(found.name, "z.grow");
            assert_eq!(found.parameters, ["float by", "int times = 1"]);
            assert_eq!(found.active, Some(0), "the receiver is not an argument");
        }
        let text = format!("{zone}z = Zone.new(1.0)\nz.grow(1.0, ");
        assert_eq!(at_end(&text).unwrap().active, Some(1));
    }

    #[test]
    fn a_method_on_a_builtin_series_or_self_has_help() {
        let found =
            at_end("method scaled(float this, float k) => this * k\nclose.scaled(").unwrap();
        assert_eq!(found.parameters, ["float k"]);
        let found = at_end(
            "type S\n    int n\nmethod forget(S self, int keep = 0) =>\n    self.n := 0\n    self.forget(",
        )
        .unwrap();
        assert_eq!(found.parameters, ["int keep = 0"]);
        // A method called on a call's result: the only method of that name.
        let found =
            at_end("method scaled(float this, float k) => this * k\nmake().scaled(").unwrap();
        assert_eq!(found.parameters, ["float k"]);
    }

    #[test]
    fn overloads_are_picked_by_the_receiver_type() {
        let methods = "type Dog\n    int legs\ntype Bird\n    int wings\n\
                       method describe(Dog this, string prefix) => 1\n\
                       method describe(Bird this, int times) => 2\n\
                       d = Dog.new(4)\nb = Bird.new(2)\nx = foo()\n";
        let dog = at_end(&format!("{methods}d.describe(")).unwrap();
        assert_eq!(
            (dog.parameters, dog.overloads),
            (vec!["string prefix".to_owned()], 0)
        );
        let bird = at_end(&format!("{methods}b.describe(")).unwrap();
        assert_eq!(bird.parameters, ["int times"]);
        // An untyped receiver could be either: nothing is guessed.
        assert!(at_end(&format!("{methods}x.describe(")).is_none());
    }

    #[test]
    fn overloads_of_one_type_are_picked_by_how_many_arguments_are_given() {
        let text = "method fill(array<float> a, float v) => a\n\
                    method fill(array<float> a, float inner, float outer, float lo, float hi) => a\n\
                    values = array.new_float()\n";
        let first = at_end(&format!("{text}values.fill(")).unwrap();
        assert_eq!((first.parameters.len(), first.overloads), (1, 1));
        let more = at_end(&format!("{text}values.fill(1.0, 0.0, ")).unwrap();
        assert_eq!(more.parameters.len(), 4);
        assert_eq!((more.active, more.overloads), (Some(2), 1));
    }

    #[test]
    fn a_method_called_as_a_function_shows_its_receiver() {
        let found =
            at_end("type Z\n    int a\nmethod grow(Z this, float by) => this\ngrow(").unwrap();
        assert_eq!(found.parameters, ["Z this", "float by"]);
    }

    #[test]
    fn builtins_are_not_taken_over_by_methods_of_the_same_name() {
        let found = at_end("method push(array<float> a) => a\narray.push(").unwrap();
        assert_eq!(found.parameters.first().map(String::as_str), Some("id"));
        assert_eq!(found.overloads, 0);
        assert!(at_end("ta.sma(close, ").is_some());
        // A library alias's members are the library's.
        assert!(at_end("import user/lib/1 as lib\nmethod f(float x) => x\nlib.f(").is_none());
        // Not declared anywhere.
        assert!(at_end("nothing(").is_none());
        assert!(
            at_end("method f(float x) => x\nx.f(").is_none(),
            "undeclared head"
        );
    }

    #[test]
    fn user_help_survives_crlf_and_astral_characters() {
        let found = at_end("// 🙂🙂\r\nscale(float x, float k) => 1\r\nscale(1, ").unwrap();
        assert_eq!(found.parameters, ["float x", "float k"]);
        assert_eq!(found.active, Some(1));
    }
}
