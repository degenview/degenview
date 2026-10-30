//! The functions and methods a script declares, and which of them a call selects. A callable is a
//! top-level `Statement::Function`; its id is its position among them, so the checker's choice can be
//! stored as a number and the session, which counts the same statements, finds the same function.
//!
//! A method is an ordinary callable whose first parameter is the receiver: `x.f(a)` and `f(x, a)`
//! select by the same rules over the same arguments. Overloads are ranked by how exactly each
//! argument fits; a tie of exact fits is ambiguous, a tie that rests on something the checker does
//! not know is left to the run (`Resolution::Dynamic`).

use std::collections::{HashMap, HashSet};

use super::ast::{FunctionKind, Qualifier, Statement};
use super::token::SourceRange;
use super::types::{Fit, Ty, qualifier_fits};

/// What a worse type fit adds to a rank, which outweighs any difference in qualifier (at most 3).
const TYPE_STEP: u32 = 10;

fn level(qualifier: Qualifier) -> u32 {
    match qualifier {
        Qualifier::Constant => 0,
        Qualifier::Input => 1,
        Qualifier::Simple => 2,
        Qualifier::Series => 3,
    }
}

pub struct ParamSig {
    pub name: String,
    /// None when the parameter has no type keyword, which takes anything.
    pub ty: Option<Ty>,
    pub qualifier: Option<Qualifier>,
    pub optional: bool,
}

pub struct Callable {
    pub name: String,
    pub kind: FunctionKind,
    pub params: Vec<ParamSig>,
    pub range: SourceRange,
}

/// One argument of a call as the checker sees it. For `x.f(a)` the receiver `x` comes first.
pub struct Given {
    pub label: Option<String>,
    pub ty: Ty,
    pub qualifier: Option<Qualifier>,
}

/// Why a callable cannot take a call.
pub enum NoFit {
    /// The wrong number of arguments, or one needed is missing.
    Arity,
    /// A named argument no parameter has.
    Label(String),
    Type {
        index: usize,
        expected: Ty,
        given: Ty,
    },
    Qualifier {
        index: usize,
        expected: Qualifier,
    },
}

pub enum Resolution {
    Selected(usize),
    /// Several fit equally well on what is known; which one runs depends on the values.
    Dynamic,
    /// Several fit equally well and every argument is known.
    Ambiguous(Vec<usize>),
    /// The name is declared but no declaration takes this call.
    NoFit(Vec<(usize, NoFit)>),
}

#[derive(Default)]
pub struct Callables {
    items: Vec<Callable>,
    by_name: HashMap<String, Vec<usize>>,
    by_position: HashMap<usize, usize>,
}

impl Callables {
    /// The top-level functions and methods of `statements`, in order. `enums` are the script's enum
    /// names, which a parameter annotation spells like a user type.
    pub fn collect(statements: &[Statement], enums: &HashSet<String>) -> Self {
        let mut table = Self::default();
        for statement in statements {
            let Statement::Function {
                name,
                kind,
                parameters,
                range,
                ..
            } = statement
            else {
                continue;
            };
            let id = table.items.len();
            table.items.push(Callable {
                name: name.clone(),
                kind: *kind,
                params: parameters
                    .iter()
                    .map(|parameter| ParamSig {
                        name: parameter.name.clone(),
                        ty: parameter
                            .type_ref
                            .as_ref()
                            .map(|reference| Ty::from_ref(reference, enums)),
                        qualifier: parameter.qualifier,
                        optional: parameter.default_value.is_some(),
                    })
                    .collect(),
                range: *range,
            });
            table.by_name.entry(name.clone()).or_default().push(id);
            table.by_position.insert(range.start.offset, id);
        }
        table
    }

    pub fn get(&self, id: usize) -> &Callable {
        &self.items[id]
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The callable declared at this source offset.
    pub fn at(&self, offset: usize) -> Option<usize> {
        self.by_position.get(&offset).copied()
    }

    pub fn declared(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// Selects among the declarations of `name` for a call with these arguments. `method_syntax`
    /// (`x.name(…)`) only considers methods.
    pub fn resolve(&self, name: &str, given: &[Given], method_syntax: bool) -> Resolution {
        let mut fitting: Vec<(usize, u32, bool)> = Vec::new();
        let mut refused = Vec::new();
        for &id in self.declared(name) {
            let callable = &self.items[id];
            if method_syntax && callable.kind != FunctionKind::Method {
                continue;
            }
            match callable.fit(given) {
                Ok((rank, loose)) => fitting.push((id, rank, loose)),
                Err(reason) => refused.push((id, reason)),
            }
        }
        let Some(best) = fitting.iter().map(|(_, rank, _)| *rank).min() else {
            return Resolution::NoFit(refused);
        };
        let tied: Vec<&(usize, u32, bool)> = fitting
            .iter()
            .filter(|(_, rank, _)| *rank == best)
            .collect();
        match tied.as_slice() {
            [(id, ..)] => Resolution::Selected(*id),
            _ if tied.iter().any(|(.., loose)| *loose) => Resolution::Dynamic,
            _ => Resolution::Ambiguous(tied.iter().map(|(id, ..)| *id).collect()),
        }
    }
}

impl Callable {
    /// How well the arguments fit: a rank to compare overloads by (lower is better) and whether any
    /// part of the fit rests on something unknown.
    fn fit(&self, given: &[Given]) -> Result<(u32, bool), NoFit> {
        let mut supplied = vec![false; self.params.len()];
        let mut position = 0;
        let mut rank = 0;
        let mut loose = false;
        for (index, argument) in given.iter().enumerate() {
            let slot = match &argument.label {
                Some(label) => self
                    .params
                    .iter()
                    .position(|parameter| &parameter.name == label)
                    .ok_or_else(|| NoFit::Label(label.clone()))?,
                None => {
                    position += 1;
                    position - 1
                }
            };
            let Some(parameter) = self.params.get(slot) else {
                return Err(NoFit::Arity);
            };
            if supplied[slot] {
                return Err(NoFit::Arity);
            }
            supplied[slot] = true;
            if let Some(expected) = &parameter.ty {
                match expected.accepts(&argument.ty) {
                    Some(Fit::Exact) => {}
                    Some(Fit::Widened) => rank += TYPE_STEP,
                    Some(Fit::Loose) => {
                        rank += 2 * TYPE_STEP;
                        loose = true;
                    }
                    None => {
                        return Err(NoFit::Type {
                            index,
                            expected: expected.clone(),
                            given: argument.ty.clone(),
                        });
                    }
                }
                // A reference type is always series, whatever it is declared as.
                if matches!(expected, Ty::Scalar(_)) {
                    let declared = parameter.qualifier.unwrap_or(Qualifier::Series);
                    if !qualifier_fits(Some(declared), argument.qualifier) {
                        return Err(NoFit::Qualifier {
                            index,
                            expected: declared,
                        });
                    }
                    // Of two overloads that take the argument, the one asking for less wins.
                    if let Some(given) = argument.qualifier {
                        rank += level(declared) - level(given);
                    }
                }
            } else {
                rank += 2 * TYPE_STEP;
                loose = true;
            }
        }
        let complete = self
            .params
            .iter()
            .zip(&supplied)
            .all(|(parameter, given)| *given || parameter.optional);
        if complete {
            Ok((rank, loose))
        } else {
            Err(NoFit::Arity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{SourcePosition, ValueType};

    fn callable(kind: FunctionKind, params: &[(&str, Option<Ty>, bool)]) -> Callable {
        Callable {
            name: "f".into(),
            kind,
            params: params
                .iter()
                .map(|(name, ty, optional)| ParamSig {
                    name: (*name).into(),
                    ty: ty.clone(),
                    qualifier: None,
                    optional: *optional,
                })
                .collect(),
            range: SourceRange {
                start: SourcePosition::default(),
                end: SourcePosition::default(),
            },
        }
    }

    fn given(ty: Ty) -> Given {
        Given {
            label: None,
            ty,
            qualifier: Some(Qualifier::Series),
        }
    }

    fn table(items: Vec<Callable>) -> Callables {
        let mut table = Callables::default();
        for item in items {
            let id = table.items.len();
            table.by_name.entry(item.name.clone()).or_default().push(id);
            table.items.push(item);
        }
        table
    }

    fn scalar(kind: ValueType) -> Ty {
        Ty::Scalar(kind)
    }

    #[test]
    fn the_exact_overload_beats_a_widened_one() {
        let overloads = table(vec![
            callable(
                FunctionKind::Method,
                &[("v", Some(scalar(ValueType::Float)), false)],
            ),
            callable(
                FunctionKind::Method,
                &[("v", Some(scalar(ValueType::Int)), false)],
            ),
        ]);
        let Resolution::Selected(chosen) =
            overloads.resolve("f", &[given(scalar(ValueType::Int))], true)
        else {
            panic!("not selected");
        };
        assert_eq!(chosen, 1);
    }

    #[test]
    fn an_unknown_argument_between_equal_overloads_is_left_to_the_run() {
        let overloads = table(vec![
            callable(
                FunctionKind::Method,
                &[("v", Some(scalar(ValueType::Float)), false)],
            ),
            callable(
                FunctionKind::Method,
                &[("v", Some(scalar(ValueType::String)), false)],
            ),
        ]);
        assert!(matches!(
            overloads.resolve("f", &[given(Ty::Any)], true),
            Resolution::Dynamic
        ));
    }

    #[test]
    fn optional_parameters_may_be_left_out_and_required_ones_may_not() {
        let overloads = table(vec![callable(
            FunctionKind::Method,
            &[
                ("v", Some(scalar(ValueType::Float)), false),
                ("n", Some(scalar(ValueType::Int)), true),
            ],
        )]);
        assert!(matches!(
            overloads.resolve("f", &[given(scalar(ValueType::Float))], true),
            Resolution::Selected(0)
        ));
        assert!(matches!(
            overloads.resolve("f", &[], true),
            Resolution::NoFit(_)
        ));
    }

    #[test]
    fn a_label_no_parameter_has_is_refused() {
        let overloads = table(vec![callable(
            FunctionKind::Method,
            &[("v", Some(scalar(ValueType::Float)), false)],
        )]);
        let mut argument = given(scalar(ValueType::Float));
        argument.label = Some("nope".into());
        let Resolution::NoFit(reasons) = overloads.resolve("f", &[argument], true) else {
            panic!("fitted");
        };
        assert!(matches!(reasons[0].1, NoFit::Label(_)));
    }

    #[test]
    fn method_syntax_ignores_plain_functions() {
        let overloads = table(vec![callable(
            FunctionKind::Function,
            &[("v", Some(scalar(ValueType::Float)), false)],
        )]);
        assert!(matches!(
            overloads.resolve("f", &[given(scalar(ValueType::Float))], true),
            Resolution::NoFit(_)
        ));
    }
}
