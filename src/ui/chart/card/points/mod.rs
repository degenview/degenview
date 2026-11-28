//! The card's part in setting a script's inputs from the chart. The workspace says each frame what
//! the card shows (scripts still waiting for their inputs, the point being asked) and takes back
//! what the user did; the card holds no state of the setup itself.

mod markers;
mod pick;
#[cfg(test)]
mod tests;

use super::*;
use crate::host::pine::declaration::ScriptDeclaration;
use crate::pine::inputs::points::{PointAxes, PointValue};

/// A script being added that waits for its inputs: shown on the card, not yet on the chart.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingScript {
    pub id: u64,
    pub name: String,
    pub overlay: bool,
}

/// The point a click on this card sets.
#[derive(Clone, Debug, PartialEq)]
pub struct PickView {
    pub axes: PointAxes,
    /// The script pane whose scale the click reads; `None` for the price chart.
    pub pane: Option<u64>,
    /// The points already clicked.
    pub picked: Vec<PointValue>,
    /// What the pill at the bottom of the plot asks; `None` once the user put it away.
    pub prompt: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetupView {
    pub pending: Vec<PendingScript>,
    pub pick: Option<PickView>,
}

/// What the user did about a setup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SetupEvent {
    Picked { time_ms: i64, price: f64 },
    PromptClosed,
}

impl ChartCard {
    pub fn set_setup_view(&mut self, view: SetupView) {
        self.setup = view;
    }

    pub fn take_setup_events(&mut self) -> Vec<SetupEvent> {
        std::mem::take(&mut self.setup_events)
    }

    pub(in crate::ui) fn pending_scripts(&self) -> &[PendingScript] {
        &self.setup.pending
    }

    pub(super) fn pending_script(&self, id: u64) -> Option<&PendingScript> {
        self.setup.pending.iter().find(|pending| pending.id == id)
    }

    /// The name of a script on the card, applied or waiting.
    pub(super) fn script_name(&self, id: u64) -> &str {
        self.instances
            .iter()
            .find(|instance| instance.id == id)
            .map(|instance| instance.name.as_str())
            .or_else(|| self.pending_script(id).map(|pending| pending.name.as_str()))
            .unwrap_or_default()
    }

    /// What the script of `source` declares.
    pub fn declaration(&self, source: &str) -> Arc<ScriptDeclaration> {
        self.declarations.of(source)
    }

    /// Whether an applied script has points "Reset points" can ask for again.
    pub fn has_point_inputs(&self, id: u64) -> bool {
        self.instances
            .iter()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| !self.declaration(&instance.source).points.is_empty())
    }
}
