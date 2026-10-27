//! The arguments of an `input.*` call, found by name or by position. Pine lets a script pass them
//! either way (`input.int(5, "Length", 1, 100)`), and each function has its own order, so a
//! positional argument is matched to the parameter in its place.

use super::ast::{Argument, Expression, Literal};
use super::runtime_value::RuntimeValue;

const PLAIN: &[&str] = &[
    "defval", "title", "tooltip", "inline", "group", "display", "active",
];
const SOURCE: &[&str] = &[
    "defval", "title", "tooltip", "inline", "group", "display", "active", "confirm",
];
const RANGE: &[&str] = &[
    "defval", "title", "minval", "maxval", "step", "tooltip", "inline", "group", "confirm",
    "display", "active",
];
const OPTIONS: &[&str] = &[
    "defval", "title", "options", "tooltip", "inline", "group", "confirm", "display", "active",
];
const CONFIRMED: &[&str] = &[
    "defval", "title", "tooltip", "inline", "group", "confirm", "display", "active",
];
const TEXT_AREA: &[&str] = &[
    "defval", "title", "tooltip", "group", "confirm", "display", "active",
];

/// Every `input.*` function, which the editor offers.
pub const FUNCTIONS: [&str; 13] = [
    "input.int",
    "input.float",
    "input.bool",
    "input.color",
    "input.string",
    "input.symbol",
    "input.timeframe",
    "input.session",
    "input.source",
    "input.time",
    "input.price",
    "input.text_area",
    "input.enum",
];

/// The parameters of `function` in Pine's order. `input.int` and `input.float` take either a
/// range (`minval`, `maxval`, `step`) or a list of `options` in the third place, told apart by what
/// the call passes there.
fn parameters(function: &str, args: &[Argument]) -> &'static [&'static str] {
    match function {
        "input.int" | "input.float" => {
            let third = args.iter().filter(|a| a.name.is_none()).nth(2);
            let options_named = args.iter().any(|a| a.name.as_deref() == Some("options"));
            if options_named || third.is_some_and(|a| matches!(a.value, Expression::Tuple(..))) {
                OPTIONS
            } else {
                RANGE
            }
        }
        "input.string" | "input.timeframe" | "input.session" | "input.enum" => OPTIONS,
        "input.bool" | "input.color" | "input.symbol" | "input.time" | "input.price" => CONFIRMED,
        "input.text_area" => TEXT_AREA,
        "input.source" => SOURCE,
        _ => PLAIN,
    }
}

pub struct InputArguments<'a> {
    args: &'a [Argument],
    parameters: &'static [&'static str],
}

impl<'a> InputArguments<'a> {
    pub fn new(function: &str, args: &'a [Argument]) -> Self {
        Self {
            args,
            parameters: parameters(function, args),
        }
    }

    /// The argument for `key`: the one named so, else the unnamed one in that parameter's place.
    pub fn find(&self, key: &str) -> Option<&'a Argument> {
        if let Some(named) = self.args.iter().find(|a| a.name.as_deref() == Some(key)) {
            return Some(named);
        }
        let place = self
            .parameters
            .iter()
            .position(|parameter| *parameter == key)?;
        self.args.iter().filter(|a| a.name.is_none()).nth(place)
    }

    pub fn string(
        &self,
        key: &str,
        constant: &dyn Fn(&Expression) -> Option<RuntimeValue>,
    ) -> Option<String> {
        match constant(&self.find(key)?.value)? {
            RuntimeValue::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn number(
        &self,
        key: &str,
        constant: &dyn Fn(&Expression) -> Option<RuntimeValue>,
    ) -> Option<f64> {
        constant(&self.find(key)?.value)?.number()
    }

    /// Whether the call passes a literal `true` for `key`.
    pub fn flag(&self, key: &str) -> bool {
        matches!(
            self.find(key).map(|a| &a.value),
            Some(Expression::Literal(Literal::Bool(true), _))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::SourceRange;

    fn literal(value: Literal) -> Expression {
        Expression::Literal(value, SourceRange::ZERO)
    }

    fn positional(value: Expression) -> Argument {
        Argument {
            name: None,
            name_range: None,
            value,
        }
    }

    fn named(name: &str, value: Expression) -> Argument {
        Argument {
            name: Some(name.to_owned()),
            name_range: None,
            value,
        }
    }

    fn constant(expression: &Expression) -> Option<RuntimeValue> {
        match expression {
            Expression::Literal(Literal::Int(value), _) => Some(RuntimeValue::Int(*value)),
            Expression::Literal(Literal::String(text), _) => {
                Some(RuntimeValue::String(text.clone()))
            }
            _ => None,
        }
    }

    #[test]
    fn a_positional_argument_finds_the_parameter_in_its_place() {
        let args = vec![
            positional(literal(Literal::Int(5))),
            positional(literal(Literal::String("Length".into()))),
            positional(literal(Literal::Int(1))),
            positional(literal(Literal::Int(100))),
            positional(literal(Literal::Int(2))),
            positional(literal(Literal::String("hint".into()))),
        ];
        let inputs = InputArguments::new("input.int", &args);
        assert_eq!(inputs.number("minval", &constant), Some(1.0));
        assert_eq!(inputs.number("maxval", &constant), Some(100.0));
        assert_eq!(inputs.number("step", &constant), Some(2.0));
        assert_eq!(inputs.string("tooltip", &constant), Some("hint".into()));
        assert_eq!(inputs.string("title", &constant), Some("Length".into()));
        assert_eq!(inputs.string("group", &constant), None);
    }

    #[test]
    fn a_named_argument_wins_and_the_tuple_in_third_place_means_options() {
        let list = Expression::Tuple(vec![literal(Literal::Int(1))], SourceRange::ZERO);
        let args = vec![
            positional(literal(Literal::Int(1))),
            positional(literal(Literal::String("Pick".into()))),
            positional(list),
            named("tooltip", literal(Literal::String("named".into()))),
        ];
        let inputs = InputArguments::new("input.int", &args);
        assert!(inputs.find("options").is_some());
        assert_eq!(inputs.number("minval", &constant), None, "not a range call");
        assert_eq!(inputs.string("tooltip", &constant), Some("named".into()));
    }

    #[test]
    fn each_function_has_its_own_order() {
        let args = vec![
            positional(literal(Literal::String("x".into()))),
            positional(literal(Literal::String("Notes".into()))),
            positional(literal(Literal::String("tip".into()))),
            positional(literal(Literal::String("grp".into()))),
        ];
        // `input.text_area` has no `inline`: its fourth parameter is the group.
        let area = InputArguments::new("input.text_area", &args);
        assert_eq!(area.string("group", &constant), Some("grp".into()));
        let plain = InputArguments::new("input.bool", &args);
        assert_eq!(plain.string("inline", &constant), Some("grp".into()));
        assert!(!plain.flag("confirm"));
    }
}
