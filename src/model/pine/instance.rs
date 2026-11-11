//! A Pine script applied to a chart. A chart holds several, in order, each with its own inputs and
//! visibility, so the same script can run twice with different settings. A port of
//! `ChartScriptInstance`; the source is stored with the instance (as `PineAlertSubscription` does),
//! so a restored chart needs no script store to run it.

use crate::pine::program::InputValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An input value as persisted (the engine's `InputValue` carries no serde of its own).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum StoredInput {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Source(String),
}

impl From<&InputValue> for StoredInput {
    fn from(value: &InputValue) -> Self {
        match value {
            InputValue::Int(v) => Self::Int(*v),
            InputValue::Float(v) => Self::Float(*v),
            InputValue::Bool(v) => Self::Bool(*v),
            InputValue::String(v) => Self::String(v.clone()),
            InputValue::Color(v) => Self::Color(*v),
            InputValue::Source(v) => Self::Source(v.clone()),
        }
    }
}

impl From<StoredInput> for InputValue {
    fn from(value: StoredInput) -> Self {
        match value {
            StoredInput::Int(v) => Self::Int(v),
            StoredInput::Float(v) => Self::Float(v),
            StoredInput::Bool(v) => Self::Bool(v),
            StoredInput::String(v) => Self::String(v),
            StoredInput::Color(v) => Self::Color(v),
            StoredInput::Source(v) => Self::Source(v),
        }
    }
}

fn visible_by_default() -> bool {
    true
}

/// Where an overlay script's visuals sit against the chart's own candles.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualOrder {
    /// As the script declares it (`behind_chart`).
    #[default]
    Script,
    /// In front of the candles, whatever the script declares.
    Front,
    /// Behind the candles, whatever the script declares.
    Behind,
}

impl VisualOrder {
    /// Whether the visuals go behind the chart for a script that declares `behind_chart`.
    pub fn behind(self, declared_behind: bool) -> bool {
        match self {
            Self::Script => declared_behind,
            Self::Front => false,
            Self::Behind => true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PineInstance {
    /// Unique within its chart; alerts and rows refer to an instance by it.
    pub id: u64,
    /// The saved script it came from, when it came from one.
    #[serde(default)]
    pub script_id: Option<String>,
    /// The script's name at the time it was added.
    pub name: String,
    pub source: String,
    /// The user's input overrides by input id; an input with no entry runs at its default.
    #[serde(default)]
    pub inputs: HashMap<String, StoredInput>,
    #[serde(default = "visible_by_default")]
    pub visible: bool,
    /// The user's style choices (`kind.id.attribute` → value) for the script's outputs; an output
    /// with no entry is drawn as the script has it.
    #[serde(default)]
    pub style_overrides: HashMap<String, String>,
    /// Where the script's visuals sit against the chart's candles.
    #[serde(default)]
    pub visual_order: VisualOrder,
    /// The script's pane is folded to its title strip. The script still runs: this is layout.
    #[serde(default)]
    pub pane_collapsed: bool,
}

impl PineInstance {
    pub fn new(id: u64, script_id: Option<String>, name: &str, source: &str) -> Self {
        Self {
            id,
            script_id,
            name: name.to_owned(),
            source: source.to_owned(),
            inputs: HashMap::new(),
            visible: true,
            style_overrides: HashMap::new(),
            visual_order: VisualOrder::Script,
            pane_collapsed: false,
        }
    }

    /// The overrides in the engine's own type.
    pub fn input_values(&self) -> HashMap<String, InputValue> {
        self.inputs
            .iter()
            .map(|(id, value)| (id.clone(), value.clone().into()))
            .collect()
    }

    pub fn set_input_values(&mut self, values: &HashMap<String, InputValue>) {
        self.inputs = values
            .iter()
            .map(|(id, value)| (id.clone(), value.into()))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instance_round_trips_with_its_inputs_and_defaults_missing_fields() {
        let mut instance = PineInstance::new(3, Some("abc".into()), "RSI", "plot(close)");
        let mut values = HashMap::new();
        values.insert("length".to_owned(), InputValue::Int(21));
        values.insert("tint".to_owned(), InputValue::Color(0x11_22_33_ff));
        instance.set_input_values(&values);
        instance.visible = false;
        let json = serde_json::to_string(&instance).unwrap();
        let back: PineInstance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, instance);
        assert_eq!(back.input_values(), values);

        // A row written without optional fields still decodes: visible, no overrides.
        let minimal: PineInstance =
            serde_json::from_str(r#"{"id":1,"name":"x","source":"plot(close)"}"#).unwrap();
        assert!(minimal.visible && minimal.inputs.is_empty() && minimal.script_id.is_none());
        assert!(minimal.style_overrides.is_empty());

        instance
            .style_overrides
            .insert("plot.0.visible".into(), "false".into());
        let json = serde_json::to_string(&instance).unwrap();
        assert_eq!(
            serde_json::from_str::<PineInstance>(&json).unwrap(),
            instance
        );
    }

    #[test]
    fn visual_order_defaults_to_the_script_and_overrides_it_either_way() {
        let minimal: PineInstance =
            serde_json::from_str(r#"{"id":1,"name":"x","source":"plot(close)"}"#).unwrap();
        assert_eq!(minimal.visual_order, VisualOrder::Script);
        assert!(VisualOrder::Script.behind(true));
        assert!(!VisualOrder::Script.behind(false));
        assert!(!VisualOrder::Front.behind(true));
        assert!(VisualOrder::Behind.behind(false));
        let mut instance = PineInstance::new(1, None, "x", "plot(close)");
        instance.visual_order = VisualOrder::Behind;
        let back: PineInstance =
            serde_json::from_str(&serde_json::to_string(&instance).unwrap()).unwrap();
        assert_eq!(back.visual_order, VisualOrder::Behind);
    }
}
