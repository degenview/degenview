//! `matrix.*`, ported from `PineRuntimeSession+Matrices.swift`: `matrix.new<T>(rows, columns,
//! initial)` and the operations a matrix takes, also as methods (`m.get(r, c)`). Element-wise
//! matrix arithmetic (`sum`, `diff`, `mult`, `pow`, `kron`), reshaping and sorting work on the
//! interpreter's values; linear algebra works on `f64`s in `pine::linalg`. Pure over the run's
//! `RuntimeState`.
//!
//! One deliberate difference: `matrix.row` / `matrix.col` with a missing (`na`) index are an
//! out-of-bounds error here; Swift resolved a missing index to `count` and then read past the end.

use super::array_builtins::{self, Bound, new_array};
use super::ast::BinaryOperator;
use super::containers::Matrix;
use super::linalg::Dense;
use super::operators;
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
        "fill" => &[
            "id",
            "value",
            "from_row",
            "to_row",
            "from_column",
            "to_column",
        ],
        "sum" | "diff" | "mult" | "kron" | "concat" => &["id1", "id2"],
        "pow" => &["id", "power"],
        "sort" => &["id", "column", "order"],
        "reshape" => &["id", "rows", "columns"],
        "submatrix" => &["id", "from_row", "to_row", "from_column", "to_column"],
        "swap_rows" => &["id", "row1", "row2"],
        "swap_columns" => &["id", "column1", "column2"],
        _ => &["id"],
    }
}

/// Every `matrix.*` function this module handles, without the prefix.
pub const MEMBERS: [&str; 33] = [
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
    "max",
    "median",
    "mode",
    "sum",
    "diff",
    "mult",
    "pow",
    "kron",
    "concat",
    "reverse",
    "sort",
    "reshape",
    "submatrix",
    "swap_rows",
    "swap_columns",
    "trace",
];

/// The functions that take two matrices (or a matrix and a number or vector) as `id1` and `id2`.
const BINARY: [&str; 5] = ["sum", "diff", "mult", "kron", "concat"];

/// Whether `name` is a `matrix.*` function this module handles.
pub fn handles(name: &str) -> bool {
    name.strip_prefix("matrix.")
        .is_some_and(|member| MEMBERS.contains(&member))
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

/// The second operand of a two-matrix function: another matrix, a vector (`matrix.mult` takes an
/// array), or a number. Read before the first matrix leaves the table, so `matrix.sum(m, m)` works.
enum Operand {
    Matrix(Matrix),
    Vector(Vec<RuntimeValue>),
    Scalar(RuntimeValue),
}

/// What one call works with besides the matrix it runs on.
struct Call<'a> {
    member: &'a str,
    name: &'a str,
    bound: &'a Bound,
    mintick: f64,
    range: SourceRange,
    /// The matrix's own handle, which `matrix.concat` returns.
    id: usize,
    operand: Option<Operand>,
}

pub fn call(
    name: &str,
    bound: &Bound,
    state: &mut RuntimeState,
    mintick: f64,
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
    let binary = BINARY.contains(&member);
    let Some(RuntimeValue::Ref(RefKind::Matrix, id)) = bound.get(if binary { "id1" } else { "id" })
    else {
        return Err(not_a_matrix());
    };
    let id = *id;
    let operand = if binary {
        Some(match bound.get("id2") {
            Some(RuntimeValue::Ref(RefKind::Matrix, other)) => Operand::Matrix(
                state
                    .matrices
                    .get(other)
                    .cloned()
                    .ok_or_else(not_a_matrix)?,
            ),
            Some(RuntimeValue::Ref(RefKind::Array, other)) => {
                Operand::Vector(state.arrays.get(other).cloned().unwrap_or_default())
            }
            other => Operand::Scalar(other.cloned().unwrap_or(RuntimeValue::Na)),
        })
    } else {
        None
    };
    // The matrix leaves the table while one operation runs on it, so `m.get(r, c)` in a loop does not
    // copy the whole matrix.
    let mut matrix = state.matrices.remove(&id).ok_or_else(not_a_matrix)?;
    let call = Call {
        member,
        name,
        bound,
        mintick,
        range,
        id,
        operand,
    };
    let outcome = apply(&call, &mut matrix, state);
    state.matrices.insert(id, matrix);
    outcome
}

fn apply(
    call: &Call,
    matrix: &mut Matrix,
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let Call {
        member,
        name,
        bound,
        range,
        ..
    } = *call;
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
            let rows = span(call, matrix.rows(), "from_row", "to_row", "row")?;
            let columns = span(call, matrix.columns(), "from_column", "to_column", "column")?;
            let value = bound.get("value").cloned().unwrap_or(RuntimeValue::Na);
            matrix.fill_range(rows, columns, &value);
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
        "median" | "mode" => {
            return Ok(array_builtins::statistic(
                if member == "median" {
                    "array.median"
                } else {
                    "array.mode"
                },
                &Bound::default(),
                matrix.all_elements(),
            ));
        }
        "trace" => return Ok(trace(matrix)),
        "sum" | "diff" | "mult" | "kron" | "concat" => return binary(call, matrix, state),
        "pow" => {
            let result = power(call, matrix)?;
            return Ok(new_matrix(state, result));
        }
        "reverse" => {
            matrix.reverse();
            RuntimeValue::Void
        }
        "swap_rows" => {
            let first = index(bound.get("row1"), matrix.rows(), false, None, name, range)?;
            let second = index(bound.get("row2"), matrix.rows(), false, None, name, range)?;
            matrix.swap_rows(first, second);
            RuntimeValue::Void
        }
        "swap_columns" => {
            let first = index(
                bound.get("column1"),
                matrix.columns(),
                false,
                None,
                name,
                range,
            )?;
            let second = index(
                bound.get("column2"),
                matrix.columns(),
                false,
                None,
                name,
                range,
            )?;
            matrix.swap_columns(first, second);
            RuntimeValue::Void
        }
        "sort" => {
            if matrix.rows() > 0 && matrix.columns() > 0 {
                let column = index(
                    bound.get("column"),
                    matrix.columns(),
                    false,
                    Some(0),
                    name,
                    range,
                )?;
                let descending = bound.get("order").and_then(RuntimeValue::text_value)
                    == Some("order.descending");
                let order: Vec<usize> = array_builtins::sorted_indices(
                    &matrix.column(column),
                    descending,
                    call.mintick,
                )
                .iter()
                .filter_map(RuntimeValue::int_value)
                .map(|row| row as usize)
                .collect();
                *matrix = matrix.with_rows_ordered(&order);
            }
            RuntimeValue::Void
        }
        "reshape" => {
            let dimension = |key: &str| bound.get(key).and_then(RuntimeValue::int_value);
            let (Some(rows), Some(columns)) = (dimension("rows"), dimension("columns")) else {
                return Err(dimensions(
                    name,
                    "reshape needs a number of rows and columns",
                    range,
                ));
            };
            if rows < 0 || columns < 0 || (rows as usize) * (columns as usize) != matrix.count() {
                return Err(dimensions(
                    name,
                    &format!(
                        "cannot reshape {}×{} ({} cells) to {rows}×{columns}",
                        matrix.rows(),
                        matrix.columns(),
                        matrix.count()
                    ),
                    range,
                ));
            }
            matrix.reshape(rows as usize, columns as usize);
            RuntimeValue::Void
        }
        "submatrix" => {
            let rows = span(call, matrix.rows(), "from_row", "to_row", "row")?;
            let columns = span(call, matrix.columns(), "from_column", "to_column", "column")?;
            return Ok(new_matrix(state, matrix.submatrix(rows, columns)));
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

/// A shape or operand a function cannot work with.
fn dimensions(name: &str, why: &str, range: SourceRange) -> Diagnostic {
    runtime_error("PINE4037", format!("{name}: {why}."), range)
}

/// The half-open run `[from, to)` along one axis of `count` cells; an absent or `na` end means the
/// start (0) or the whole axis.
fn span(
    call: &Call,
    count: usize,
    from_key: &str,
    to_key: &str,
    what: &str,
) -> Result<std::ops::Range<usize>, Diagnostic> {
    let end = |key: &str, default: usize| -> Result<usize, Diagnostic> {
        match call
            .bound
            .get(key)
            .filter(|value| **value != RuntimeValue::Na)
        {
            None => Ok(default),
            Some(value) => match value.int_value() {
                Some(index) if index >= 0 && index as usize <= count => Ok(index as usize),
                other => Err(bounds(
                    call.name,
                    &format!("{what} index"),
                    other,
                    count,
                    call.range,
                )),
            },
        }
    };
    let from = end(from_key, 0)?;
    let to = end(to_key, count)?;
    if from > to {
        return Err(dimensions(
            call.name,
            &format!("the {what} range starts at {from}, after it ends at {to}"),
            call.range,
        ));
    }
    Ok(from..to)
}

/// The sum of the main diagonal's numbers; `na` for a matrix without any.
fn trace(matrix: &Matrix) -> RuntimeValue {
    let diagonal: Vec<f64> = (0..matrix.rows().min(matrix.columns()))
        .filter_map(|at| matrix.get(at, at).number())
        .collect();
    if diagonal.is_empty() {
        RuntimeValue::Na
    } else {
        RuntimeValue::Float(diagonal.iter().sum())
    }
}

/// A matrix as `f64`s; an element that is not a number is NaN.
fn dense(matrix: &Matrix) -> Dense {
    Dense::new(
        matrix.rows(),
        matrix.columns(),
        matrix
            .all_elements()
            .iter()
            .map(|element| element.number().unwrap_or(f64::NAN))
            .collect(),
    )
}

/// The inverse of `dense`: NaN is `na`.
fn matrix_of(dense: &Dense) -> Matrix {
    Matrix::from_elements(
        dense.rows(),
        dense.columns(),
        dense
            .data()
            .iter()
            .map(|value| {
                if value.is_nan() {
                    RuntimeValue::Na
                } else {
                    RuntimeValue::Float(*value)
                }
            })
            .collect(),
    )
}

/// `matrix.pow(id, power)`: the square matrix times itself `power` times, as floats.
fn power(call: &Call, matrix: &Matrix) -> Result<Matrix, Diagnostic> {
    if !matrix.is_square() {
        return Err(dimensions(
            call.name,
            &format!(
                "needs a square matrix, got {}×{}",
                matrix.rows(),
                matrix.columns()
            ),
            call.range,
        ));
    }
    let Some(exponent) = call.bound.get("power").and_then(RuntimeValue::int_value) else {
        return Err(dimensions(
            call.name,
            "needs a whole number power",
            call.range,
        ));
    };
    let Ok(exponent) = u64::try_from(exponent) else {
        return Err(dimensions(
            call.name,
            "a negative power is not supported",
            call.range,
        ));
    };
    Ok(matrix_of(&dense(matrix).power(exponent)))
}

/// `sum`, `diff`, `mult`, `kron` and `concat`. Arithmetic keeps an element an int while both of its
/// operands are ints, like the `+`, `-` and `*` operators it reuses.
fn binary(
    call: &Call,
    matrix: &mut Matrix,
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let Some(operand) = &call.operand else {
        unreachable!("a binary function always has an operand");
    };
    let (name, range) = (call.name, call.range);
    let same_shape = |other: &Matrix| -> Result<(), Diagnostic> {
        if other.rows() == matrix.rows() && other.columns() == matrix.columns() {
            Ok(())
        } else {
            Err(dimensions(
                name,
                &format!(
                    "the matrices are {}×{} and {}×{}",
                    matrix.rows(),
                    matrix.columns(),
                    other.rows(),
                    other.columns()
                ),
                range,
            ))
        }
    };
    let result = match (call.member, operand) {
        ("sum" | "diff", Operand::Matrix(other)) => {
            same_shape(other)?;
            let op = arithmetic_operator(call.member);
            Matrix::from_elements(
                matrix.rows(),
                matrix.columns(),
                matrix
                    .all_elements()
                    .iter()
                    .zip(other.all_elements())
                    .map(|(a, b)| operators::apply(op, a, b))
                    .collect(),
            )
        }
        ("sum" | "diff" | "mult", Operand::Scalar(scalar)) => {
            let op = arithmetic_operator(call.member);
            Matrix::from_elements(
                matrix.rows(),
                matrix.columns(),
                matrix
                    .all_elements()
                    .iter()
                    .map(|element| operators::apply(op, element, scalar))
                    .collect(),
            )
        }
        ("mult", Operand::Matrix(other)) => {
            if matrix.columns() != other.rows() {
                return Err(dimensions(
                    name,
                    &format!(
                        "the first matrix has {} columns and the second {} rows",
                        matrix.columns(),
                        other.rows()
                    ),
                    range,
                ));
            }
            let mut elements = Vec::with_capacity(matrix.rows() * other.columns());
            for row in 0..matrix.rows() {
                for column in 0..other.columns() {
                    elements.push(dot(
                        (0..matrix.columns()).map(|inner| matrix.get(row, inner)),
                        (0..matrix.columns()).map(|inner| other.get(inner, column)),
                    ));
                }
            }
            Matrix::from_elements(matrix.rows(), other.columns(), elements)
        }
        ("mult", Operand::Vector(vector)) => {
            if matrix.columns() != vector.len() {
                return Err(dimensions(
                    name,
                    &format!(
                        "the matrix has {} columns and the array {} elements",
                        matrix.columns(),
                        vector.len()
                    ),
                    range,
                ));
            }
            let product = (0..matrix.rows())
                .map(|row| {
                    dot(
                        (0..matrix.columns()).map(|inner| matrix.get(row, inner)),
                        vector.iter(),
                    )
                })
                .collect();
            return Ok(new_array(state, product));
        }
        ("kron", Operand::Matrix(other)) => matrix_of(&dense(matrix).kronecker(&dense(other))),
        ("concat", Operand::Matrix(other)) => {
            if matrix.is_empty() {
                *matrix = other.clone();
            } else if matrix.columns() == other.columns() {
                matrix.append_rows(other);
            } else {
                return Err(dimensions(
                    name,
                    &format!(
                        "the matrices have {} and {} columns",
                        matrix.columns(),
                        other.columns()
                    ),
                    range,
                ));
            }
            return Ok(RuntimeValue::Ref(RefKind::Matrix, call.id));
        }
        (member, _) => {
            let wanted = match member {
                "sum" | "diff" => "a matrix or a number",
                "mult" => "a matrix, a number or an array",
                _ => "a matrix",
            };
            return Err(dimensions(
                name,
                &format!("the second argument must be {wanted}"),
                range,
            ));
        }
    };
    Ok(new_matrix(state, result))
}

fn arithmetic_operator(member: &str) -> BinaryOperator {
    match member {
        "sum" => BinaryOperator::Add,
        "diff" => BinaryOperator::Subtract,
        _ => BinaryOperator::Multiply,
    }
}

/// The sum of the pairwise products, with the operators' own int/float and `na` rules; 0 for none.
fn dot<'a>(
    left: impl Iterator<Item = &'a RuntimeValue>,
    right: impl Iterator<Item = &'a RuntimeValue>,
) -> RuntimeValue {
    left.zip(right)
        .map(|(a, b)| operators::apply(BinaryOperator::Multiply, a, b))
        .reduce(|sum, product| operators::apply(BinaryOperator::Add, &sum, &product))
        .unwrap_or(RuntimeValue::Int(0))
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
        call(name, &bound, state, 0.01, SourceRange::ZERO)
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

    /// The matrix as rows of ints/floats/na, for comparisons.
    fn rows_of(state: &RuntimeState, matrix: &RuntimeValue) -> Vec<Vec<RuntimeValue>> {
        let RuntimeValue::Ref(RefKind::Matrix, id) = matrix else {
            panic!("not a matrix: {matrix:?}");
        };
        let m = &state.matrices[id];
        (0..m.rows()).map(|row| m.row(row)).collect()
    }

    fn ints(values: &[i64]) -> Vec<RuntimeValue> {
        values.iter().map(|v| Int(*v)).collect()
    }

    fn matrix_from(state: &mut RuntimeState, rows: &[&[i64]]) -> RuntimeValue {
        let m = ok(
            state,
            "matrix.new",
            &[Int(rows.len() as i64), Int(rows[0].len() as i64), Int(0)],
        );
        for (r, row) in rows.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                ok(
                    state,
                    "matrix.set",
                    &[m.clone(), Int(r as i64), Int(c as i64), Int(*value)],
                );
            }
        }
        m
    }

    #[test]
    fn sum_and_diff_work_cell_by_cell_with_a_matrix_or_a_number() {
        let mut state = RuntimeState::default();
        let a = matrix_from(&mut state, &[&[1, 2], &[3, 4]]);
        let b = matrix_from(&mut state, &[&[10, 20], &[30, 40]]);
        let sum = ok(&mut state, "matrix.sum", &[a.clone(), b.clone()]);
        assert_eq!(rows_of(&state, &sum), [ints(&[11, 22]), ints(&[33, 44])]);
        let diff = ok(&mut state, "matrix.diff", &[b.clone(), a.clone()]);
        assert_eq!(rows_of(&state, &diff), [ints(&[9, 18]), ints(&[27, 36])]);
        let shifted = ok(&mut state, "matrix.sum", &[a.clone(), Float(0.5)]);
        assert_eq!(
            rows_of(&state, &shifted),
            [[Float(1.5), Float(2.5)], [Float(3.5), Float(4.5)]]
        );
        let lowered = ok(&mut state, "matrix.diff", &[a.clone(), Int(1)]);
        assert_eq!(rows_of(&state, &lowered), [ints(&[0, 1]), ints(&[2, 3])]);
        // A matrix may be its own operand, and the originals are untouched.
        let doubled = ok(&mut state, "matrix.sum", &[a.clone(), a.clone()]);
        assert_eq!(rows_of(&state, &doubled), [ints(&[2, 4]), ints(&[6, 8])]);
        assert_eq!(rows_of(&state, &a), [ints(&[1, 2]), ints(&[3, 4])]);
        let wide = matrix_from(&mut state, &[&[1, 2, 3]]);
        let error = run(&mut state, "matrix.sum", &[a, wide]).unwrap_err();
        assert_eq!(error.code, "PINE4037");
        assert!(error.message.contains("2×2 and 1×3"), "{}", error.message);
    }

    #[test]
    fn mult_takes_a_matrix_a_number_or_an_array() {
        let mut state = RuntimeState::default();
        let a = matrix_from(&mut state, &[&[1, 2, 3], &[4, 5, 6]]);
        let b = matrix_from(&mut state, &[&[7, 8], &[9, 10], &[11, 12]]);
        let product = ok(&mut state, "matrix.mult", &[a.clone(), b.clone()]);
        assert_eq!(
            rows_of(&state, &product),
            [ints(&[58, 64]), ints(&[139, 154])]
        );
        let scaled = ok(&mut state, "matrix.mult", &[a.clone(), Int(2)]);
        assert_eq!(
            rows_of(&state, &scaled),
            [ints(&[2, 4, 6]), ints(&[8, 10, 12])]
        );
        let vector = new_array(&mut state, ints(&[1, 0, 1]));
        let applied = ok(&mut state, "matrix.mult", &[a.clone(), vector]);
        assert_eq!(items(&state, &applied), ints(&[4, 10]));
        let error = run(&mut state, "matrix.mult", &[a.clone(), a.clone()]).unwrap_err();
        assert_eq!(error.code, "PINE4037");
        let short = new_array(&mut state, ints(&[1, 2]));
        assert_eq!(
            run(&mut state, "matrix.mult", &[a, short])
                .unwrap_err()
                .code,
            "PINE4037"
        );
    }

    #[test]
    fn na_makes_a_product_cell_na_like_the_operators_do() {
        let mut state = RuntimeState::default();
        let a = matrix_from(&mut state, &[&[1, 2]]);
        ok(&mut state, "matrix.set", &[a.clone(), Int(0), Int(1), Na]);
        let b = matrix_from(&mut state, &[&[3], &[4]]);
        let product = ok(&mut state, "matrix.mult", &[a, b]);
        assert_eq!(rows_of(&state, &product), [[Na]]);
    }

    #[test]
    fn pow_multiplies_a_square_matrix_by_itself() {
        let mut state = RuntimeState::default();
        let fibonacci = matrix_from(&mut state, &[&[1, 1], &[1, 0]]);
        let tenth = ok(&mut state, "matrix.pow", &[fibonacci.clone(), Int(10)]);
        assert_eq!(
            rows_of(&state, &tenth),
            [[Float(89.0), Float(55.0)], [Float(55.0), Float(34.0)]]
        );
        let zeroth = ok(&mut state, "matrix.pow", &[fibonacci.clone(), Int(0)]);
        assert_eq!(
            rows_of(&state, &zeroth),
            [[Float(1.0), Float(0.0)], [Float(0.0), Float(1.0)]]
        );
        let wide = matrix_from(&mut state, &[&[1, 2, 3]]);
        assert_eq!(
            run(&mut state, "matrix.pow", &[wide, Int(2)])
                .unwrap_err()
                .code,
            "PINE4037"
        );
        assert_eq!(
            run(&mut state, "matrix.pow", &[fibonacci.clone(), Na])
                .unwrap_err()
                .code,
            "PINE4037"
        );
        assert_eq!(
            run(&mut state, "matrix.pow", &[fibonacci, Int(-1)])
                .unwrap_err()
                .code,
            "PINE4037"
        );
    }

    #[test]
    fn kron_tiles_scaled_copies_as_floats() {
        let mut state = RuntimeState::default();
        let a = matrix_from(&mut state, &[&[1, 2], &[3, 4]]);
        let b = matrix_from(&mut state, &[&[0, 5]]);
        let product = ok(&mut state, "matrix.kron", &[a, b]);
        assert_eq!(
            rows_of(&state, &product),
            [
                [Float(0.0), Float(5.0), Float(0.0), Float(10.0)],
                [Float(0.0), Float(15.0), Float(0.0), Float(20.0)]
            ]
        );
    }

    #[test]
    fn median_and_mode_ignore_na_and_mode_prefers_the_smallest() {
        let mut state = RuntimeState::default();
        let m = matrix_from(&mut state, &[&[3, 1, 2], &[2, 3, 9]]);
        // 1, 2, 2, 3, 3, 9: median 2.5; 2 and 3 both occur twice, the smaller wins.
        assert_eq!(
            ok(&mut state, "matrix.median", std::slice::from_ref(&m)),
            Float(2.5)
        );
        assert_eq!(
            ok(&mut state, "matrix.mode", std::slice::from_ref(&m)),
            Float(2.0)
        );
        ok(&mut state, "matrix.set", &[m.clone(), Int(1), Int(2), Na]);
        assert_eq!(ok(&mut state, "matrix.median", &[m]), Float(2.0));
        let empty = ok(&mut state, "matrix.new", &[Int(0), Int(0)]);
        assert_eq!(ok(&mut state, "matrix.median", &[empty]), Na);
    }

    #[test]
    fn trace_sums_the_main_diagonal() {
        let mut state = RuntimeState::default();
        let m = matrix_from(&mut state, &[&[1, 2, 3], &[4, 5, 6], &[7, 8, 9]]);
        assert_eq!(ok(&mut state, "matrix.trace", &[m]), Float(15.0));
        let wide = matrix_from(&mut state, &[&[1, 2, 3], &[4, 5, 6]]);
        assert_eq!(ok(&mut state, "matrix.trace", &[wide]), Float(6.0));
        let empty = ok(&mut state, "matrix.new", &[Int(0), Int(0)]);
        assert_eq!(ok(&mut state, "matrix.trace", &[empty]), Na);
    }

    #[test]
    fn concat_appends_rows_in_place_and_returns_the_same_matrix() {
        let mut state = RuntimeState::default();
        let a = matrix_from(&mut state, &[&[1, 2]]);
        let b = matrix_from(&mut state, &[&[3, 4], &[5, 6]]);
        let joined = ok(&mut state, "matrix.concat", &[a.clone(), b.clone()]);
        assert_eq!(joined, a, "the receiver is returned");
        assert_eq!(
            rows_of(&state, &a),
            [ints(&[1, 2]), ints(&[3, 4]), ints(&[5, 6])]
        );
        assert_eq!(rows_of(&state, &b).len(), 2, "the argument is untouched");
        // Concatenating a matrix with itself doubles its rows.
        ok(&mut state, "matrix.concat", &[b.clone(), b.clone()]);
        assert_eq!(rows_of(&state, &b).len(), 4);
        let narrow = matrix_from(&mut state, &[&[1]]);
        assert_eq!(
            run(&mut state, "matrix.concat", &[a, narrow])
                .unwrap_err()
                .code,
            "PINE4037"
        );
        let grown = ok(&mut state, "matrix.new", &[Int(0), Int(0)]);
        let rows = matrix_from(&mut state, &[&[7, 8]]);
        ok(&mut state, "matrix.concat", &[grown.clone(), rows]);
        assert_eq!(rows_of(&state, &grown), [ints(&[7, 8])]);
    }

    #[test]
    fn reverse_swaps_and_reshape_move_cells() {
        let mut state = RuntimeState::default();
        let m = matrix_from(&mut state, &[&[1, 2, 3], &[4, 5, 6]]);
        ok(&mut state, "matrix.reverse", std::slice::from_ref(&m));
        assert_eq!(rows_of(&state, &m), [ints(&[6, 5, 4]), ints(&[3, 2, 1])]);
        ok(&mut state, "matrix.swap_rows", &[m.clone(), Int(0), Int(1)]);
        assert_eq!(rows_of(&state, &m), [ints(&[3, 2, 1]), ints(&[6, 5, 4])]);
        ok(
            &mut state,
            "matrix.swap_columns",
            &[m.clone(), Int(0), Int(2)],
        );
        assert_eq!(rows_of(&state, &m), [ints(&[1, 2, 3]), ints(&[4, 5, 6])]);
        ok(
            &mut state,
            "matrix.swap_columns",
            &[m.clone(), Int(1), Int(1)],
        );
        assert_eq!(
            run(&mut state, "matrix.swap_rows", &[m.clone(), Int(0), Int(2)])
                .unwrap_err()
                .code,
            "PINE4010"
        );
        ok(&mut state, "matrix.reshape", &[m.clone(), Int(3), Int(2)]);
        assert_eq!(
            rows_of(&state, &m),
            [ints(&[1, 2]), ints(&[3, 4]), ints(&[5, 6])]
        );
        let error = run(&mut state, "matrix.reshape", &[m, Int(4), Int(2)]).unwrap_err();
        assert_eq!(error.code, "PINE4037");
        assert!(error.message.contains("6 cells"), "{}", error.message);
    }

    #[test]
    fn sort_orders_the_rows_by_a_column() {
        let mut state = RuntimeState::default();
        let m = matrix_from(&mut state, &[&[3, 10], &[1, 30], &[2, 20], &[1, 5]]);
        ok(&mut state, "matrix.sort", std::slice::from_ref(&m));
        assert_eq!(
            rows_of(&state, &m),
            [
                ints(&[1, 30]),
                ints(&[1, 5]),
                ints(&[2, 20]),
                ints(&[3, 10])
            ],
            "ascending by column 0, ties keep their order"
        );
        ok(
            &mut state,
            "matrix.sort",
            &[
                m.clone(),
                Int(1),
                RuntimeValue::String("order.descending".into()),
            ],
        );
        assert_eq!(
            rows_of(&state, &m),
            [
                ints(&[1, 30]),
                ints(&[2, 20]),
                ints(&[3, 10]),
                ints(&[1, 5])
            ]
        );
        assert_eq!(
            run(&mut state, "matrix.sort", &[m, Int(2)])
                .unwrap_err()
                .code,
            "PINE4010"
        );
    }

    #[test]
    fn submatrix_and_ranged_fill_take_half_open_ranges() {
        let mut state = RuntimeState::default();
        let m = matrix_from(&mut state, &[&[1, 2, 3], &[4, 5, 6], &[7, 8, 9]]);
        let corner = ok(
            &mut state,
            "matrix.submatrix",
            &[m.clone(), Int(1), Int(3), Int(0), Int(2)],
        );
        assert_eq!(rows_of(&state, &corner), [ints(&[4, 5]), ints(&[7, 8])]);
        let whole = ok(&mut state, "matrix.submatrix", std::slice::from_ref(&m));
        assert_eq!(rows_of(&state, &whole).len(), 3);
        let tail = ok(&mut state, "matrix.submatrix", &[m.clone(), Int(2)]);
        assert_eq!(rows_of(&state, &tail), [ints(&[7, 8, 9])]);
        let backwards = run(&mut state, "matrix.submatrix", &[m.clone(), Int(2), Int(1)]);
        assert_eq!(backwards.unwrap_err().code, "PINE4037");
        let outside = run(&mut state, "matrix.submatrix", &[m.clone(), Int(0), Int(4)]);
        assert_eq!(outside.unwrap_err().code, "PINE4010");
        ok(
            &mut state,
            "matrix.fill",
            &[m.clone(), Int(0), Int(0), Int(2), Int(1), Int(3)],
        );
        assert_eq!(
            rows_of(&state, &m),
            [ints(&[1, 0, 0]), ints(&[4, 0, 0]), ints(&[7, 8, 9])]
        );
        ok(&mut state, "matrix.fill", &[m.clone(), Int(9)]);
        assert!(rows_of(&state, &m).iter().flatten().all(|v| *v == Int(9)));
    }

    #[test]
    fn every_member_is_handled_and_has_parameters() {
        for member in MEMBERS {
            let name = format!("matrix.{member}");
            assert!(handles(&name), "{name}");
            assert!(!parameters(&name).is_empty(), "{name}");
        }
        assert!(!handles("matrix.nope") && !handles("array.get"));
    }
}
