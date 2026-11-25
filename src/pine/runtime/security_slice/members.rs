//! What the slicer knows about names: the request functions, the collection members that read or
//! change, and the predicates over them.

use crate::pine::syntax::ast::{Argument, Expression};

/// The request functions whose third argument is the expression.
const SLICED_REQUESTS: [&str; 2] = ["request.security", "request.security_lower_tf"];
/// Their parameters in order, the first three being `symbol`, `timeframe` and `expression`.
const EXPRESSION_POSITION: usize = 2;
/// The namespaces whose functions take the collection as their first argument.
pub(super) const COLLECTION_NAMESPACES: [&str; 3] = ["array", "map", "matrix"];
/// Members that never change the collection they are given. Any other member is taken to, so a
/// function added to the language later errs towards running more, not towards wrong results.
pub(super) const READ_ONLY_MEMBERS: [&str; 62] = [
    "get",
    "size",
    "first",
    "last",
    "includes",
    "indexof",
    "lastindexof",
    "contains",
    "join",
    "copy",
    "slice",
    "sum",
    "avg",
    "min",
    "max",
    "median",
    "keys",
    "values",
    // Array queries and the functions that return a new array.
    "abs",
    "standardize",
    "binary_search",
    "binary_search_leftmost",
    "binary_search_rightmost",
    "sort_indices",
    "covariance",
    "every",
    "some",
    "mode",
    "range",
    "variance",
    "stdev",
    "percentile_nearest_rank",
    "percentile_linear_interpolation",
    "percentrank",
    // Matrix queries and the functions that return a new matrix.
    "col",
    "columns",
    "det",
    "diff",
    "eigenvalues",
    "eigenvectors",
    "elements_count",
    "inv",
    "is_square",
    "is_stochastic",
    "kron",
    "mult",
    "pinv",
    "pow",
    "rank",
    "row",
    "rows",
    "submatrix",
    "trace",
    "transpose",
    "is_identity",
    "is_diagonal",
    "is_antidiagonal",
    "is_symmetric",
    "is_antisymmetric",
    "is_triangular",
    "is_binary",
    "is_zero",
];
/// Members, besides `new…`, that return a collection of their own.
const COLLECTION_RETURNING_MEMBERS: [&str; 5] = ["from", "copy", "slice", "keys", "values"];
/// Methods that change the variable they are called on, whatever it is declared from (a collection
/// a script function returned, say).
pub(super) const MUTATING_METHODS: [&str; 21] = [
    "push",
    "pop",
    "shift",
    "unshift",
    "insert",
    "remove",
    "clear",
    "set",
    "fill",
    "sort",
    "reverse",
    "put",
    "put_all",
    "add_row",
    "add_col",
    "remove_row",
    "remove_col",
    "swap_rows",
    "swap_columns",
    "reshape",
    "concat",
];

/// Whether `name` is a request that evaluates an expression on another series, and so gets a slice.
pub(super) fn is_sliced_request(name: &str) -> bool {
    SLICED_REQUESTS.contains(&name)
}

/// Whether `name` is any `request.*` function. A statement that calls one is never part of a slice:
/// run inside another request it would nest requests.
pub(super) fn makes_request(name: &str) -> bool {
    name.starts_with("request.")
}

/// Whether the callee returns a collection or an object of its own: `array.new_float()`,
/// `Point.new()`, `array.copy(samples)`.
pub(super) fn returns_reference(callee: &str) -> bool {
    let (namespace, member) = callee.rsplit_once('.').unwrap_or(("", callee));
    member.starts_with("new")
        || (COLLECTION_NAMESPACES.contains(&namespace)
            && COLLECTION_RETURNING_MEMBERS.contains(&member))
}

/// The `expression` argument of a request call, found the way the call binds its arguments: by
/// name, or as the third argument that has none.
pub(crate) fn expression_argument<'a>(
    name: &str,
    arguments: &'a [Argument],
) -> Option<&'a Expression> {
    if !is_sliced_request(name) {
        return None;
    }
    let mut position = 0;
    for argument in arguments {
        match argument.name.as_deref() {
            Some("expression") => return Some(&argument.value),
            Some(_) => {}
            None => {
                if position == EXPRESSION_POSITION {
                    return Some(&argument.value);
                }
                position += 1;
            }
        }
    }
    None
}

/// The collection a function of `array.*`, `map.*` or `matrix.*` is given: the argument labelled
/// `id` (`id1` for `array.concat`), else the first one without a label.
pub(super) fn collection_argument(arguments: &[Argument]) -> Option<&Expression> {
    arguments
        .iter()
        .find(|argument| matches!(argument.name.as_deref(), Some("id" | "id1")))
        .or_else(|| arguments.iter().find(|argument| argument.name.is_none()))
        .map(|argument| &argument.value)
}
