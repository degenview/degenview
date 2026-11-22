//! A request's result kept apart from the chart's state, and copied out every time it is used.
//!
//! A collection or object an expression returns is a handle into the state it was made in. The
//! chart gets a copy of its own each time it is handed one, never the same handle twice: a result
//! the site remembers for the next bar, or for a second call in the same bar, would otherwise be
//! one array the script then changes (`array.push`) and reads back changed on a later bar.

use std::collections::HashMap;
use std::sync::Arc;

use super::state::RuntimeState;
use super::value::{RefKind, RuntimeValue};

/// What a request returned: a value, with the collections and objects it holds kept in a heap of
/// the value's own (none for a number, a string or `na`).
#[derive(Clone, Debug)]
pub struct SiteValue {
    value: RuntimeValue,
    heap: Option<Arc<RuntimeState>>,
}

impl SiteValue {
    /// `na`.
    pub fn na() -> Self {
        Self {
            value: RuntimeValue::Na,
            heap: None,
        }
    }

    /// Keeps `value`, which was made in `source`: what it holds is copied into a heap of its own.
    pub fn capture(value: RuntimeValue, source: &RuntimeState) -> Self {
        if !holds_reference(&value) {
            return Self { value, heap: None };
        }
        let mut heap = RuntimeState::default();
        let value = ReferenceExport::new(source).export(value, &mut heap);
        Self {
            value,
            heap: Some(Arc::new(heap)),
        }
    }

    /// The value for the chart: what it holds is copied into `chart`, new handles every time.
    pub fn hand_out(&self, chart: &mut RuntimeState) -> RuntimeValue {
        match &self.heap {
            None => self.value.clone(),
            Some(heap) => ReferenceExport::new(heap).export(self.value.clone(), chart),
        }
    }
}

/// Whether `value` is, or in a tuple holds, a handle.
fn holds_reference(value: &RuntimeValue) -> bool {
    match value {
        RuntimeValue::Ref(..) => true,
        RuntimeValue::Tuple(items) => items.iter().any(holds_reference),
        _ => false,
    }
}

/// Copies the collections and objects a security expression returned out of the site's own state
/// and into the chart's, so a handle that crossed over does not name an unrelated value. Arrays and
/// instances are copied following the handles they hold (a handle reached twice is copied once);
/// handles inside a map or matrix are not followed and become `na`, and so do drawing handles.
pub(super) struct ReferenceExport<'a> {
    source: &'a RuntimeState,
    copies: HashMap<usize, RuntimeValue>,
}

impl<'a> ReferenceExport<'a> {
    pub(super) fn new(source: &'a RuntimeState) -> Self {
        Self {
            source,
            copies: HashMap::new(),
        }
    }

    pub(super) fn export(
        &mut self,
        value: RuntimeValue,
        target: &mut RuntimeState,
    ) -> RuntimeValue {
        match value {
            RuntimeValue::Tuple(items) => RuntimeValue::Tuple(
                items
                    .into_iter()
                    .map(|item| self.export(item, target))
                    .collect(),
            ),
            RuntimeValue::Ref(kind, id) => self.copy(kind, id, target),
            other => other,
        }
    }

    fn copy(&mut self, kind: RefKind, id: usize, target: &mut RuntimeState) -> RuntimeValue {
        if let Some(known) = self.copies.get(&id) {
            return known.clone();
        }
        match kind {
            RefKind::Array => {
                let Some(items) = self.source.arrays.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                let copied = items
                    .into_iter()
                    .map(|item| self.export(item, target))
                    .collect();
                target.arrays.insert(handle, copied);
                self.copies[&id].clone()
            }
            RefKind::Object => {
                let Some(object) = self.source.instances.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                let mut copied = object.clone();
                copied.fields = object
                    .fields
                    .into_iter()
                    .map(|(name, field)| (name, self.export(field, target)))
                    .collect();
                target.instances.insert(handle, copied);
                self.copies[&id].clone()
            }
            RefKind::Map => {
                let Some(map) = self.source.maps.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                target.maps.insert(handle, map);
                self.copies[&id].clone()
            }
            RefKind::Matrix => {
                let Some(matrix) = self.source.matrices.get(&id).cloned() else {
                    return RuntimeValue::Na;
                };
                let handle = self.register(kind, id, target);
                target.matrices.insert(handle, matrix);
                self.copies[&id].clone()
            }
            // Drawing handles and any other reference belong to the expression's state.
            _ => RuntimeValue::Na,
        }
    }

    fn register(&mut self, kind: RefKind, id: usize, target: &mut RuntimeState) -> usize {
        target.next_reference += 1;
        let handle = target.next_reference;
        self.copies.insert(id, RuntimeValue::Ref(kind, handle));
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_array(items: Vec<RuntimeValue>) -> (RuntimeState, RuntimeValue) {
        let mut state = RuntimeState::default();
        state.next_reference += 1;
        let id = state.next_reference;
        state.arrays.insert(id, items);
        (state, RuntimeValue::Ref(RefKind::Array, id))
    }

    fn items(state: &RuntimeState, value: &RuntimeValue) -> Vec<RuntimeValue> {
        let RuntimeValue::Ref(RefKind::Array, id) = value else {
            panic!("{value:?}");
        };
        state.arrays[id].clone()
    }

    #[test]
    fn a_scalar_is_kept_without_a_heap() {
        let source = RuntimeState::default();
        for value in [
            RuntimeValue::Na,
            RuntimeValue::Int(3),
            RuntimeValue::Bool(true),
        ] {
            let kept = SiteValue::capture(value.clone(), &source);
            assert!(kept.heap.is_none());
            assert_eq!(kept.hand_out(&mut RuntimeState::default()), value);
        }
        assert!(!holds_reference(&RuntimeValue::Tuple(vec![
            RuntimeValue::Int(1)
        ])));
    }

    #[test]
    fn every_hand_out_is_a_copy_of_its_own() {
        let (source, array) = state_with_array(vec![RuntimeValue::Int(7)]);
        let kept = SiteValue::capture(array, &source);
        let mut chart = RuntimeState::default();
        let first = kept.hand_out(&mut chart);
        let second = kept.hand_out(&mut chart);
        assert_ne!(first, second, "a new handle each time");
        let RuntimeValue::Ref(RefKind::Array, id) = &first else {
            panic!("{first:?}");
        };
        chart.arrays.get_mut(id).unwrap().push(RuntimeValue::Int(8));
        assert_eq!(items(&chart, &first).len(), 2);
        assert_eq!(items(&chart, &second), vec![RuntimeValue::Int(7)]);
        let third = kept.hand_out(&mut chart);
        assert_eq!(items(&chart, &third), vec![RuntimeValue::Int(7)]);
    }

    #[test]
    fn what_the_source_does_later_does_not_reach_a_captured_value() {
        let (mut source, array) = state_with_array(vec![RuntimeValue::Int(1)]);
        let kept = SiteValue::capture(array.clone(), &source);
        let RuntimeValue::Ref(_, id) = &array else {
            unreachable!()
        };
        source
            .arrays
            .get_mut(id)
            .unwrap()
            .push(RuntimeValue::Int(2));
        let mut chart = RuntimeState::default();
        let value = kept.hand_out(&mut chart);
        assert_eq!(items(&chart, &value), vec![RuntimeValue::Int(1)]);
    }

    #[test]
    fn a_tuple_of_arrays_is_captured_whole() {
        let (source, array) = state_with_array(vec![RuntimeValue::Int(5)]);
        let tuple = RuntimeValue::Tuple(vec![array.clone(), RuntimeValue::Int(9), array]);
        assert!(holds_reference(&tuple));
        let kept = SiteValue::capture(tuple, &source);
        let mut chart = RuntimeState::default();
        let RuntimeValue::Tuple(parts) = kept.hand_out(&mut chart) else {
            panic!("a tuple");
        };
        assert_eq!(parts[1], RuntimeValue::Int(9));
        assert_eq!(parts[0], parts[2], "an array reached twice is copied once");
        assert_eq!(items(&chart, &parts[0]), vec![RuntimeValue::Int(5)]);
    }

    #[test]
    fn reference_export_copies_arrays_across_the_boundary() {
        let mut source = RuntimeState::default();
        source.next_reference += 1;
        source.arrays.insert(1, vec![RuntimeValue::Int(5)]);
        let mut exporter = ReferenceExport::new(&source);

        let mut target = RuntimeState::default();
        let value = exporter.export(RuntimeValue::Ref(RefKind::Array, 1), &mut target);
        let RuntimeValue::Ref(RefKind::Array, handle) = value else {
            panic!("{value:?}");
        };
        assert_eq!(target.arrays[&handle], vec![RuntimeValue::Int(5)]);
    }

    #[test]
    fn reference_export_follows_nested_handles_and_degrades_drawings() {
        let mut source = RuntimeState::default();
        source.next_reference += 1;
        let array_id = source.next_reference;
        source.arrays.insert(array_id, vec![RuntimeValue::Int(7)]);
        source.next_reference += 1;
        let object_id = source.next_reference;
        source.instances.insert(
            object_id,
            crate::pine::runtime::containers::Object {
                type_name: "T".to_owned(),
                fields: [
                    (
                        "arr".to_owned(),
                        RuntimeValue::Ref(RefKind::Array, array_id),
                    ),
                    ("line".to_owned(), RuntimeValue::Ref(RefKind::Line, 99)),
                ]
                .into_iter()
                .collect(),
            },
        );
        // A self-referential array (a cycle), to check handles reached twice copy once.
        source.next_reference += 1;
        let cycle_id = source.next_reference;
        source
            .arrays
            .insert(cycle_id, vec![RuntimeValue::Ref(RefKind::Array, cycle_id)]);

        let mut exporter = ReferenceExport::new(&source);
        let mut target = RuntimeState::default();

        let value = exporter.export(RuntimeValue::Ref(RefKind::Object, object_id), &mut target);
        let RuntimeValue::Ref(RefKind::Object, handle) = value else {
            panic!("{value:?}");
        };
        let object = &target.instances[&handle];
        let RuntimeValue::Ref(RefKind::Array, arr_handle) = &object.fields["arr"] else {
            panic!();
        };
        assert_eq!(target.arrays[arr_handle], vec![RuntimeValue::Int(7)]);
        assert_eq!(
            object.fields["line"],
            RuntimeValue::Na,
            "a drawing handle is na"
        );

        let cycle = exporter.export(RuntimeValue::Ref(RefKind::Array, cycle_id), &mut target);
        let RuntimeValue::Ref(RefKind::Array, cycle_handle) = cycle else {
            panic!("{cycle:?}");
        };
        assert_eq!(
            target.arrays[&cycle_handle],
            vec![RuntimeValue::Ref(RefKind::Array, cycle_handle)],
            "a cycle survives the copy"
        );
    }
}
