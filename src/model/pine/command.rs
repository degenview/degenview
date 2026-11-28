//! A script a chart is given, and the changes made to the scripts already on it.

use crate::model::pine::instance::{PineInstance, VisualOrder};
use crate::pine::inputs::values::InputValues;
use std::collections::HashMap;

/// A script to put on a chart.
#[derive(Clone, Debug, PartialEq)]
pub struct NewScript {
    /// The saved script it comes from, when it comes from one.
    pub script_id: Option<String>,
    pub name: String,
    pub source: String,
    pub inputs: InputValues,
    pub visible: bool,
}

impl NewScript {
    /// The script as a person adds it: shown, at its default inputs.
    pub fn new(script_id: Option<String>, name: &str, source: &str) -> Self {
        Self {
            script_id,
            name: name.to_owned(),
            source: source.to_owned(),
            inputs: InputValues::new(),
            visible: true,
        }
    }

    /// Another copy of an applied script, at its default inputs.
    pub fn another(instance: &PineInstance) -> Self {
        Self::new(instance.script_id.clone(), &instance.name, &instance.source)
    }
}

/// A change to one script on a chart.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptCommand {
    Remove(u64),
    /// Moves the script to this place among the chart's scripts, which is its draw order.
    MoveTo(u64, usize),
    Change(u64, ScriptChange),
}

/// The settings of an applied script to change; what is `None` stays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptChange {
    pub inputs: Option<InputValues>,
    pub style: Option<HashMap<String, String>>,
    pub visible: Option<bool>,
    pub order: Option<VisualOrder>,
}

impl ScriptCommand {
    pub fn show(id: u64, visible: bool) -> Self {
        Self::Change(
            id,
            ScriptChange {
                visible: Some(visible),
                ..ScriptChange::default()
            },
        )
    }

    pub fn order(id: u64, order: VisualOrder) -> Self {
        Self::Change(
            id,
            ScriptChange {
                order: Some(order),
                ..ScriptChange::default()
            },
        )
    }

    /// The instance the command is about.
    pub fn instance(&self) -> u64 {
        match self {
            Self::Remove(id) | Self::MoveTo(id, _) | Self::Change(id, _) => *id,
        }
    }
}
