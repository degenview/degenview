//! Bookkeeping between ticks and bars: what `varip` keeps across an unconfirmed tick, the
//! histories a confirmed bar commits, and the instruction budget and deadline.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{INTERNAL_PREFIX, IntrabarObject, RunError, STRATEGY_SERIES, Session};
use crate::model::Kline;
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::types::RuntimeContext;
use crate::pine::runtime::value::{RefKind, RuntimeValue};
use crate::pine::strategy::builtins;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

impl Session {
    /// Keeps what a `varip` variable points at, the `varip` fields of every object, and everything
    /// reachable from either through arrays, maps, matrices and objects, so the next tick of this
    /// bar can have them back after the rollback. Costs nothing for a script with neither.
    pub(super) fn capture_intrabar_objects(&mut self) {
        self.intrabar_objects.clear();
        // The references still to follow; `seen` keeps a cycle from being followed twice.
        let mut pending: Vec<(RefKind, usize)> = Vec::new();
        for value in self.intrabar.values() {
            if let RuntimeValue::Ref(kind, id) = value {
                pending.push((*kind, *id));
            }
        }
        if !self.varip_fields.is_empty() {
            for (id, object) in &self.working.instances {
                let Some((_, names)) = self
                    .varip_fields
                    .iter()
                    .find(|(type_name, _)| *type_name == object.type_name)
                else {
                    continue;
                };
                let fields: Vec<(String, RuntimeValue)> = names
                    .iter()
                    .filter_map(|name| Some((name.clone(), object.fields.get(name)?.clone())))
                    .collect();
                for (_, value) in &fields {
                    if let RuntimeValue::Ref(kind, target) = value {
                        pending.push((*kind, *target));
                    }
                }
                self.intrabar_objects.push(IntrabarObject::Fields {
                    id: *id,
                    type_name: object.type_name.clone(),
                    fields,
                });
            }
        }
        let mut seen: HashSet<(RefKind, usize)> = HashSet::new();
        while let Some((kind, id)) = pending.pop() {
            if !seen.insert((kind, id)) {
                continue;
            }
            let captured = match kind {
                RefKind::Array => self.working.arrays.get(&id).map(|items| {
                    push_references(items.iter(), &mut pending);
                    IntrabarObject::Array(id, items.clone())
                }),
                RefKind::Map => self.working.maps.get(&id).map(|map| {
                    push_references(map.values().iter(), &mut pending);
                    IntrabarObject::Map(id, map.clone())
                }),
                RefKind::Matrix => self.working.matrices.get(&id).map(|matrix| {
                    push_references(matrix.all_elements().iter(), &mut pending);
                    IntrabarObject::Matrix(id, matrix.clone())
                }),
                RefKind::Object => self.working.instances.get(&id).map(|object| {
                    push_references(object.fields.values(), &mut pending);
                    IntrabarObject::Object(id, object.clone())
                }),
                _ => None,
            };
            if let Some(captured) = captured {
                self.intrabar_objects.push(captured);
            }
        }
    }

    /// Puts back what `capture_intrabar_objects` kept. The fields of an object only go back into an
    /// object the committed state has: one made on the tick that was rolled back is gone, as it is
    /// in TradingView, and its handle may by now belong to another object.
    pub(super) fn restore_intrabar_objects(&mut self) {
        for captured in std::mem::take(&mut self.intrabar_objects) {
            match captured {
                IntrabarObject::Array(id, items) => {
                    self.working.arrays.insert(id, items);
                    self.working.next_reference = self.working.next_reference.max(id + 1);
                }
                IntrabarObject::Map(id, map) => {
                    self.working.maps.insert(id, map);
                    self.working.next_reference = self.working.next_reference.max(id + 1);
                }
                IntrabarObject::Matrix(id, matrix) => {
                    self.working.matrices.insert(id, matrix);
                    self.working.next_reference = self.working.next_reference.max(id + 1);
                }
                IntrabarObject::Object(id, object) => {
                    self.working.instances.insert(id, object);
                    self.working.next_reference = self.working.next_reference.max(id + 1);
                }
                IntrabarObject::Fields {
                    id,
                    type_name,
                    fields,
                } => {
                    if let Some(object) = self.working.instances.get_mut(&id)
                        && object.type_name == type_name
                    {
                        for (name, value) in fields {
                            object.fields.insert(name, value);
                        }
                    }
                }
            }
        }
    }

    /// The value of each builtin variable the script reads a history of, as this bar left it. A name
    /// the script declares itself is a variable and is recorded with the others.
    pub(super) fn read_history_builtins(
        &mut self,
        context: &mut RuntimeContext,
    ) -> Vec<(String, RuntimeValue)> {
        if self.history_builtins.is_empty() {
            return Vec::new();
        }
        let names = std::mem::take(&mut self.history_builtins);
        let mut values = Vec::with_capacity(names.len());
        for name in &names {
            if self.working.variables.contains_key(name) {
                continue;
            }
            let value = self
                .resolve_identifier(name, SourceRange::ZERO, context)
                .unwrap_or(RuntimeValue::Na);
            values.push((name.clone(), value));
        }
        self.history_builtins = names;
        values
    }

    /// Records the bar in every history. `builtins` are values of builtin variables the script
    /// indexes (see `read_history_builtins`); a plain call from elsewhere passes none.
    pub(crate) fn commit_histories(
        &mut self,
        bar: Kline,
        releasing_committed: bool,
        builtins: Vec<(String, RuntimeValue)>,
    ) {
        let mut series: FastMap<String, RuntimeValue> = FastMap::default();
        series.insert("open".to_owned(), RuntimeValue::Float(bar.open));
        series.insert("high".to_owned(), RuntimeValue::Float(bar.high));
        series.insert("low".to_owned(), RuntimeValue::Float(bar.low));
        series.insert("close".to_owned(), RuntimeValue::Float(bar.close));
        series.insert("volume".to_owned(), RuntimeValue::Float(bar.volume));
        series.insert(
            "hl2".to_owned(),
            RuntimeValue::Float((bar.high + bar.low) / 2.0),
        );
        series.insert(
            "hlc3".to_owned(),
            RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0),
        );
        series.insert(
            "ohlc4".to_owned(),
            RuntimeValue::Float((bar.open + bar.high + bar.low + bar.close) / 4.0),
        );
        series.insert("time".to_owned(), RuntimeValue::Int(bar.open_time));
        series.insert(
            "time_close".to_owned(),
            RuntimeValue::Int(bar.open_time + (self.bar_seconds * 1000.0) as i64),
        );
        series.insert(
            "bar_index".to_owned(),
            RuntimeValue::Int(self.working.bar_index),
        );
        if self.program.declaration.script_type
            == crate::pine::compiler::program::ScriptType::Strategy
        {
            for name in STRATEGY_SERIES {
                if let Some(value) = builtins::strategy_value(name, &self.working.broker, bar.close)
                {
                    series.insert(name.to_owned(), value);
                }
            }
        }

        let mut histories = std::mem::take(&mut self.working.histories);
        if releasing_committed {
            self.committed.histories.clear();
        }
        let builtin: HashSet<String> = series.keys().cloned().collect();
        let own = self
            .working
            .variables
            .iter()
            .filter(|(name, _)| !builtin.contains(*name))
            .map(|(name, value)| (name.clone(), value.clone()));
        for (name, value) in series.into_iter().chain(builtins).chain(own) {
            if !name.starts_with(INTERNAL_PREFIX) {
                histories.entry(name).or_default().push(value);
            }
        }
        let expression_keys: HashSet<String> = histories
            .keys()
            .filter(|k| k.starts_with(INTERNAL_PREFIX))
            .cloned()
            .chain(self.working.expression_values.keys().cloned())
            .collect();
        for key in expression_keys {
            let value = self
                .working
                .expression_values
                .get(&key)
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            histories.entry(key).or_default().push(value);
        }
        self.working.histories = histories;
        self.working.expression_values.clear();
    }

    pub(super) fn budget(&mut self) -> Result<(), Diagnostic> {
        self.working.instructions += 1;
        if self.working.instructions == 1 || self.working.instructions.is_multiple_of(1024) {
            self.check_interruption().map_err(|error| match error {
                RunError::Script(diagnostic) => diagnostic,
                RunError::Rejection => unreachable!("interruption is always a diagnostic"),
            })?;
        }
        if self.working.instructions > self.limits.instructions_per_bar {
            return Err(Diagnostic::error(
                "PINE8004",
                DiagnosticCategory::Resource,
                "Per-bar instruction limit exceeded.",
                SourceRange::ZERO,
            ));
        }
        Ok(())
    }

    pub(super) fn start_deadline(&mut self) {
        let seconds = if self.limits.deadline_seconds.is_finite() {
            self.limits.deadline_seconds.max(0.0)
        } else {
            0.0
        };
        self.deadline_at = Instant::now().checked_add(Duration::from_secs_f64(seconds));
    }

    pub(super) fn check_interruption(&self) -> Result<(), RunError> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(RunError::Script(Diagnostic::error(
                "PINE8008",
                DiagnosticCategory::Cancellation,
                "Evaluation cancelled.",
                SourceRange::ZERO,
            )));
        }
        if self
            .deadline_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(RunError::Script(Diagnostic::error(
                "PINE8007",
                DiagnosticCategory::Resource,
                "Evaluation deadline exceeded.",
                SourceRange::ZERO,
            )));
        }
        Ok(())
    }
}

/// Adds the references among `values` to `out`.
fn push_references<'a>(
    values: impl Iterator<Item = &'a RuntimeValue>,
    out: &mut Vec<(RefKind, usize)>,
) {
    for value in values {
        if let RuntimeValue::Ref(kind, id) = value {
            out.push((*kind, *id));
        }
    }
}
