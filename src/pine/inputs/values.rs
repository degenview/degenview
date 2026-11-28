//! The values a user gave a script's inputs, as overrides of the declared defaults.

use crate::pine::compiler::program::{InputSchema, InputValue};
use std::collections::HashMap;

/// Override map by input id; an input with no entry holds its default.
pub type InputValues = HashMap<String, InputValue>;

/// One change to a script's inputs.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEdit {
    Set(String, InputValue),
    Reset,
}

/// Keeps only the overrides whose input still exists with the same type of value.
pub fn prune_values(values: &InputValues, schema: &InputSchema) -> InputValues {
    values
        .iter()
        .filter_map(|(id, value)| {
            let input = schema.inputs.iter().find(|input| input.id == *id)?;
            let value = value.clone().typed(input.kind);
            (std::mem::discriminant(&input.default_value) == std::mem::discriminant(&value))
                .then(|| (id.clone(), value))
        })
        .collect()
}

/// Applies a change: a value equal to the default is no override at all.
pub fn apply_edit(values: &mut InputValues, schema: &InputSchema, edit: InputEdit) {
    match edit {
        InputEdit::Reset => values.clear(),
        InputEdit::Set(id, value) => {
            let default = schema
                .inputs
                .iter()
                .find(|input| input.id == id)
                .map(|input| input.default_value.clone());
            if default.as_ref() == Some(&value) {
                values.remove(&id);
            } else {
                values.insert(id, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

    fn schema(body: &str) -> InputSchema {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}\nplot(1)");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        program.input_schema
    }

    #[test]
    fn values_for_removed_or_retyped_inputs_are_pruned() {
        let schema = schema("len = input.int(1, \"Length\")");
        let mut values = InputValues::new();
        values.insert("len".into(), InputValue::Int(9));
        values.insert("gone".into(), InputValue::Int(1));
        assert_eq!(prune_values(&values, &schema).len(), 1);
        values.insert("len".into(), InputValue::Float(1.0));
        assert!(prune_values(&values, &schema).is_empty(), "type changed");
    }

    #[test]
    fn a_whole_number_kept_for_a_float_input_is_a_float() {
        let schema = schema("level = input.price(100.5, \"Level\")");
        let values = InputValues::from([("level".to_owned(), InputValue::Int(120))]);
        assert_eq!(
            prune_values(&values, &schema).get("level"),
            Some(&InputValue::Float(120.0))
        );
    }

    #[test]
    fn a_value_equal_to_the_default_is_no_override() {
        let schema = schema("len = input.int(14, \"Length\")");
        let mut values = InputValues::new();
        let set = |values: &mut InputValues, value| {
            let edit = InputEdit::Set("len".into(), InputValue::Int(value));
            apply_edit(values, &schema, edit);
        };
        set(&mut values, 30);
        assert_eq!(values.get("len"), Some(&InputValue::Int(30)));
        set(&mut values, 14);
        assert!(values.is_empty(), "back at the default");
        set(&mut values, 5);
        apply_edit(&mut values, &schema, InputEdit::Reset);
        assert!(values.is_empty());
    }
}
