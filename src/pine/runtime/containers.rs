//! Runtime-owned containers: `Object` (a script-defined `type`), `Map` and `Matrix`, ported from
//! `PineObject.swift`, `PineMap.swift` and `PineMatrix.swift`.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::fast_hash::FastMap;
use super::value::{RefKind, RuntimeValue};
use crate::pine::syntax::token::ValueType;

/// One instance of a script-defined `type`. Fields are dynamic: the declaration only supplies
/// the names and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub type_name: String,
    pub fields: HashMap<String, RuntimeValue>,
}

/// A `map<K, V>` lookup key. Untyped maps share a key for an integer and the same whole float.
/// Typed float maps instead widen integer inputs before constructing the key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MapKey {
    Int(i64),
    /// The float's bit pattern, with the two zeros represented by the same key.
    Float(u64),
    Bool(bool),
    String(String),
    Color(u32),
}

impl MapKey {
    /// None for a value that cannot be a key (`na`, handles, tuples).
    pub fn from_value(value: &RuntimeValue) -> Option<Self> {
        match value {
            RuntimeValue::Int(x) => Some(Self::Int(*x)),
            RuntimeValue::Float(x) => {
                if *x == x.round() && x.abs() < 9_007_199_254_740_992.0 {
                    Some(Self::Int(*x as i64))
                } else {
                    Some(Self::Float(x.to_bits()))
                }
            }
            RuntimeValue::Bool(x) => Some(Self::Bool(*x)),
            RuntimeValue::String(x) => Some(Self::String(x.clone())),
            RuntimeValue::Color(x) => Some(Self::Color(*x)),
            _ => None,
        }
    }

    pub fn value(&self) -> RuntimeValue {
        match self {
            Self::Int(x) => RuntimeValue::Int(*x),
            Self::Float(bits) => RuntimeValue::Float(f64::from_bits(*bits)),
            Self::Bool(x) => RuntimeValue::Bool(*x),
            Self::String(x) => RuntimeValue::String(x.clone()),
            Self::Color(x) => RuntimeValue::Color(*x),
        }
    }
}

/// A `map<K, V>`: insertion-ordered, like Pine's. A map built with `map.new<K, V>()` holds only
/// what it was declared to: a write of anything else is refused, so the types a reader of the map
/// relies on (a `for [key, value]` loop's bindings) always hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map {
    order: Vec<MapKey>,
    storage: FastMap<MapKey, RuntimeValue>,
    types: Option<[ValueType; 2]>,
    key_revision: u64,
}

/// What a typed map refused of a write: the key or the value, with the type declared for it. An
/// untyped map only refuses a key that cannot be one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rejected {
    Key(Option<ValueType>),
    Value(ValueType),
}

impl Map {
    pub fn typed(types: [ValueType; 2]) -> Self {
        Self {
            types: Some(types),
            ..Self::default()
        }
    }

    pub fn count(&self) -> usize {
        self.order.len()
    }

    pub fn key_revision(&self) -> u64 {
        self.key_revision
    }

    /// The key a read (`get`, `contains`, `remove`) looks `value` up by. A typed float map widens an
    /// integer to the float it means; a read never refuses a key of another type, it finds nothing.
    pub fn lookup_key(&self, value: &RuntimeValue) -> Option<MapKey> {
        if self.types.is_some_and(|[key, _]| key == ValueType::Float) {
            return float_key(value).or_else(|| MapKey::from_value(value));
        }
        MapKey::from_value(value)
    }

    /// The key and value as this map stores them, or what its declared types refuse. A `float` slot
    /// takes an `int` as the float it means, a `na` value fits every type, and a slot declared as a
    /// script type or an enum (`object`) takes any handle or enum member: which type it is, is for
    /// the checker.
    pub fn admit(
        &self,
        key: &RuntimeValue,
        value: RuntimeValue,
    ) -> Result<(MapKey, RuntimeValue), Rejected> {
        let Some([key_type, value_type]) = self.types else {
            return MapKey::from_value(key)
                .map(|key| (key, value))
                .ok_or(Rejected::Key(None));
        };
        let key = match key_type {
            ValueType::Float => float_key(key),
            _ if slot_takes(key_type, key) => MapKey::from_value(key),
            _ => None,
        }
        .ok_or(Rejected::Key(Some(key_type)))?;
        let value = match value {
            RuntimeValue::Na => value,
            RuntimeValue::Int(n) if value_type == ValueType::Float => RuntimeValue::Float(n as f64),
            value if slot_takes(value_type, &value) => value,
            _ => return Err(Rejected::Value(value_type)),
        };
        Ok((key, value))
    }

    /// Stores `value` under `key` if the map's types allow both, returning what the key held before.
    pub fn insert(
        &mut self,
        key: &RuntimeValue,
        value: RuntimeValue,
    ) -> Result<Option<RuntimeValue>, Rejected> {
        let (key, value) = self.admit(key, value)?;
        Ok(self.store(value, key))
    }

    /// Stores every pair of `other`. Each is checked before one is stored, so a refused pair leaves
    /// the map as it was.
    pub fn put_all(&mut self, other: &Map) -> Result<(), Rejected> {
        let admitted = other
            .pairs()
            .into_iter()
            .map(|(key, value)| self.admit(&key, value))
            .collect::<Result<Vec<_>, _>>()?;
        for (key, value) in admitted {
            self.store(value, key);
        }
        Ok(())
    }

    pub fn pair_at(&self, index: usize) -> Option<(RuntimeValue, RuntimeValue)> {
        let key = self.order.get(index)?;
        Some((key.value(), self.storage[key].clone()))
    }

    pub fn pairs(&self) -> Vec<(RuntimeValue, RuntimeValue)> {
        self.order
            .iter()
            .map(|key| (key.value(), self.storage[key].clone()))
            .collect()
    }

    pub fn keys(&self) -> Vec<RuntimeValue> {
        self.order.iter().map(MapKey::value).collect()
    }

    pub fn values(&self) -> Vec<RuntimeValue> {
        self.order
            .iter()
            .map(|key| self.storage.get(key).cloned().unwrap_or(RuntimeValue::Na))
            .collect()
    }

    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = &mut RuntimeValue> {
        self.storage.values_mut()
    }

    pub fn contains(&self, key: &MapKey) -> bool {
        self.storage.contains_key(key)
    }

    pub fn value(&self, key: &MapKey) -> Option<&RuntimeValue> {
        self.storage.get(key)
    }

    fn store(&mut self, value: RuntimeValue, key: MapKey) -> Option<RuntimeValue> {
        match self.storage.entry(key) {
            Entry::Occupied(mut slot) => Some(slot.insert(value)),
            Entry::Vacant(slot) => {
                self.order.push(slot.key().clone());
                self.key_revision += 1;
                slot.insert(value);
                None
            }
        }
    }

    pub fn remove(&mut self, key: &MapKey) -> Option<RuntimeValue> {
        let removed = self.storage.remove(key)?;
        self.order.retain(|candidate| candidate != key);
        self.key_revision += 1;
        Some(removed)
    }

    pub fn clear(&mut self) {
        if !self.order.is_empty() {
            self.key_revision += 1;
            self.order.clear();
            self.storage.clear();
        }
    }
}

/// The key of a number in a `float` map: an integer is the float it means, and the two zeros are
/// one key.
fn float_key(value: &RuntimeValue) -> Option<MapKey> {
    let number = value.number()?;
    Some(MapKey::Float(if number == 0.0 {
        0
    } else {
        number.to_bits()
    }))
}

/// Whether a slot declared as `kind` takes `value`, widening aside.
fn slot_takes(kind: ValueType, value: &RuntimeValue) -> bool {
    match kind {
        ValueType::Object => matches!(
            value,
            RuntimeValue::String(_)
                | RuntimeValue::Ref(RefKind::Object | RefKind::Footprint | RefKind::VolumeRow, _)
        ),
        kind => value.is_kind(kind),
    }
}

/// A `matrix<T>`: row-major, rows and columns can grow and shrink. Element types are not tracked.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    rows: usize,
    columns: usize,
    elements: Vec<RuntimeValue>,
}

impl Matrix {
    pub fn new(rows: usize, columns: usize, initial: RuntimeValue) -> Self {
        Self {
            rows,
            columns,
            elements: vec![initial; rows * columns],
        }
    }

    /// A matrix of the given shape from its elements row by row; `elements.len()` must be
    /// `rows * columns`.
    pub fn from_elements(rows: usize, columns: usize, elements: Vec<RuntimeValue>) -> Self {
        assert_eq!(elements.len(), rows * columns, "a matrix is rows × columns");
        Self {
            rows,
            columns,
            elements,
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn is_square(&self) -> bool {
        self.rows == self.columns
    }

    pub fn count(&self) -> usize {
        self.elements.len()
    }

    pub fn all_elements(&self) -> &[RuntimeValue] {
        &self.elements
    }

    pub(super) fn all_elements_mut(&mut self) -> &mut [RuntimeValue] {
        &mut self.elements
    }

    pub fn contains(&self, row: usize, column: usize) -> bool {
        row < self.rows && column < self.columns
    }

    pub fn get(&self, row: usize, column: usize) -> &RuntimeValue {
        &self.elements[row * self.columns + column]
    }

    pub fn set(&mut self, row: usize, column: usize, value: RuntimeValue) {
        self.elements[row * self.columns + column] = value;
    }

    pub fn row(&self, index: usize) -> Vec<RuntimeValue> {
        (0..self.columns)
            .map(|c| self.get(index, c).clone())
            .collect()
    }

    pub fn column(&self, index: usize) -> Vec<RuntimeValue> {
        (0..self.rows).map(|r| self.get(r, index).clone()).collect()
    }

    /// Inserts a row before `index` (`rows` appends). Missing cells are `na`.
    /// Whether the matrix is 0×0, the only shape the first added row (or column) may redefine.
    pub fn is_empty(&self) -> bool {
        self.rows == 0 && self.columns == 0
    }

    /// Sets the shape of a 0×0 matrix, so the first row (or column) added decides the other dimension.
    pub fn reshape_empty(&mut self, rows: usize, columns: usize) {
        if self.is_empty() {
            self.rows = rows;
            self.columns = columns;
            self.elements = vec![RuntimeValue::Na; rows * columns];
        }
    }

    pub fn insert_row(&mut self, index: usize, values: &[RuntimeValue]) {
        let row: Vec<RuntimeValue> = (0..self.columns)
            .map(|c| values.get(c).cloned().unwrap_or(RuntimeValue::Na))
            .collect();
        self.elements
            .splice(index * self.columns..index * self.columns, row);
        self.rows += 1;
    }

    pub fn insert_column(&mut self, index: usize, values: &[RuntimeValue]) {
        let mut next: Vec<RuntimeValue> = Vec::with_capacity((self.columns + 1) * self.rows);
        for r in 0..self.rows {
            for c in 0..=self.columns {
                if c == index {
                    next.push(values.get(r).cloned().unwrap_or(RuntimeValue::Na));
                } else {
                    let source = if c < index { c } else { c - 1 };
                    next.push(self.get(r, source).clone());
                }
            }
        }
        self.elements = next;
        self.columns += 1;
    }

    pub fn remove_row(&mut self, index: usize) {
        self.elements
            .drain(index * self.columns..(index + 1) * self.columns);
        self.rows -= 1;
    }

    pub fn remove_column(&mut self, index: usize) {
        let mut next = Vec::with_capacity(self.rows * (self.columns - 1));
        for r in 0..self.rows {
            for c in 0..self.columns {
                if c != index {
                    next.push(self.get(r, c).clone());
                }
            }
        }
        self.elements = next;
        self.columns -= 1;
    }

    /// Sets every cell in `rows` × `columns` (half-open, both within the matrix) to `value`.
    pub fn fill_range(
        &mut self,
        rows: std::ops::Range<usize>,
        columns: std::ops::Range<usize>,
        value: &RuntimeValue,
    ) {
        for row in rows {
            self.elements[row * self.columns + columns.start..row * self.columns + columns.end]
                .fill(value.clone());
        }
    }

    /// Reverses the order of rows and of columns: the first cell becomes the last. Row by row
    /// storage makes that a reversal of the whole element list.
    pub fn reverse(&mut self) {
        self.elements.reverse();
    }

    pub fn swap_rows(&mut self, first: usize, second: usize) {
        if first != second {
            for column in 0..self.columns {
                self.elements.swap(
                    first * self.columns + column,
                    second * self.columns + column,
                );
            }
        }
    }

    pub fn swap_columns(&mut self, first: usize, second: usize) {
        if first != second {
            for row in 0..self.rows {
                self.elements
                    .swap(row * self.columns + first, row * self.columns + second);
            }
        }
    }

    /// The same elements read row by row into `rows` × `columns`, which must hold as many.
    pub fn reshape(&mut self, rows: usize, columns: usize) {
        assert_eq!(
            rows * columns,
            self.elements.len(),
            "a reshape keeps the cells"
        );
        self.rows = rows;
        self.columns = columns;
    }

    /// The rows `rows` and columns `columns` (half-open, within the matrix) as a new matrix.
    pub fn submatrix(&self, rows: std::ops::Range<usize>, columns: std::ops::Range<usize>) -> Self {
        let mut elements = Vec::with_capacity(rows.len() * columns.len());
        for row in rows.clone() {
            elements.extend_from_slice(
                &self.elements
                    [row * self.columns + columns.start..row * self.columns + columns.end],
            );
        }
        Self::from_elements(rows.len(), columns.len(), elements)
    }

    /// The matrix with its rows in the given order (`order[i]` is the old row that becomes row `i`).
    pub fn with_rows_ordered(&self, order: &[usize]) -> Self {
        let mut elements = Vec::with_capacity(self.elements.len());
        for row in order {
            elements
                .extend_from_slice(&self.elements[row * self.columns..(row + 1) * self.columns]);
        }
        Self::from_elements(order.len(), self.columns, elements)
    }

    /// Appends the rows of `other`, which has as many columns.
    pub fn append_rows(&mut self, other: &Matrix) {
        debug_assert_eq!(self.columns, other.columns);
        self.elements.extend_from_slice(&other.elements);
        self.rows += other.rows;
    }

    pub fn transposed(&self) -> Self {
        let mut result = Matrix::new(self.columns, self.rows, RuntimeValue::Na);
        for r in 0..self.rows {
            for c in 0..self.columns {
                result.set(c, r, self.get(r, c).clone());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_keys_fold_whole_floats_into_ints() {
        let two = MapKey::from_value(&RuntimeValue::Int(2)).unwrap();
        let two_float = MapKey::from_value(&RuntimeValue::Float(2.0)).unwrap();
        assert_eq!(two, two_float);
        assert!(matches!(
            MapKey::from_value(&RuntimeValue::Float(2.5)).unwrap(),
            MapKey::Float(_)
        ));
        assert!(MapKey::from_value(&RuntimeValue::Na).is_none());
    }

    #[test]
    fn map_is_insertion_ordered() {
        let mut map = Map::default();
        let int = RuntimeValue::Int;
        map.insert(&int(10), int(1)).unwrap();
        map.insert(&int(20), int(2)).unwrap();
        map.insert(&int(10), int(3)).unwrap(); // overwrite, order unchanged
        assert_eq!(map.count(), 2);
        assert_eq!(map.keys(), vec![int(10), int(20)]);
        assert_eq!(map.values(), vec![int(3), int(2)]);
    }

    #[test]
    fn a_typed_map_widens_ints_and_refuses_what_it_was_not_declared_to_hold() {
        use ValueType::{Float, Int, String};
        let text = |text: &str| RuntimeValue::String(text.to_owned());
        let mut map = Map::typed([Int, Float]);
        // A value widens, a key of the wrong type or a value of the wrong type is refused.
        map.insert(&RuntimeValue::Int(1), RuntimeValue::Int(2))
            .unwrap();
        assert_eq!(map.values(), vec![RuntimeValue::Float(2.0)]);
        assert_eq!(
            map.insert(&text("a"), RuntimeValue::Float(1.0)),
            Err(Rejected::Key(Some(Int)))
        );
        assert_eq!(
            map.insert(&RuntimeValue::Float(1.5), RuntimeValue::Float(1.0)),
            Err(Rejected::Key(Some(Int)))
        );
        assert_eq!(
            map.insert(&RuntimeValue::Int(2), text("b")),
            Err(Rejected::Value(Float))
        );
        // `na` is a value of every type, and a refused write leaves the map as it was.
        map.insert(&RuntimeValue::Int(2), RuntimeValue::Na).unwrap();
        assert_eq!(map.count(), 2);
        let revision = map.key_revision();
        assert!(map.insert(&text("a"), RuntimeValue::Na).is_err());
        assert_eq!(map.key_revision(), revision);
        // A float map takes an int key as the float it means.
        let mut floats = Map::typed([Float, String]);
        floats.insert(&RuntimeValue::Int(3), text("x")).unwrap();
        floats.insert(&RuntimeValue::Float(3.0), text("y")).unwrap();
        assert_eq!(floats.keys(), vec![RuntimeValue::Float(3.0)]);
        // An untyped map only refuses a key that cannot be one.
        assert_eq!(
            Map::default().insert(&RuntimeValue::Na, RuntimeValue::Int(1)),
            Err(Rejected::Key(None))
        );
    }

    #[test]
    fn a_refused_put_all_stores_nothing() {
        use ValueType::{Float, Int};
        let mut source = Map::default();
        source
            .insert(&RuntimeValue::Int(1), RuntimeValue::Float(1.5))
            .unwrap();
        source
            .insert(&RuntimeValue::Int(2), RuntimeValue::String("x".to_owned()))
            .unwrap();
        let mut target = Map::typed([Int, Float]);
        assert_eq!(target.put_all(&source), Err(Rejected::Value(Float)));
        assert_eq!(target.count(), 0);
        assert_eq!(target.key_revision(), 0);
    }

    #[test]
    fn an_object_slot_takes_enum_members_and_handles_but_not_numbers() {
        use ValueType::Object;
        let mut map = Map::typed([Object, Object]);
        let handle = RuntimeValue::Ref(RefKind::Object, 1);
        map.insert(&RuntimeValue::String("Side.buy".to_owned()), handle)
            .unwrap();
        assert_eq!(
            map.insert(&RuntimeValue::Int(1), RuntimeValue::Na),
            Err(Rejected::Key(Some(Object)))
        );
        assert_eq!(
            map.insert(
                &RuntimeValue::String("Side.sell".to_owned()),
                RuntimeValue::Int(1)
            ),
            Err(Rejected::Value(Object))
        );
    }

    #[test]
    fn matrix_grows_and_transposes() {
        let mut matrix = Matrix::new(1, 2, RuntimeValue::Int(0));
        matrix.set(0, 0, RuntimeValue::Int(1));
        matrix.set(0, 1, RuntimeValue::Int(2));
        matrix.insert_row(1, &[RuntimeValue::Int(3), RuntimeValue::Int(4)]);
        assert_eq!(matrix.rows(), 2);
        let transposed = matrix.transposed();
        assert_eq!(transposed.rows(), 2);
        assert_eq!(transposed.columns(), 2);
        assert_eq!(transposed.get(0, 1), &RuntimeValue::Int(3));
    }
}
