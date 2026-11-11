//! Static checks that run after parsing and before a script executes: the type checker, name and structure rules, and what each declaration may be called.

pub mod builtin_types;
pub mod callables;
pub mod declaration_rules;
pub mod identifier_rules;
pub mod ignored_features;
pub mod name_check;
pub mod structure_validator;
pub mod type_checker;
pub mod types;
