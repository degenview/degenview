//! Setting a script's inputs from the chart, as TradingView does with `confirm = true`: the points
//! it asks for, one click each in the order they are declared, then (for a script being added whose
//! other inputs ask too) a dialog. It holds only what was picked or edited; nothing reaches the
//! script until the setup is done, and dropping it changes nothing.

#[cfg(test)]
mod tests;

use crate::pine::compiler::program::InputSchema;
use crate::pine::inputs::points::{self, PointInput, PointValue};
use crate::pine::inputs::values::{self, InputEdit, InputValues};

/// What the setup is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    /// A script being added: only what is marked `confirm` is asked.
    Add,
    /// "Reset points" on an applied script: every point is asked again.
    Repick,
}

/// What the setup waits for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step<'a> {
    /// A click on the chart for this point.
    Pick(&'a PointInput),
    /// The Confirm Inputs dialog.
    Confirm,
    Done,
}

#[derive(Clone, Debug)]
pub struct InputSetup {
    purpose: Purpose,
    schema: InputSchema,
    asked: Vec<PointInput>,
    picked: usize,
    /// The dialog is still to come.
    dialog: bool,
    /// The values given so far, in the order they were given.
    changes: Vec<InputEdit>,
}

impl InputSetup {
    /// The setup a script being added goes through; `None` when nothing is marked `confirm`.
    pub fn adding(schema: &InputSchema) -> Option<Self> {
        let asked: Vec<PointInput> = points::points(schema)
            .into_iter()
            .filter(|point| point.confirm)
            .collect();
        let in_a_point = |id: &str| {
            asked
                .iter()
                .any(|point| point.axes.time() == Some(id) || point.axes.price() == Some(id))
        };
        let dialog = schema
            .inputs
            .iter()
            .any(|input| input.confirm && !in_a_point(&input.id));
        (dialog || !asked.is_empty()).then(|| Self {
            purpose: Purpose::Add,
            schema: schema.clone(),
            asked,
            picked: 0,
            dialog,
            changes: Vec::new(),
        })
    }

    /// The setup that asks for every point again; `None` for a script with none.
    pub fn repicking(schema: &InputSchema) -> Option<Self> {
        let asked = points::points(schema);
        (!asked.is_empty()).then(|| Self {
            purpose: Purpose::Repick,
            schema: schema.clone(),
            asked,
            picked: 0,
            dialog: false,
            changes: Vec::new(),
        })
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    pub fn schema(&self) -> &InputSchema {
        &self.schema
    }

    pub fn step(&self) -> Step<'_> {
        match self.asked.get(self.picked) {
            Some(point) => Step::Pick(point),
            None if self.dialog => Step::Confirm,
            None => Step::Done,
        }
    }

    /// Takes the click for the point being asked.
    pub fn pick(&mut self, time_ms: i64, price: f64) {
        if let Some(point) = self.asked.get(self.picked) {
            self.changes.extend(point.moved_to(time_ms, price));
            self.picked += 1;
        }
    }

    /// Takes a change made in the dialog.
    pub fn edit(&mut self, edit: InputEdit) {
        self.changes.push(edit);
    }

    /// The dialog's Apply.
    pub fn confirm(&mut self) {
        if self.step() == Step::Confirm {
            self.dialog = false;
        }
    }

    /// `base` with everything given so far.
    pub fn values_on(&self, base: &InputValues) -> InputValues {
        let mut values = base.clone();
        for edit in &self.changes {
            values::apply_edit(&mut values, &self.schema, edit.clone());
        }
        values
    }

    /// Where the points already clicked are, to draw while the rest are asked.
    pub fn picked_points(&self) -> Vec<PointValue> {
        let values = self.values_on(&InputValues::new());
        self.asked[..self.picked]
            .iter()
            .map(|point| point.value(&self.schema, &values))
            .collect()
    }
}
