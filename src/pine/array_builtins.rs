//! `array.*`, ported from `PineRuntimeSession+Arrays.swift`. Pure over the run's `RuntimeState`
//! (arrays live in `state.arrays`, by handle), so the session only has to evaluate and bind the
//! call's arguments: `parameters(name)` lists the names to bind, in positional order, and
//! `call` does the rest.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::number_format;
use super::runtime_value::{RefKind, RuntimeValue};
use super::state::RuntimeState;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};

/// The most parameters a builtin binds by name (`ta.alma`).
const BOUND_CAPACITY: usize = 5;

/// Evaluated arguments by parameter name. A builtin has a handful of parameters with static names,
/// so this is a short list searched in order: binding a call allocates nothing, where a table of
/// owned names cost an allocation per argument on every `array.get` in a loop.
pub struct Bound {
    entries: [(&'static str, RuntimeValue); BOUND_CAPACITY],
    len: usize,
}

impl Default for Bound {
    fn default() -> Self {
        Self {
            entries: [const { ("", RuntimeValue::Na) }; BOUND_CAPACITY],
            len: 0,
        }
    }
}

impl Bound {
    pub fn get(&self, name: &str) -> Option<&RuntimeValue> {
        self.entries[..self.len]
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, value)| value)
    }

    /// Binds `name`, replacing an earlier binding of it.
    pub fn insert(&mut self, name: &'static str, value: RuntimeValue) {
        if let Some(entry) = self.entries[..self.len]
            .iter_mut()
            .find(|(candidate, _)| *candidate == name)
        {
            entry.1 = value;
            return;
        }
        assert!(
            self.len < BOUND_CAPACITY,
            "a builtin binds at most {BOUND_CAPACITY} parameters"
        );
        self.entries[self.len] = (name, value);
        self.len += 1;
    }
}

impl FromIterator<(&'static str, RuntimeValue)> for Bound {
    fn from_iter<I: IntoIterator<Item = (&'static str, RuntimeValue)>>(pairs: I) -> Self {
        let mut bound = Self::default();
        for (name, value) in pairs {
            bound.insert(name, value);
        }
        bound
    }
}

/// Parameter names per operation, in positional order. Operations not listed take `id`.
pub fn parameters(name: &str) -> &'static [&'static str] {
    if name.starts_with("array.new_") {
        return &["size", "initial_value"];
    }
    match name {
        "array.push"
        | "array.unshift"
        | "array.includes"
        | "array.indexof"
        | "array.lastindexof"
        | "array.binary_search"
        | "array.binary_search_leftmost"
        | "array.binary_search_rightmost" => &["id", "value"],
        "array.fill" => &["id", "value", "index_from", "index_to"],
        "array.get" | "array.remove" => &["id", "index"],
        "array.set" | "array.insert" => &["id", "index", "value"],
        "array.sort" | "array.sort_indices" => &["id", "order"],
        "array.join" => &["id", "separator"],
        "array.concat" => &["id", "other"],
        "array.slice" => &["id", "index_from", "index_to"],
        "array.variance" | "array.stdev" => &["id", "biased"],
        "array.percentile_nearest_rank" | "array.percentile_linear_interpolation" => {
            &["id", "percentage"]
        }
        "array.covariance" => &["id", "id2", "biased"],
        _ => &["id"],
    }
}

/// Whether `name` is an `array.*` function this module handles.
pub fn handles(name: &str) -> bool {
    // Every known name has the prefix, so a `map.*` or `matrix.*` call skips the scan of `KNOWN`.
    name.starts_with("array.")
        && (name.starts_with("array.new_") || name == "array.from" || KNOWN.contains(&name))
}

const KNOWN: [&str; 47] = [
    "array.fill",
    "array.lastindexof",
    "array.abs",
    "array.standardize",
    "array.binary_search",
    "array.binary_search_leftmost",
    "array.binary_search_rightmost",
    "array.push",
    "array.unshift",
    "array.set",
    "array.insert",
    "array.remove",
    "array.shift",
    "array.pop",
    "array.clear",
    "array.reverse",
    "array.sort",
    "array.concat",
    "array.get",
    "array.size",
    "array.sort_indices",
    "array.first",
    "array.last",
    "array.includes",
    "array.indexof",
    "array.join",
    "array.copy",
    "array.slice",
    "array.sum",
    "array.avg",
    "array.max",
    "array.min",
    "array.median",
    "array.mode",
    "array.range",
    "array.variance",
    "array.stdev",
    "array.percentile_nearest_rank",
    "array.percentile_linear_interpolation",
    "array.covariance",
    // The two constructors are matched by prefix and name above; kept here so `handles` is total.
    "array.from",
    "array.new",
    "array.new_float",
    "array.new_int",
    "array.new_bool",
    "array.new_string",
    "array.new_color",
];

fn runtime_error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

/// A fresh array holding `items`; the handle value a script keeps.
pub fn new_array(state: &mut RuntimeState, items: Vec<RuntimeValue>) -> RuntimeValue {
    state.next_reference += 1;
    let id = state.next_reference;
    state.arrays.insert(id, items);
    RuntimeValue::Ref(RefKind::Array, id)
}

/// Runs one `array.*` call. `all_arguments` is every evaluated argument in order (for
/// `array.from`).
pub fn call(
    name: &str,
    bound: &Bound,
    all_arguments: &[RuntimeValue],
    state: &mut RuntimeState,
    mintick: f64,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    if name.starts_with("array.new_") {
        let size = bound
            .get("size")
            .and_then(RuntimeValue::int_value)
            .map_or(0, |size| size.max(0) as usize);
        let initial = bound
            .get("initial_value")
            .cloned()
            .unwrap_or(RuntimeValue::Na);
        return Ok(new_array(state, vec![initial; size]));
    }
    if name == "array.from" {
        return Ok(new_array(state, all_arguments.to_vec()));
    }
    let Some(RuntimeValue::Ref(RefKind::Array, id)) = bound.get("id") else {
        return Err(runtime_error(
            "PINE4011",
            format!("{name} requires an array; got na."),
            range,
        ));
    };
    let id = *id;
    // The array leaves the map while one operation runs on it, so an `array.get` in a loop does not
    // copy the array. The two operations that read a second array (possibly this one) work on a copy.
    let reads_another = matches!(name, "array.concat" | "array.covariance");
    let taken = if reads_another {
        state.arrays.get(&id).cloned()
    } else {
        state.arrays.remove(&id)
    };
    let Some(mut items) = taken else {
        return Err(runtime_error(
            "PINE4011",
            format!("{name} requires an array; got na."),
            range,
        ));
    };
    let context = Context {
        name,
        bound,
        mintick,
        range,
    };
    let outcome = (|| {
        if let Some(mutated) = context.mutate(state, id, &mut items)? {
            Ok(mutated)
        } else if let Some(queried) = context.query(state, &items)? {
            Ok(queried)
        } else {
            Err(runtime_error(
                "PINE4007",
                format!("Unknown or unsupported function '{name}'."),
                range,
            ))
        }
    })();
    state.arrays.insert(id, items);
    outcome
}

struct Context<'a> {
    name: &'a str,
    bound: &'a Bound,
    mintick: f64,
    range: SourceRange,
}

impl Context<'_> {
    fn value(&self) -> RuntimeValue {
        self.bound.get("value").cloned().unwrap_or(RuntimeValue::Na)
    }

    fn descending(&self) -> bool {
        self.bound.get("order").and_then(RuntimeValue::text_value) == Some("order.descending")
    }

    /// A validated element index; `allow_end` admits `count` itself (insert, slice start).
    fn index(
        &self,
        value: Option<&RuntimeValue>,
        count: usize,
        allow_end: bool,
    ) -> Result<usize, Diagnostic> {
        let limit = count + usize::from(allow_end);
        match value.and_then(RuntimeValue::int_value) {
            Some(index) if index >= 0 && (index as usize) < limit => Ok(index as usize),
            other => Err(runtime_error(
                "PINE4010",
                format!(
                    "{} index {} is out of bounds (size {count}).",
                    self.name,
                    other.map_or("na".to_owned(), |i| i.to_string())
                ),
                self.range,
            )),
        }
    }

    fn require_array(
        &self,
        state: &RuntimeState,
        value: Option<&RuntimeValue>,
    ) -> Result<Vec<RuntimeValue>, Diagnostic> {
        match value {
            Some(RuntimeValue::Ref(RefKind::Array, id)) => state.arrays.get(id).cloned(),
            _ => None,
        }
        .ok_or_else(|| {
            runtime_error(
                "PINE4011",
                format!("{} requires two arrays.", self.name),
                self.range,
            )
        })
    }

    /// Operations that change `items`. None when the call is not one of them.
    fn mutate(
        &self,
        state: &RuntimeState,
        id: usize,
        items: &mut Vec<RuntimeValue>,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let value = self.value();
        match self.name {
            "array.push" => items.push(value),
            "array.unshift" => items.insert(0, value),
            "array.set" => {
                let index = self.index(self.bound.get("index"), items.len(), false)?;
                items[index] = value;
            }
            "array.insert" => {
                let index = self.index(self.bound.get("index"), items.len(), true)?;
                items.insert(index, value);
            }
            "array.remove" => {
                let index = self.index(self.bound.get("index"), items.len(), false)?;
                return Ok(Some(items.remove(index)));
            }
            "array.shift" => {
                return Ok(Some(if items.is_empty() {
                    RuntimeValue::Na
                } else {
                    items.remove(0)
                }));
            }
            "array.pop" => return Ok(Some(items.pop().unwrap_or(RuntimeValue::Na))),
            "array.clear" => items.clear(),
            "array.fill" => {
                let from = match self.bound.get("index_from") {
                    Some(RuntimeValue::Na) | None => 0,
                    other => self.index(other, items.len(), true)?,
                };
                let to = match self.bound.get("index_to").and_then(RuntimeValue::int_value) {
                    Some(to) => to,
                    None => items.len() as i64,
                };
                if to < from as i64 || to as usize > items.len() {
                    return Err(runtime_error(
                        "PINE4010",
                        format!("array.fill end index {to} is out of bounds."),
                        self.range,
                    ));
                }
                items[from..to as usize].fill(value);
            }
            "array.reverse" => items.reverse(),
            "array.sort" => sort(items, self.descending(), self.mintick),
            "array.concat" => {
                items.extend(self.require_array(state, self.bound.get("other"))?);
                return Ok(Some(RuntimeValue::Ref(RefKind::Array, id)));
            }
            _ => return Ok(None),
        }
        Ok(Some(RuntimeValue::Void))
    }

    /// Operations that only read `items`. None when the call is not one of them.
    fn query(
        &self,
        state: &mut RuntimeState,
        items: &[RuntimeValue],
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let value = self.value();
        Ok(Some(match self.name {
            "array.get" => items[self.index(self.bound.get("index"), items.len(), false)?].clone(),
            "array.size" => RuntimeValue::Int(items.len() as i64),
            "array.sort_indices" => {
                let indices = sorted_indices(items, self.descending(), self.mintick);
                new_array(state, indices)
            }
            "array.first" => items.first().cloned().unwrap_or(RuntimeValue::Na),
            "array.last" => items.last().cloned().unwrap_or(RuntimeValue::Na),
            "array.includes" => RuntimeValue::Bool(items.contains(&value)),
            "array.indexof" => RuntimeValue::Int(
                items
                    .iter()
                    .position(|item| *item == value)
                    .map_or(-1, |i| i as i64),
            ),
            "array.lastindexof" => RuntimeValue::Int(
                items
                    .iter()
                    .rposition(|item| *item == value)
                    .map_or(-1, |i| i as i64),
            ),
            "array.binary_search"
            | "array.binary_search_leftmost"
            | "array.binary_search_rightmost" => binary_search(self.name, items, &value),
            "array.abs" => {
                let absolute = items
                    .iter()
                    .map(|item| match item {
                        RuntimeValue::Int(x) => RuntimeValue::Int(x.saturating_abs()),
                        RuntimeValue::Float(x) => RuntimeValue::Float(x.abs()),
                        other => other.clone(),
                    })
                    .collect();
                new_array(state, absolute)
            }
            "array.standardize" => {
                let numbers: Vec<f64> = items.iter().filter_map(RuntimeValue::number).collect();
                let count = numbers.len() as f64;
                let mean = numbers.iter().sum::<f64>() / count;
                let deviation =
                    (numbers.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / count).sqrt();
                let scaled = items
                    .iter()
                    .map(|item| match item.number() {
                        Some(x) if deviation > 0.0 => RuntimeValue::Float((x - mean) / deviation),
                        Some(_) => RuntimeValue::Float(0.0),
                        None => RuntimeValue::Na,
                    })
                    .collect();
                new_array(state, scaled)
            }
            "array.join" => {
                let separator = self
                    .bound
                    .get("separator")
                    .and_then(RuntimeValue::text_value)
                    .unwrap_or(",");
                RuntimeValue::String(
                    items
                        .iter()
                        .map(|item| number_format::format(item, None, self.mintick))
                        .collect::<Vec<_>>()
                        .join(separator),
                )
            }
            "array.copy" => new_array(state, items.to_vec()),
            "array.slice" => self.slice(state, items)?,
            "array.sum" | "array.avg" | "array.max" | "array.min" => aggregate(self.name, items),
            "array.median"
            | "array.mode"
            | "array.range"
            | "array.variance"
            | "array.stdev"
            | "array.percentile_nearest_rank"
            | "array.percentile_linear_interpolation" => statistic(self.name, self.bound, items),
            "array.covariance" => self.covariance(state, items)?,
            _ => return Ok(None),
        }))
    }

    fn slice(
        &self,
        state: &mut RuntimeState,
        items: &[RuntimeValue],
    ) -> Result<RuntimeValue, Diagnostic> {
        let from = self.index(self.bound.get("index_from"), items.len(), true)?;
        let to = self
            .bound
            .get("index_to")
            .and_then(RuntimeValue::int_value)
            .unwrap_or(items.len() as i64);
        if to < from as i64 || to as usize > items.len() {
            return Err(runtime_error(
                "PINE4010",
                format!("array.slice end index {to} is out of bounds."),
                self.range,
            ));
        }
        Ok(new_array(state, items[from..to as usize].to_vec()))
    }

    /// `array.covariance(id1, id2, biased)` over the first `min(size)` pairs.
    fn covariance(
        &self,
        state: &RuntimeState,
        items: &[RuntimeValue],
    ) -> Result<RuntimeValue, Diagnostic> {
        let other = self.require_array(state, self.bound.get("id2"))?;
        let pairs: Vec<(f64, f64)> = items
            .iter()
            .zip(&other)
            .filter_map(|(x, y)| Some((x.number()?, y.number()?)))
            .collect();
        let biased = self
            .bound
            .get("biased")
            .and_then(RuntimeValue::bool)
            .unwrap_or(true);
        if !(pairs.len() > 1 || (pairs.len() == 1 && biased)) {
            return Ok(RuntimeValue::Na);
        }
        let count = pairs.len() as f64;
        let mean_x = pairs.iter().map(|p| p.0).sum::<f64>() / count;
        let mean_y = pairs.iter().map(|p| p.1).sum::<f64>() / count;
        let total: f64 = pairs.iter().map(|p| (p.0 - mean_x) * (p.1 - mean_y)).sum();
        Ok(RuntimeValue::Float(
            total / if biased { count } else { count - 1.0 },
        ))
    }
}

/// Numbers first (ascending or descending), then everything else by its printed text (`na`
/// prints as "NaN"). Matches `PineRuntimeSession.precedes`.
fn compare(lhs: &RuntimeValue, rhs: &RuntimeValue, descending: bool, mintick: f64) -> Ordering {
    match (lhs.number(), rhs.number()) {
        (Some(x), Some(y)) => {
            let order = x.total_cmp(&y);
            if descending { order.reverse() } else { order }
        }
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => number_format::format(lhs, None, mintick)
            .cmp(&number_format::format(rhs, None, mintick)),
    }
}

fn sort(items: &mut [RuntimeValue], descending: bool, mintick: f64) {
    items.sort_by(|a, b| compare(a, b, descending, mintick));
}

/// `array.sort_indices`: the indexes that would sort `items`, ties keeping their original order.
fn sorted_indices(items: &[RuntimeValue], descending: bool, mintick: f64) -> Vec<RuntimeValue> {
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by(|a, b| compare(&items[*a], &items[*b], descending, mintick).then(a.cmp(b)));
    indices
        .into_iter()
        .map(|i| RuntimeValue::Int(i as i64))
        .collect()
}

/// `array.binary_search*` over a sorted array. Plain: the index of a match or -1. Leftmost: the first
/// match, else the index of the next smaller element (-1 when none is smaller). Rightmost: the last
/// match, else the index of the next larger element (the size when none is larger).
fn binary_search(name: &str, items: &[RuntimeValue], target: &RuntimeValue) -> RuntimeValue {
    let Some(target) = target.number() else {
        return RuntimeValue::Int(-1);
    };
    let values: Vec<f64> = items
        .iter()
        .map(|item| item.number().unwrap_or(f64::NAN))
        .collect();
    let first_not_below = values.partition_point(|value| *value < target);
    let first_above = values.partition_point(|value| *value <= target);
    let found = first_not_below < first_above;
    RuntimeValue::Int(match name {
        "array.binary_search" => {
            if found {
                first_not_below as i64
            } else {
                -1
            }
        }
        "array.binary_search_leftmost" => {
            if found {
                first_not_below as i64
            } else {
                first_not_below as i64 - 1
            }
        }
        _ => {
            if found {
                first_above as i64 - 1
            } else {
                first_above as i64
            }
        }
    })
}

fn aggregate(name: &str, items: &[RuntimeValue]) -> RuntimeValue {
    let numbers: Vec<f64> = items.iter().filter_map(RuntimeValue::number).collect();
    match name {
        "array.sum" => {
            let total: f64 = numbers.iter().sum();
            let integral = items
                .iter()
                .all(|item| item.is_int() || *item == RuntimeValue::Na);
            if integral {
                RuntimeValue::integral(total)
            } else {
                RuntimeValue::Float(total)
            }
        }
        "array.avg" => {
            if numbers.is_empty() {
                RuntimeValue::Na
            } else {
                RuntimeValue::Float(numbers.iter().sum::<f64>() / numbers.len() as f64)
            }
        }
        "array.max" => numbers
            .into_iter()
            .reduce(f64::max)
            .map_or(RuntimeValue::Na, RuntimeValue::Float),
        _ => numbers
            .into_iter()
            .reduce(f64::min)
            .map_or(RuntimeValue::Na, RuntimeValue::Float),
    }
}

/// The statistics over the numbers in the array (`na` entries are ignored); `na` when there are
/// none. `variance`, `stdev` and `covariance` are the population figures unless `biased = false`.
fn statistic(name: &str, bound: &Bound, items: &[RuntimeValue]) -> RuntimeValue {
    let mut numbers: Vec<f64> = items.iter().filter_map(RuntimeValue::number).collect();
    numbers.sort_by(f64::total_cmp);
    if numbers.is_empty() {
        return RuntimeValue::Na;
    }
    let count = numbers.len() as f64;
    let float = RuntimeValue::Float;
    match name {
        "array.median" => {
            let middle = numbers.len() / 2;
            float(if numbers.len() % 2 == 1 {
                numbers[middle]
            } else {
                (numbers[middle - 1] + numbers[middle]) / 2.0
            })
        }
        "array.mode" => {
            let (mut best, mut best_count, mut run) = (numbers[0], 0, 0);
            for (index, value) in numbers.iter().enumerate() {
                run = if index > 0 && numbers[index - 1] == *value {
                    run + 1
                } else {
                    1
                };
                if run > best_count {
                    best = *value;
                    best_count = run;
                }
            }
            float(best)
        }
        "array.range" => float(numbers[numbers.len() - 1] - numbers[0]),
        "array.variance" | "array.stdev" => {
            let mean = numbers.iter().sum::<f64>() / count;
            let squares: f64 = numbers.iter().map(|x| (x - mean) * (x - mean)).sum();
            let biased = bound
                .get("biased")
                .and_then(RuntimeValue::bool)
                .unwrap_or(true);
            if !(biased || numbers.len() > 1) {
                return RuntimeValue::Na;
            }
            let variance = squares / if biased { count } else { count - 1.0 };
            float(if name == "array.variance" {
                variance
            } else {
                variance.sqrt()
            })
        }
        "array.percentile_nearest_rank" => {
            let Some(percentage) = bound.get("percentage").and_then(RuntimeValue::number) else {
                return RuntimeValue::Na;
            };
            let rank = ((percentage / 100.0 * count).ceil().max(1.0)) as usize;
            float(numbers[rank.min(numbers.len()) - 1])
        }
        _ => {
            // percentile_linear_interpolation
            let Some(percentage) = bound.get("percentage").and_then(RuntimeValue::number) else {
                return RuntimeValue::Na;
            };
            let position = percentage.clamp(0.0, 100.0) / 100.0 * (count - 1.0);
            let lower = position.floor() as usize;
            let upper = (numbers.len() - 1).min(lower + 1);
            float(numbers[lower] + (numbers[upper] - numbers[lower]) * (position - lower as f64))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Bool, Float, Int, Na};

    struct Fixture {
        state: RuntimeState,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                state: RuntimeState::default(),
            }
        }

        /// Runs `name`, binding `arguments` to its parameter names positionally and by `named`.
        fn run(
            &mut self,
            name: &str,
            arguments: &[RuntimeValue],
        ) -> Result<RuntimeValue, Diagnostic> {
            let bound: Bound = parameters(name)
                .iter()
                .zip(arguments)
                .map(|(parameter, value)| (*parameter, value.clone()))
                .collect();
            call(
                name,
                &bound,
                arguments,
                &mut self.state,
                0.01,
                SourceRange::ZERO,
            )
        }

        fn ok(&mut self, name: &str, arguments: &[RuntimeValue]) -> RuntimeValue {
            self.run(name, arguments)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message))
        }

        fn array(&mut self, items: &[RuntimeValue]) -> RuntimeValue {
            new_array(&mut self.state, items.to_vec())
        }

        fn items(&self, array: &RuntimeValue) -> Vec<RuntimeValue> {
            match array {
                RuntimeValue::Ref(RefKind::Array, id) => self.state.arrays[id].clone(),
                other => panic!("not an array: {other:?}"),
            }
        }
    }

    fn s(text: &str) -> RuntimeValue {
        RuntimeValue::String(text.to_owned())
    }

    fn ints(values: &[i64]) -> Vec<RuntimeValue> {
        values.iter().map(|v| Int(*v)).collect()
    }

    #[test]
    fn constructors_make_sized_and_literal_arrays() {
        let mut f = Fixture::new();
        let filled = f.ok("array.new_float", &[Int(3), Float(1.5)]);
        assert_eq!(f.items(&filled), [Float(1.5), Float(1.5), Float(1.5)]);
        let empty = f.ok("array.new_int", &[]);
        assert_eq!(f.items(&empty), []);
        let negative = f.ok("array.new_int", &[Int(-4), Int(1)]);
        assert_eq!(f.items(&negative), [], "a negative size is empty");
        let from = f.ok("array.from", &ints(&[3, 1, 2]));
        assert_eq!(f.items(&from), ints(&[3, 1, 2]));
        assert_ne!(filled, from, "each array has its own handle");
        assert!(handles("array.new_color") && handles("array.from") && handles("array.push"));
        assert!(!handles("array.nope") && !handles("map.get"));
    }

    #[test]
    fn mutation_changes_the_array_in_place() {
        let mut f = Fixture::new();
        let a = f.array(&ints(&[1, 2]));
        assert_eq!(f.ok("array.push", &[a.clone(), Int(3)]), RuntimeValue::Void);
        f.ok("array.unshift", &[a.clone(), Int(0)]);
        assert_eq!(f.items(&a), ints(&[0, 1, 2, 3]));
        f.ok("array.set", &[a.clone(), Int(1), Int(9)]);
        f.ok("array.insert", &[a.clone(), Int(4), Int(7)]);
        assert_eq!(
            f.items(&a),
            ints(&[0, 9, 2, 3, 7]),
            "insert may append at the end"
        );
        assert_eq!(
            f.ok("array.remove", &[a.clone(), Int(1)]),
            Int(9),
            "remove returns the element"
        );
        assert_eq!(f.ok("array.shift", std::slice::from_ref(&a)), Int(0));
        assert_eq!(f.ok("array.pop", std::slice::from_ref(&a)), Int(7));
        assert_eq!(f.items(&a), ints(&[2, 3]));
        f.ok("array.reverse", std::slice::from_ref(&a));
        assert_eq!(f.items(&a), ints(&[3, 2]));
        f.ok("array.clear", std::slice::from_ref(&a));
        assert_eq!(
            f.ok("array.shift", std::slice::from_ref(&a)),
            Na,
            "shifting an empty array is na"
        );
        assert_eq!(f.ok("array.pop", &[a]), Na);
    }

    #[test]
    fn bad_indexes_are_runtime_errors_naming_the_size() {
        let mut f = Fixture::new();
        let a = f.array(&ints(&[1, 2]));
        for (name, arguments) in [
            ("array.get", vec![a.clone(), Int(2)]),
            ("array.get", vec![a.clone(), Int(-1)]),
            ("array.get", vec![a.clone(), Na]),
            ("array.set", vec![a.clone(), Int(5), Int(0)]),
            ("array.insert", vec![a.clone(), Int(3), Int(0)]),
            ("array.remove", vec![a.clone(), Int(2)]),
        ] {
            let error = f.run(name, &arguments).unwrap_err();
            assert_eq!(error.code, "PINE4010", "{name} {arguments:?}");
            assert!(
                error.message.contains("out of bounds (size 2)"),
                "{}",
                error.message
            );
        }
        assert!(
            f.run("array.get", &[a, Na])
                .unwrap_err()
                .message
                .contains("index na")
        );
    }

    #[test]
    fn a_non_array_is_pine4011_and_unknown_names_pine4007() {
        let mut f = Fixture::new();
        let error = f.run("array.size", &[Na]).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("PINE4011", "array.size requires an array; got na.")
        );
        assert_eq!(f.run("array.size", &[Int(3)]).unwrap_err().code, "PINE4011");
        let a = f.array(&[]);
        assert_eq!(
            f.run("array.nope", std::slice::from_ref(&a))
                .unwrap_err()
                .code,
            "PINE4007"
        );
        assert_eq!(
            f.run("array.concat", &[a, Na]).unwrap_err().code,
            "PINE4011"
        );
    }

    #[test]
    fn queries_read_without_changing() {
        let mut f = Fixture::new();
        let a = f.array(&[Int(5), Float(2.5), s("x"), Na]);
        assert_eq!(f.ok("array.size", std::slice::from_ref(&a)), Int(4));
        assert_eq!(f.ok("array.get", &[a.clone(), Int(1)]), Float(2.5));
        assert_eq!(f.ok("array.first", std::slice::from_ref(&a)), Int(5));
        assert_eq!(f.ok("array.last", std::slice::from_ref(&a)), Na);
        assert_eq!(f.ok("array.includes", &[a.clone(), s("x")]), Bool(true));
        assert_eq!(
            f.ok("array.includes", &[a.clone(), Float(5.0)]),
            Bool(false),
            "int 5 is not float 5.0"
        );
        assert_eq!(f.ok("array.indexof", &[a.clone(), s("x")]), Int(2));
        assert_eq!(f.ok("array.indexof", &[a.clone(), s("y")]), Int(-1));
        assert_eq!(
            f.ok("array.join", std::slice::from_ref(&a)),
            s("5,2.5,x,NaN")
        );
        assert_eq!(
            f.ok("array.join", &[a.clone(), s(" | ")]),
            s("5 | 2.5 | x | NaN")
        );
        assert_eq!(f.items(&a).len(), 4);
        let empty = f.array(&[]);
        assert_eq!(f.ok("array.first", std::slice::from_ref(&empty)), Na);
        assert_eq!(f.ok("array.last", &[empty]), Na);
    }

    #[test]
    fn copy_and_slice_make_new_arrays() {
        let mut f = Fixture::new();
        let a = f.array(&ints(&[1, 2, 3, 4]));
        let copy = f.ok("array.copy", std::slice::from_ref(&a));
        f.ok("array.push", &[copy.clone(), Int(5)]);
        assert_eq!(f.items(&a).len(), 4, "a copy is independent");
        assert_eq!(f.items(&copy).len(), 5);
        let middle = f.ok("array.slice", &[a.clone(), Int(1), Int(3)]);
        assert_eq!(f.items(&middle), ints(&[2, 3]));
        let tail = f.ok("array.slice", &[a.clone(), Int(2)]);
        assert_eq!(f.items(&tail), ints(&[3, 4]));
        let end = f.ok("array.slice", &[a.clone(), Int(4)]);
        assert_eq!(f.items(&end), [], "the start may be the end");
        assert_eq!(
            f.run("array.slice", &[a.clone(), Int(5)]).unwrap_err().code,
            "PINE4010"
        );
        assert!(
            f.run("array.slice", &[a.clone(), Int(3), Int(2)])
                .unwrap_err()
                .message
                .contains("end index 2")
        );
        assert!(f.run("array.slice", &[a, Int(0), Int(9)]).is_err());
    }

    #[test]
    fn concat_appends_and_returns_the_same_array() {
        let mut f = Fixture::new();
        let a = f.array(&ints(&[1]));
        let b = f.array(&ints(&[2, 3]));
        assert_eq!(f.ok("array.concat", &[a.clone(), b.clone()]), a);
        assert_eq!(f.items(&a), ints(&[1, 2, 3]));
        assert_eq!(f.items(&b), ints(&[2, 3]));
        f.ok("array.concat", &[a.clone(), a.clone()]);
        assert_eq!(f.items(&a).len(), 6, "an array can be appended to itself");
    }

    #[test]
    fn sorting_puts_numbers_first_then_the_rest_by_printed_text() {
        let mut f = Fixture::new();
        let a = f.array(&[s("b"), Int(3), Na, Float(1.5), s("a"), Int(2)]);
        f.ok("array.sort", std::slice::from_ref(&a));
        // Non-numbers compare by their printed text, and `na` prints as "NaN" — which sorts before
        // lowercase words, exactly as the Swift comparator does.
        assert_eq!(
            f.items(&a),
            [Float(1.5), Int(2), Int(3), Na, s("a"), s("b")]
        );
        f.ok("array.sort", &[a.clone(), s("order.descending")]);
        let descending = f.items(&a);
        assert_eq!(&descending[..3], [Int(3), Int(2), Float(1.5)]);
        assert_eq!(descending.len(), 6);
    }

    #[test]
    fn sort_indices_are_stable_and_leave_the_array_alone() {
        let mut f = Fixture::new();
        let a = f.array(&ints(&[30, 10, 20, 10]));
        let order = f.ok("array.sort_indices", std::slice::from_ref(&a));
        assert_eq!(
            f.items(&order),
            ints(&[1, 3, 2, 0]),
            "equal values keep their order"
        );
        let descending = f.ok("array.sort_indices", &[a.clone(), s("order.descending")]);
        assert_eq!(f.items(&descending), ints(&[0, 2, 1, 3]));
        assert_eq!(f.items(&a), ints(&[30, 10, 20, 10]));
    }

    #[test]
    fn aggregates_ignore_na() {
        let mut f = Fixture::new();
        let whole = f.array(&[Int(1), Na, Int(3)]);
        assert_eq!(
            f.ok("array.sum", std::slice::from_ref(&whole)),
            Int(4),
            "all ints: an int"
        );
        assert_eq!(f.ok("array.avg", std::slice::from_ref(&whole)), Float(2.0));
        assert_eq!(f.ok("array.max", std::slice::from_ref(&whole)), Float(3.0));
        assert_eq!(f.ok("array.min", &[whole]), Float(1.0));
        let mixed = f.array(&[Int(1), Float(2.5)]);
        assert_eq!(f.ok("array.sum", &[mixed]), Float(3.5));
        let empty = f.array(&[]);
        assert_eq!(f.ok("array.sum", std::slice::from_ref(&empty)), Int(0));
        assert_eq!(f.ok("array.avg", std::slice::from_ref(&empty)), Na);
        assert_eq!(f.ok("array.max", &[empty]), Na);
    }

    #[test]
    fn fill_search_and_normalising_follow_the_reference() {
        let mut f = Fixture::new();
        let a = f.array(&[Int(1), Int(3), Int(3), Int(5), Int(3)]);
        f.ok("array.fill", &[a.clone(), Int(9), Int(1), Int(3)]);
        assert_eq!(f.ok("array.get", &[a.clone(), Int(0)]), Int(1));
        assert_eq!(f.ok("array.get", &[a.clone(), Int(2)]), Int(9));
        assert_eq!(f.ok("array.get", &[a.clone(), Int(3)]), Int(5));
        f.ok("array.fill", &[a.clone(), Int(0)]);
        assert_eq!(f.ok("array.sum", std::slice::from_ref(&a)), Int(0));
        let repeated = f.array(&[Int(2), Int(7), Int(2)]);
        assert_eq!(
            f.ok("array.lastindexof", &[repeated.clone(), Int(2)]),
            Int(2)
        );
        assert_eq!(f.ok("array.lastindexof", &[repeated, Int(4)]), Int(-1));
        let sorted = f.array(&[Int(1), Int(3), Int(3), Int(5)]);
        let search =
            |f: &mut Fixture, name: &str, target: i64| f.ok(name, &[sorted.clone(), Int(target)]);
        assert!(
            search(&mut f, "array.binary_search", 3)
                .int_value()
                .is_some_and(|i| i == 1 || i == 2)
        );
        assert_eq!(search(&mut f, "array.binary_search", 4), Int(-1));
        assert_eq!(search(&mut f, "array.binary_search_leftmost", 3), Int(1));
        assert_eq!(search(&mut f, "array.binary_search_rightmost", 3), Int(2));
        assert_eq!(search(&mut f, "array.binary_search_leftmost", 4), Int(2));
        assert_eq!(search(&mut f, "array.binary_search_rightmost", 4), Int(3));
        assert_eq!(search(&mut f, "array.binary_search_leftmost", 0), Int(-1));
        assert_eq!(search(&mut f, "array.binary_search_rightmost", 9), Int(4));
        let signed = f.array(&[Int(-2), Float(1.5), Float(-0.5)]);
        let absolute = f.ok("array.abs", &[signed]);
        assert_eq!(f.ok("array.get", &[absolute.clone(), Int(0)]), Int(2));
        assert_eq!(f.ok("array.get", &[absolute, Int(2)]), Float(0.5));
        let spread = f.array(&[Int(2), Int(4), Int(6)]);
        let scaled = f.ok("array.standardize", &[spread]);
        let first = f
            .ok("array.get", &[scaled.clone(), Int(0)])
            .number()
            .unwrap();
        assert!((first + 1.224_744_871_391_589).abs() < 1e-12, "{first}");
        assert_eq!(f.ok("array.get", &[scaled, Int(1)]), Float(0.0));
    }

    #[test]
    fn statistics_follow_pines_population_defaults() {
        let mut f = Fixture::new();
        let a = f.array(&[
            Int(2),
            Int(4),
            Int(4),
            Int(4),
            Int(5),
            Int(5),
            Int(7),
            Int(9),
        ]);
        assert_eq!(f.ok("array.median", std::slice::from_ref(&a)), Float(4.5));
        assert_eq!(f.ok("array.mode", std::slice::from_ref(&a)), Float(4.0));
        assert_eq!(f.ok("array.range", std::slice::from_ref(&a)), Float(7.0));
        assert_eq!(f.ok("array.variance", std::slice::from_ref(&a)), Float(4.0));
        assert_eq!(f.ok("array.stdev", std::slice::from_ref(&a)), Float(2.0));
        let sample = f.ok("array.variance", &[a.clone(), Bool(false)]);
        assert!(
            (sample.number().unwrap() - 32.0 / 7.0).abs() < 1e-12,
            "{sample:?}"
        );
        assert_eq!(
            f.ok("array.percentile_nearest_rank", &[a.clone(), Int(50)]),
            Float(4.0)
        );
        assert_eq!(
            f.ok("array.percentile_nearest_rank", &[a.clone(), Int(100)]),
            Float(9.0)
        );
        assert_eq!(
            f.ok("array.percentile_nearest_rank", &[a.clone(), Int(0)]),
            Float(2.0),
            "rank 1 at the least"
        );
        assert_eq!(
            f.ok(
                "array.percentile_linear_interpolation",
                &[a.clone(), Int(50)]
            ),
            Float(4.5)
        );
        assert_eq!(
            f.ok(
                "array.percentile_linear_interpolation",
                &[a.clone(), Int(0)]
            ),
            Float(2.0)
        );
        assert_eq!(
            f.ok(
                "array.percentile_linear_interpolation",
                &[a.clone(), Int(150)]
            ),
            Float(9.0),
            "clamped"
        );
        assert_eq!(f.ok("array.percentile_linear_interpolation", &[a, Na]), Na);
        let one = f.array(&[Int(5)]);
        assert_eq!(
            f.ok("array.variance", &[one, Bool(false)]),
            Na,
            "sample variance needs two"
        );
        let nothing = f.array(&[Na]);
        assert_eq!(f.ok("array.median", &[nothing]), Na);
    }

    #[test]
    fn covariance_pairs_up_the_numbers() {
        let mut f = Fixture::new();
        let x = f.array(&ints(&[1, 2, 3, 4]));
        let y = f.array(&ints(&[2, 4, 6, 8]));
        // Population covariance of a line y = 2x: 2 × var(x) = 2 × 1.25.
        assert_eq!(
            f.ok("array.covariance", &[x.clone(), y.clone()]),
            Float(2.5)
        );
        let sample = f.ok("array.covariance", &[x.clone(), y.clone(), Bool(false)]);
        assert!((sample.number().unwrap() - 10.0 / 3.0).abs() < 1e-12);
        let single = f.array(&ints(&[1]));
        assert_eq!(
            f.ok(
                "array.covariance",
                &[single.clone(), single.clone(), Bool(false)]
            ),
            Na
        );
        assert_eq!(
            f.ok("array.covariance", &[single.clone(), single]),
            Float(0.0)
        );
        assert_eq!(
            f.run("array.covariance", &[x, Na]).unwrap_err().code,
            "PINE4011"
        );
    }

    #[test]
    fn bound_arguments_are_found_by_name_and_replaced_on_rebind() {
        let mut bound = Bound::default();
        assert_eq!(bound.get("id"), None);
        bound.insert("id", Int(1));
        bound.insert("index", Int(2));
        bound.insert("id", Int(3));
        assert_eq!(bound.get("id"), Some(&Int(3)));
        assert_eq!(bound.get("index"), Some(&Int(2)));
        assert_eq!(bound.get("value"), None);
        let collected: Bound = [("a", Int(1)), ("b", Na)].into_iter().collect();
        assert_eq!(collected.get("b"), Some(&Na));
        let mut full = Bound::default();
        for name in ["a", "b", "c", "d", "e"] {
            full.insert(name, Int(0));
        }
        assert_eq!(full.get("e"), Some(&Int(0)), "five parameters fit");
    }
}
