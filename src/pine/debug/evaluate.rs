//! Running a checked expression against a stopped script.
//!
//! [`parse`](super::expression::parse) only admits expressions made of reads and pure
//! computations. This module still treats the run as something to undo: it puts the frame's own
//! values on the variable table for the length of the call and takes them off again, hands the
//! interpreter its own copy of the bar context, and gives back the instruction budget and any
//! handle the evaluation allocated, so the script goes on exactly as if nothing had been asked.

use crate::pine::runtime_types::RuntimeContext;
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::Session;

use super::expression::DebugExpression;
use super::frames::Frame;

/// The most instructions one evaluation may use.
const BUDGET: usize = 100_000;

pub(crate) fn evaluate(
    session: &mut Session,
    frames: &[Frame],
    frame: usize,
    context: &RuntimeContext,
    expression: &DebugExpression,
) -> Result<RuntimeValue, String> {
    // While a `request.security()` expression runs, the chart's variables are set aside inside the
    // interpreter. They can be listed, but an expression cannot be run against them.
    if frames.iter().skip(frame).any(Frame::is_security) {
        return Err(
            "This frame is outside the request.security() expression that is running, so nothing can be evaluated in it."
                .to_owned(),
        );
    }
    // A call in progress removed the names it declares from the table and parked the caller's
    // values with its frame. Looking at an outer frame means putting those values back, briefly.
    let mut restore: Vec<(String, Option<RuntimeValue>)> = Vec::new();
    for callee in frames.iter().skip(frame + 1) {
        for (name, value) in &callee.shadowed {
            if restore.iter().any(|(done, _)| done == name) {
                continue;
            }
            restore.push((name.clone(), session.working.variables.get(name).cloned()));
            match value {
                Some(value) => {
                    session
                        .working
                        .variables
                        .insert(name.clone(), value.clone());
                }
                None => {
                    session.working.variables.remove(name);
                }
            }
        }
    }
    let instructions = session.working.instructions;
    let next_reference = session.working.next_reference;
    session.working.instructions = session.limits.instructions_per_bar.saturating_sub(BUDGET);

    let mut scratch = context.clone();
    scratch.site_prefix = 0;
    scratch.depth = 0;
    scratch.scope = None;
    scratch.locals = None;
    scratch.history_locals = None;
    let outcome = session.eval(expression.expression(), &mut scratch);

    session.working.instructions = instructions;
    let allocated = session.working.next_reference != next_reference;
    if allocated {
        release_allocations_since(session, next_reference);
    }
    for (name, previous) in restore.into_iter().rev() {
        match previous {
            Some(value) => {
                session.working.variables.insert(name, value);
            }
            None => {
                session.working.variables.remove(&name);
            }
        }
    }
    match outcome {
        Ok(RuntimeValue::Ref(kind, id)) if allocated && id > next_reference => Err(format!(
            "The result is a new {} that exists only for this evaluation.",
            format!("{kind:?}").to_lowercase()
        )),
        Ok(value) => Ok(value),
        Err(diagnostic) => Err(diagnostic.message),
    }
}

/// Forgets the containers and objects created since `before`, and the counter that numbered them.
fn release_allocations_since(session: &mut Session, before: usize) {
    let state = &mut session.working;
    state.arrays.retain(|id, _| *id <= before);
    state.maps.retain(|id, _| *id <= before);
    state.matrices.retain(|id, _| *id <= before);
    state.instances.retain(|id, _| *id <= before);
    state.next_reference = before;
}
