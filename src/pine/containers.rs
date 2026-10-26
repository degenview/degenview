//! Runtime-owned containers: `Object` (a script-defined `type`), `Map` and `Matrix`, ported from
//! `PineObject.swift`, `PineMap.swift` and `PineMatrix.swift`.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::fast_hash::FastMap;
use super::runtime_value::RuntimeValue;

/// One instance of a script-defined `type`. Fields are dynamic: the declaration only supplies
/// the names and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub type_name: String,
    pub fields: HashMap<String, RuntimeValue>,
}

/// A `map<K, V>` key. A whole float and the int with the same value are one key, because a script
/// that passes `2` where a float key is expected means `2.0`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MapKey {
    Int(i64),
    /// The float's bit pattern, so only whole floats fold into the int key.
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

/// A `map<K, V>`: insertion-ordered, like Pine's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map {
    order: Vec<MapKey>,
    storage: FastMap<MapKey, RuntimeValue>,
}

impl Map {
    pub fn count(&self) -> usize {
        self.order.len()
    }

    pub fn pairs(&self) -> Vec<(RuntimeValue, RuntimeValue)> {
        self.order
            .iter()
            .map(|key| {
                (
                    key.value(),
                    self.storage.get(key).cloned().unwrap_or(RuntimeValue::Na),
                )
            })
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

    pub fn contains(&self, key: &MapKey) -> bool {
        self.storage.contains_key(key)
    }

    pub fn value(&self, key: &MapKey) -> Option<&RuntimeValue> {
        self.storage.get(key)
    }

    /// Stores `value`, returning what the key held before.
    pub fn put(&mut self, value: RuntimeValue, key: MapKey) -> Option<RuntimeValue> {
        match self.storage.entry(key) {
            Entry::Occupied(mut slot) => Some(slot.insert(value)),
            Entry::Vacant(slot) => {
                self.order.push(slot.key().clone());
                slot.insert(value);
                None
            }
        }
    }

    pub fn remove(&mut self, key: &MapKey) -> Option<RuntimeValue> {
        let removed = self.storage.remove(key)?;
        self.order.retain(|candidate| candidate != key);
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.storage.clear();
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

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn count(&self) -> usize {
        self.elements.len()
    }

    pub fn all_elements(&self) -> &[RuntimeValue] {
        &self.elements
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

    pub fn fill(&mut self, value: RuntimeValue) {
        self.elements = vec![value; self.elements.len()];
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
        map.put(RuntimeValue::Int(1), MapKey::Int(10));
        map.put(RuntimeValue::Int(2), MapKey::Int(20));
        map.put(RuntimeValue::Int(3), MapKey::Int(10)); // overwrite, order unchanged
        assert_eq!(map.count(), 2);
        assert_eq!(
            map.keys(),
            vec![RuntimeValue::Int(10), RuntimeValue::Int(20)]
        );
        assert_eq!(
            map.values(),
            vec![RuntimeValue::Int(3), RuntimeValue::Int(2)]
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
