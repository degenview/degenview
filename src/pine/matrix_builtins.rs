//! `matrix.*`, ported from `PineRuntimeSession+Matrices.swift`: `matrix.new<T>(rows, columns,
//! initial)` and the operations a matrix takes, also as methods (`m.get(r, c)`). Element-wise
//! matrix arithmetic and linear algebra are not supported. Pure over the run's `RuntimeState`.
//!
//! One deliberate difference: `matrix.row` / `matrix.col` with a missing (`na`) index are an
//! out-of-bounds error here; Swift resolved a missing index to `count` and then read past the end.

use super::array_builtins::{Bound, new_array};
use super::containers::Matrix;
use super::runtime_value::{RefKind, RuntimeValue};
use super::state::RuntimeState;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};

/// Parameter names per operation (by member name, after `matrix.`), in positional order.
pub fn parameters(name: &str) -> &'static [&'static str] {
    match name.strip_prefix("matrix.").unwrap_or(name) {
        "new" => &["rows", "columns", "initial_value"],
        "get" => &["id", "row", "column"],
        "set" => &["id", "row", "column", "value"],
        "row" | "remove_row" => &["id", "row"],
        "col" | "remove_col" => &["id", "column"],
        "add_row" => &["id", "row", "array_id"],
        "add_col" => &["id", "column", "array_id"],
        "fill" => &["id", "value"],
        _ => &["id"],
    }
}

const MEMBERS: [&str; 17] = [
    "new",
    "get",
    "set",
    "rows",
    "columns",
    "elements_count",
    "row",
    "col",
    "add_row",
    "add_col",
    "remove_row",
    "remove_col",
    "fill",
    "copy",
    "transpose",
    "avg",
    "min",
];

/// Whether `name` is a `matrix.*` function this module handles.
pub fn handles(name: &str) -> bool {
    name.strip_prefix("matrix.")
        .is_some_and(|member| MEMBERS.contains(&member) || member == "max")
}

fn runtime_error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

fn new_matrix(state: &mut RuntimeState, matrix: Matrix) -> RuntimeValue {
    state.next_reference += 1;
    let id = state.next_reference;
    state.matrices.insert(id, matrix);
    RuntimeValue::Ref(RefKind::Matrix, id)
}

fn cell(
    bound: &Bound,
    matrix: &Matrix,
    name: &str,
    range: SourceRange,
) -> Result<(usize, usize), Diagnostic> {
    let row = bound.get("row").and_then(RuntimeValue::int_value);
    let column = bound.get("column").and_then(RuntimeValue::int_value);
    match (row, column) {
        (Some(row), Some(column))
            if row >= 0 && column >= 0 && matrix.contains(row as usize, column as usize) =>
        {
            Ok((row as usize, column as usize))
        }
        _ => {
            let shown = |value: Option<i64>| value.map_or("na".to_owned(), |v| v.to_string());
            Err(runtime_error(
                "PINE4010",
                format!(
                    "{name} cell ({}, {}) is out of bounds ({}×{}).",
                    shown(row),
                    shown(column),
                    matrix.rows(),
                    matrix.columns()
                ),
                range,
            ))
        }
    }
}

fn bounds(
    name: &str,
    what: &str,
    shown: Option<i64>,
    count: usize,
    range: SourceRange,
) -> Diagnostic {
    runtime_error(
        "PINE4010",
        format!(
            "{name} {what} {} is out of bounds (size {count}).",
            shown.map_or("na".to_owned(), |v| v.to_string())
        ),
        range,
    )
}

/// A row or column index; `allow_end` admits `count` (append). An absent or `na` index means
/// `default` when there is one (removals default to the last), else the end for appends.
fn index(
    value: Option<&RuntimeValue>,
    count: usize,
    allow_end: bool,
    default: Option<usize>,
    name: &str,
    range: SourceRange,
) -> Result<usize, Diagnostic> {
    let Some(value) = value.filter(|value| **value != RuntimeValue::Na) else {
        return match default {
            Some(default) => Ok(default),
            None if allow_end => Ok(count),
            None => Err(bounds(name, "index", None, count, range)),
        };
    };
    let limit = count + usize::from(allow_end);
    match value.int_value() {
        Some(index) if index >= 0 && (index as usize) < limit => Ok(index as usize),
        other => Err(bounds(name, "index", other, count, range)),
    }
}

fn values_of(state: &RuntimeState, array: Option<&RuntimeValue>) -> Vec<RuntimeValue> {
    match array {
        Some(RuntimeValue::Ref(RefKind::Array, id)) => {
            state.arrays.get(id).cloned().unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

pub fn call(
    name: &str,
    bound: &Bound,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let member = name.strip_prefix("matrix.").unwrap_or(name);
    if member == "new" {
        let dimension = |key: &str| {
            bound
                .get(key)
                .and_then(RuntimeValue::int_value)
                .map_or(0, |n| n.max(0) as usize)
        };
        let initial = bound
            .get("initial_value")
            .cloned()
            .unwrap_or(RuntimeValue::Na);
        return Ok(new_matrix(
            state,
            Matrix::new(dimension("rows"), dimension("columns"), initial),
        ));
    }
    let not_a_matrix = || {
        runtime_error(
            "PINE4028",
            format!("{name} requires a matrix; got na."),
            range,
        )
    };
    let Some(RuntimeValue::Ref(RefKind::Matrix, id)) = bound.get("id") else {
        return Err(not_a_matrix());
    };
    let id = *id;
    // The matrix leaves the table while one operation runs on it, so `m.get(r, c)` in a loop does not
    // copy the whole matrix.
    let mut matrix = state.matrices.remove(&id).ok_or_else(not_a_matrix)?;
    let outcome = apply(member, name, bound, &mut matrix, state, range);
    state.matrices.insert(id, matrix);
    outcome
}

fn apply(
    member: &str,
    name: &str,
    bound: &Bound,
    matrix: &mut Matrix,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let result = match member {
        "get" => {
            let (row, column) = cell(bound, matrix, name, range)?;
            matrix.get(row, column).clone()
        }
        "set" => {
            let (row, column) = cell(bound, matrix, name, range)?;
            matrix.set(
                row,
                column,
                bound.get("value").cloned().unwrap_or(RuntimeValue::Na),
            );
            RuntimeValue::Void
        }
        "rows" => RuntimeValue::Int(matrix.rows() as i64),
        "columns" => RuntimeValue::Int(matrix.columns() as i64),
        "elements_count" => RuntimeValue::Int(matrix.count() as i64),
        "row" => {
            let at = index(bound.get("row"), matrix.rows(), false, None, name, range)?;
            return Ok(new_array(state, matrix.row(at)));
        }
        "col" => {
            let at = index(
                bound.get("column"),
                matrix.columns(),
                false,
                None,
                name,
                range,
            )?;
            return Ok(new_array(state, matrix.column(at)));
        }
        "add_row" => {
            let at = index(bound.get("row"), matrix.rows(), true, None, name, range)?;
            let values = values_of(state, bound.get("array_id"));
            // The first row added to a 0×0 matrix gives it its width.
            if matrix.is_empty() {
                matrix.reshape_empty(0, values.len());
            }
            matrix.insert_row(at, &values);
            RuntimeValue::Void
        }
        "add_col" => {
            let at = index(
                bound.get("column"),
                matrix.columns(),
                true,
                None,
                name,
                range,
            )?;
            let values = values_of(state, bound.get("array_id"));
            if matrix.is_empty() {
                matrix.reshape_empty(values.len(), 0);
            }
            matrix.insert_column(at, &values);
            RuntimeValue::Void
        }
        "remove_row" => {
            if matrix.rows() == 0 {
                return Err(bounds(name, "row", None, 0, range));
            }
            let last = matrix.rows() - 1;
            let at = index(
                bound.get("row"),
                matrix.rows(),
                false,
                Some(last),
                name,
                range,
            )?;
            matrix.remove_row(at);
            RuntimeValue::Void
        }
        "remove_col" => {
            if matrix.columns() == 0 {
                return Err(bounds(name, "column", None, 0, range));
            }
            let last = matrix.columns() - 1;
            let at = index(
                bound.get("column"),
                matrix.columns(),
                false,
                Some(last),
                name,
                range,
            )?;
            matrix.remove_column(at);
            RuntimeValue::Void
        }
        "fill" => {
            matrix.fill(bound.get("value").cloned().unwrap_or(RuntimeValue::Na));
            RuntimeValue::Void
        }
        "copy" => return Ok(new_matrix(state, matrix.clone())),
        "transpose" => {
            let transposed = matrix.transposed();
            return Ok(new_matrix(state, transposed));
        }
        "avg" | "min" | "max" => {
            let numbers: Vec<f64> = matrix
                .all_elements()
                .iter()
                .filter_map(RuntimeValue::number)
                .collect();
            if numbers.is_empty() {
                return Ok(RuntimeValue::Na);
            }
            return Ok(RuntimeValue::Float(match member {
                "avg" => numbers.iter().sum::<f64>() / numbers.len() as f64,
                "min" => numbers.iter().copied().fold(f64::INFINITY, f64::min),
                _ => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            }));
        }
        _ => {
            return Err(runtime_error(
                "PINE4007",
                format!("Unknown or unsupported function '{name}'."),
                range,
            ));
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Float, Int, Na};

    fn run(
        state: &mut RuntimeState,
        name: &str,
        arguments: &[RuntimeValue],
    ) -> Result<RuntimeValue, Diagnostic> {
        let bound: Bound = parameters(name)
            .iter()
            .zip(arguments)
            .map(|(parameter, value)| (*parameter, value.clone()))
            .collect();
        call(name, &bound, state, SourceRange::ZERO)
    }

    fn ok(state: &mut RuntimeState, name: &str, arguments: &[RuntimeValue]) -> RuntimeValue {
        run(state, name, arguments).unwrap_or_else(|e| panic!("{name}: {}", e.message))
    }

    fn items(state: &RuntimeState, array: &RuntimeValue) -> Vec<RuntimeValue> {
        match array {
            RuntimeValue::Ref(RefKind::Array, id) => state.arrays[id].clone(),
            other => panic!("not an array: {other:?}"),
        }
    }

    /// A 2×3 matrix holding 1…6 row by row.
    fn sample(state: &mut RuntimeState) -> RuntimeValue {
        let m = ok(state, "matrix.new", &[Int(2), Int(3), Int(0)]);
        for row in 0..2 {
            for column in 0..3 {
                ok(
                    state,
                    "matrix.set",
                    &[m.clone(), Int(row), Int(column), Int(row * 3 + column + 1)],
                );
            }
        }
        m
    }

    #[test]
    fn a_new_matrix_has_its_shape_and_initial_value() {
        let mut state = RuntimeState::default();
        let m = ok(&mut state, "matrix.new", &[Int(2), Int(3), Float(1.5)]);
        assert_eq!(
            ok(&mut state, "matrix.rows", std::slice::from_ref(&m)),
            Int(2)
        );
        assert_eq!(
            ok(&mut state, "matrix.columns", std::slice::from_ref(&m)),
            Int(3)
        );
        assert_eq!(
            ok(
                &mut state,
                "matrix.elements_count",
                std::slice::from_ref(&m)
            ),
            Int(6)
        );
        assert_eq!(
            ok(&mut state, "matrix.get", &[m, Int(1), Int(2)]),
            Float(1.5)
        );
        let empty = ok(&mut state, "matrix.new", &[Int(-2), Na]);
        assert_eq!(
            ok(&mut state, "matrix.elements_count", &[empty]),
            Int(0),
            "negative or na: empty"
        );
    }

    #[test]
    fn get_and_set_check_the_cell() {
        let mut state = RuntimeState::default();
        let m = sample(&mut state);
        assert_eq!(
            ok(&mut state, "matrix.get", &[m.clone(), Int(1), Int(0)]),
            Int(4)
        );
        for (row, column) in [(2, 0), (0, 3), (-1, 0), (0, -1)] {
            let error = run(
                &mut state,
                "matrix.get",
                &[m.clone(), Int(row), Int(column)],
            )
            .unwrap_err();
            assert_eq!(error.code, "PINE4010");
            assert!(error.message.contains("(2×3)"), "{}", error.message);
        }
        let error = run(&mut state, "matrix.set", &[m.clone(), Na, Int(0), Int(1)]).unwrap_err();
        assert!(error.message.contains("cell (na, 0)"), "{}", error.message);
        assert_eq!(
            run(&mut state, "matrix.get", &[Na, Int(0), Int(0)])
                .unwrap_err()
                .code,
            "PINE4028"
        );
    }

    #[test]
    fn rows_and_columns_come_back_as_arrays() {
        let mut state = RuntimeState::default();
        let m = sample(&mut state);
        let row = ok(&mut state, "matrix.row", &[m.clone(), Int(1)]);
        assert_eq!(items(&state, &row), [Int(4), Int(5), Int(6)]);
        let column = ok(&mut state, "matrix.col", &[m.clone(), Int(2)]);
        assert_eq!(items(&state, &column), [Int(3), Int(6)]);
        assert_eq!(
            run(&mut state, "matrix.row", &[m.clone(), Int(2)])
                .unwrap_err()
                .code,
            "PINE4010"
        );
        assert_eq!(
            run(&mut state, "matrix.row", &[m.clone(), Na])
                .unwrap_err()
                .code,
            "PINE4010",
            "no read past the end"
        );
        assert!(run(&mut state, "matrix.col", &[m, Int(-1)]).is_err());
    }

    #[test]
    fn rows_and_columns_can_be_added_and_removed() {
        let mut state = RuntimeState::default();
        let m = sample(&mut state);
        state.next_reference += 1;
        let array_id = state.next_reference;
        state.arrays.insert(array_id, vec![Int(7), Int(8), Int(9)]);
        let array = RuntimeValue::Ref(RefKind::Array, array_id);
        ok(
            &mut state,
            "matrix.add_row",
            &[m.clone(), Na, array.clone()],
        );
        assert_eq!(
            ok(&mut state, "matrix.rows", std::slice::from_ref(&m)),
            Int(3),
            "no index: appended"
        );
        assert_eq!(
            ok(&mut state, "matrix.get", &[m.clone(), Int(2), Int(1)]),
            Int(8)
        );
        ok(
            &mut state,
            "matrix.add_row",
            &[m.clone(), Int(0), array.clone()],
        );
        assert_eq!(
            ok(&mut state, "matrix.get", &[m.clone(), Int(0), Int(0)]),
            Int(7),
            "inserted at the front"
        );
        assert_eq!(
            run(
                &mut state,
                "matrix.add_row",
                &[m.clone(), Int(9), array.clone()]
            )
            .unwrap_err()
            .code,
            "PINE4010"
        );
        ok(&mut state, "matrix.remove_row", std::slice::from_ref(&m));
        assert_eq!(
            ok(&mut state, "matrix.rows", std::slice::from_ref(&m)),
            Int(3),
            "no index: the last row goes"
        );
        ok(&mut state, "matrix.remove_row", &[m.clone(), Int(0)]);
        assert_eq!(
            ok(&mut state, "matrix.get", &[m.clone(), Int(0), Int(0)]),
            Int(1)
        );

        ok(&mut state, "matrix.add_col", &[m.clone(), Int(1), array]);
        assert_eq!(
            ok(&mut state, "matrix.columns", std::slice::from_ref(&m)),
            Int(4)
        );
        assert_eq!(
            ok(&mut state, "matrix.get", &[m.clone(), Int(0), Int(1)]),
            Int(7)
        );
        assert_eq!(
            ok(&mut state, "matrix.get", &[m.clone(), Int(1), Int(1)]),
            Int(8)
        );
        ok(&mut state, "matrix.remove_col", &[m.clone(), Int(1)]);
        ok(&mut state, "matrix.remove_col", std::slice::from_ref(&m));
        assert_eq!(
            ok(&mut state, "matrix.columns", std::slice::from_ref(&m)),
            Int(2)
        );
        let empty = ok(&mut state, "matrix.new", &[Int(0), Int(0)]);
        assert_eq!(
            run(
                &mut state,
                "matrix.remove_row",
                std::slice::from_ref(&empty)
            )
            .unwrap_err()
            .code,
            "PINE4010"
        );
        assert_eq!(
            run(&mut state, "matrix.remove_col", &[empty])
                .unwrap_err()
                .code,
            "PINE4010"
        );
    }

    #[test]
    fn fill_copy_and_transpose() {
        let mut state = RuntimeState::default();
        let m = sample(&mut state);
        let copy = ok(&mut state, "matrix.copy", std::slice::from_ref(&m));
        ok(&mut state, "matrix.fill", &[m.clone(), Int(0)]);
        assert_eq!(ok(&mut state, "matrix.get", &[m, Int(1), Int(2)]), Int(0));
        assert_eq!(
            ok(&mut state, "matrix.get", &[copy.clone(), Int(1), Int(2)]),
            Int(6),
            "the copy kept its values"
        );
        let transposed = ok(&mut state, "matrix.transpose", &[copy]);
        assert_eq!(
            ok(&mut state, "matrix.rows", std::slice::from_ref(&transposed)),
            Int(3)
        );
        assert_eq!(
            ok(&mut state, "matrix.get", &[transposed, Int(2), Int(1)]),
            Int(6)
        );
    }

    #[test]
    fn statistics_ignore_non_numbers() {
        let mut state = RuntimeState::default();
        let m = sample(&mut state);
        assert_eq!(
            ok(&mut state, "matrix.avg", std::slice::from_ref(&m)),
            Float(3.5)
        );
        assert_eq!(
            ok(&mut state, "matrix.min", std::slice::from_ref(&m)),
            Float(1.0)
        );
        assert_eq!(
            ok(&mut state, "matrix.max", std::slice::from_ref(&m)),
            Float(6.0)
        );
        ok(&mut state, "matrix.fill", &[m.clone(), Na]);
        assert_eq!(ok(&mut state, "matrix.avg", &[m]), Na);
        assert_eq!(
            run(&mut state, "matrix.nope", &[Na]).unwrap_err().code,
            "PINE4028"
        );
        let m = ok(&mut state, "matrix.new", &[Int(1), Int(1)]);
        assert_eq!(
            run(&mut state, "matrix.nope", &[m]).unwrap_err().code,
            "PINE4007"
        );
        assert!(
            handles("matrix.new")
                && handles("matrix.max")
                && !handles("matrix.nope")
                && !handles("map.get")
        );
    }
}
