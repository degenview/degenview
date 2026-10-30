//! The static types the checker resolves calls with. [`ValueType`] only says "array" or "object";
//! `Ty` also says `array<float>` or which user type, which is what picks one method overload over
//! another. `Ty::Any` stands for a part the checker does not know (the element of an array it only
//! knows is an array), and fits everything: the checker never reports a mismatch it is not sure of.

use std::collections::HashSet;

use super::ast::{Qualifier, TypeRef};
use super::token::ValueType;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Ty {
    /// `int`, `float`, `bool`, `string`, `color`, `time` and the drawing handles.
    Scalar(ValueType),
    Array(Box<Ty>),
    Matrix(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    /// A user-defined (or library, or `chart.point`) type, by its short name.
    Udt(String),
    Enum(String),
    Any,
}

/// How well an argument fits a parameter; a lower rank is a better overload.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Fit {
    Exact,
    /// An `int` where a `float` is declared.
    Widened,
    /// Something on one side is not known.
    Loose,
}

impl Ty {
    /// The type a parameter, field or `array<T>` argument was annotated with. `enums` are the
    /// script's enum names, which an annotation spells like a user type.
    pub fn from_ref(reference: &TypeRef, enums: &HashSet<String>) -> Self {
        let argument = |index: usize| {
            reference
                .arguments
                .get(index)
                .map_or(Self::Any, |inner| Self::from_ref(inner, enums))
        };
        match reference.kind {
            ValueType::Array => Self::Array(Box::new(argument(0))),
            ValueType::Matrix => Self::Matrix(Box::new(argument(0))),
            ValueType::Map => Self::Map(Box::new(argument(0)), Box::new(argument(1))),
            ValueType::Object => match reference.name.as_deref() {
                Some(name) if enums.contains(short_name(name)) => {
                    Self::Enum(short_name(name).to_owned())
                }
                Some(name) => Self::Udt(short_name(name).to_owned()),
                None => Self::Any,
            },
            kind => Self::Scalar(kind),
        }
    }

    /// The flat kind the older checks read; None for the types they know nothing about.
    pub fn kind(&self) -> Option<ValueType> {
        match self {
            Self::Scalar(kind) => Some(*kind),
            Self::Array(_) => Some(ValueType::Array),
            Self::Matrix(_) => Some(ValueType::Matrix),
            Self::Map(..) => Some(ValueType::Map),
            Self::Udt(_) | Self::Enum(_) | Self::Any => None,
        }
    }

    /// Whether a parameter declared `self` takes an argument of type `argument`.
    pub fn accepts(&self, argument: &Self) -> Option<Fit> {
        match (self, argument) {
            (Self::Any, _) | (_, Self::Any) => Some(Fit::Loose),
            (Self::Scalar(declared), Self::Scalar(given)) => {
                if declared == given {
                    Some(Fit::Exact)
                } else if matches!(
                    (declared, given),
                    (ValueType::Float, ValueType::Int | ValueType::Time)
                        | (ValueType::Int, ValueType::Time)
                        | (ValueType::Time, ValueType::Int)
                ) {
                    Some(Fit::Widened)
                } else {
                    None
                }
            }
            (Self::Array(declared), Self::Array(given))
            | (Self::Matrix(declared), Self::Matrix(given)) => declared.accepts(given),
            (Self::Map(key, value), Self::Map(given_key, given_value)) => {
                let key = key.accepts(given_key)?;
                let value = value.accepts(given_value)?;
                Some(key.max(value))
            }
            (Self::Udt(declared), Self::Udt(given)) | (Self::Enum(declared), Self::Enum(given))
                if declared == given =>
            {
                Some(Fit::Exact)
            }
            _ => None,
        }
    }

    /// The type as a script spells it, for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Self::Scalar(kind) => super::builtin_types::value_type_name(*kind).to_owned(),
            Self::Array(element) => format!("array<{}>", element.describe()),
            Self::Matrix(element) => format!("matrix<{}>", element.describe()),
            Self::Map(key, value) => format!("map<{}, {}>", key.describe(), value.describe()),
            Self::Udt(name) | Self::Enum(name) => name.clone(),
            Self::Any => "unknown".to_owned(),
        }
    }
}

/// `alias.Zone` and `path::Zone` are the library's `Zone`; the last segment names the type.
pub fn short_name(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name)
}

/// Whether an argument of `given` may be passed for a parameter of `declared`: a `simple` or `const`
/// parameter takes nothing that changes from bar to bar. Reference types are always series.
pub fn qualifier_fits(declared: Option<Qualifier>, given: Option<Qualifier>) -> bool {
    match (declared, given) {
        (Some(declared), Some(given)) => given <= declared,
        _ => true,
    }
}

/// Handle to a [`Ty`] in a [`Types`] table, small enough for `Inferred` to stay `Copy`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TyId(u32);

/// Interned types: one entry per distinct type.
#[derive(Default)]
pub struct Types {
    items: Vec<Ty>,
    index: std::collections::HashMap<Ty, TyId>,
}

impl Types {
    pub fn intern(&mut self, ty: Ty) -> TyId {
        if let Some(id) = self.index.get(&ty) {
            return *id;
        }
        let id = TyId(self.items.len() as u32);
        self.items.push(ty.clone());
        self.index.insert(ty, id);
        id
    }

    pub fn get(&self, id: TyId) -> &Ty {
        &self.items[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array(element: ValueType) -> Ty {
        Ty::Array(Box::new(Ty::Scalar(element)))
    }

    #[test]
    fn element_types_tell_arrays_apart() {
        assert_eq!(
            array(ValueType::Float).accepts(&array(ValueType::Float)),
            Some(Fit::Exact)
        );
        assert_eq!(
            array(ValueType::Float).accepts(&array(ValueType::String)),
            None
        );
        assert_eq!(
            array(ValueType::Float).accepts(&Ty::Array(Box::new(Ty::Any))),
            Some(Fit::Loose)
        );
    }

    #[test]
    fn an_int_fits_a_float_but_not_the_reverse() {
        let float = Ty::Scalar(ValueType::Float);
        let int = Ty::Scalar(ValueType::Int);
        assert_eq!(float.accepts(&int), Some(Fit::Widened));
        assert_eq!(int.accepts(&float), None);
    }

    #[test]
    fn user_types_match_by_their_short_name() {
        assert_eq!(
            Ty::Udt("Zone".into()).accepts(&Ty::Udt(short_name("lib.Zone").into())),
            Some(Fit::Exact)
        );
        assert_eq!(
            Ty::Udt("Zone".into()).accepts(&Ty::Udt("Band".into())),
            None
        );
    }

    #[test]
    fn a_series_argument_does_not_fit_a_simple_parameter() {
        assert!(!qualifier_fits(
            Some(Qualifier::Simple),
            Some(Qualifier::Series)
        ));
        assert!(qualifier_fits(
            Some(Qualifier::Simple),
            Some(Qualifier::Input)
        ));
        assert!(qualifier_fits(None, Some(Qualifier::Series)));
    }

    #[test]
    fn interning_gives_equal_types_one_handle() {
        let mut types = Types::default();
        let a = types.intern(array(ValueType::Int));
        let b = types.intern(array(ValueType::Int));
        let c = types.intern(array(ValueType::Float));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(types.get(c), &array(ValueType::Float));
    }
}
