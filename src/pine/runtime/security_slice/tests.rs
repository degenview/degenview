use super::*;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::syntax::token::Limits;

/// How many statements each request's slice has, in source order of the requests.
fn slice_sizes(source: &str) -> Vec<usize> {
    let mut program = compiler::compile(source, Limits::default(), &NoLibraries);
    // The session binds calls to their definitions before it builds the slices.
    crate::pine::runtime::call_binding::bind(&mut program);
    let slices = build(&program.statements);
    let mut sizes: Vec<(ExpressionKey, usize)> = slices
        .iter()
        .map(|(key, slice)| (*key, slice.len()))
        .collect();
    sizes.sort();
    sizes.into_iter().map(|(_, size)| size).collect()
}

#[test]
fn a_variable_pulls_in_the_statements_that_compute_it() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlength = 3\nbase = ta.sma(close, length)\nunused = open\ntrend = base - 1\nplot(request.security(syminfo.tickerid, \"D\", trend))",
    );
    // `length`, `base`, `trend`; not `unused`.
    assert_eq!(sizes, [3]);
}

#[test]
fn a_reassignment_and_a_called_function_are_followed() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nfloor_ = 1.0\nlevel = 0.0\nif close > 2\n    level := close\nscaled(x) =>\n    x * floor_\nplot(request.security(syminfo.tickerid, \"D\", scaled(level)))",
    );
    // `floor_` (read by `scaled`), `level`, the `if` that reassigns it.
    assert_eq!(sizes, [3]);
}

#[test]
fn only_code_before_the_request_counts() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvalue = close\nx = request.security(syminfo.tickerid, \"D\", value)\nvalue := value * 2\nplot(x)",
    );
    assert_eq!(sizes, [1], "the reassignment below the request is left out");
}

#[test]
fn a_call_that_may_fill_a_collection_is_a_write() {
    let request = "plot(request.security(syminfo.tickerid, \"D\", n))";
    for filler in [
        "array.push(samples, close)",
        "samples.push(close)",
        "fill(samples)",
    ] {
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nfill(a) =>\n    array.push(a, 1.0)\nvar samples = array.new_float()\n{filler}\nn = array.size(samples)\n{request}"
        ));
        // The declaration, the call, `n`.
        assert_eq!(sizes, [3], "{filler}");
    }
    // Reading it is not a write, and a plain number passed to a function is not a collection.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nid(a) =>\n    a\nlevel = 1.0\nseen = id(level)\nn = level\n{request}"
    ));
    assert_eq!(sizes, [2], "`level` and `n`, not `seen`");
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nsamples = array.new_float()\nfirst = array.get(samples, 0)\nn = array.size(samples)\n{request}"
    ));
    assert_eq!(sizes, [2], "`samples` and `n`; `first` only reads it");
}

#[test]
fn a_function_that_only_reads_its_argument_is_not_a_write() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nid(a) =>\n    a\nchecked(v) =>\n    runtime.error(\"x\")\n    v\nlevel = id(close)\nunused = checked(level)\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [1], "only `level`");
    // One that fills it, directly or through another function, is.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nmakeArray() =>\n    array.new_float()\nadd(a) =>\n    a.push(1.0)\nfill(a) =>\n    add(a)\nsamples = makeArray()\nfill(samples)\nn = array.size(samples)\nplot(request.security(syminfo.tickerid, \"D\", n))",
    );
    assert_eq!(sizes, [3], "`samples`, `fill(samples)` and `n`");
}

#[test]
fn a_name_the_requests_blocks_declare_is_not_the_global() {
    let request = "request.security(syminfo.tickerid, \"D\", level)";
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 10.0\nvar float x = na\nif close > 0\n    level = 20.0\n    x := {request}\nplot(x)"
    ));
    assert!(sizes.is_empty(), "{sizes:?}: the block's own `level`");
    // Declared after the request, in the block, it is not visible yet: the global is.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 10.0\nvar float x = na\nif close > 0\n    x := {request}\n    level = 20.0\nplot(x)"
    ));
    assert_eq!(sizes, [1]);
    // A loop variable is local to its loop, whatever the globals are called.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\ni = 5\nvar float x = na\nfor i = 0 to 2\n    x := request.security(syminfo.tickerid, \"D\", close[i])\nplot(x)",
    );
    assert!(sizes.is_empty(), "{sizes:?}");
    // At the top level nothing is local.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 10.0\nplot({request})"
    ));
    assert_eq!(sizes, [1]);
}

#[test]
fn a_global_that_would_shadow_the_local_cancels_the_slice() {
    // `other` needs the global `level`, which the expression's own `level` must not see.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = 10.0\nother = level * 2\nvar float x = na\nif close > 0\n    level = 20.0\n    x := request.security(syminfo.tickerid, \"D\", level + other)\nplot(x)",
    );
    assert!(sizes.is_empty(), "{sizes:?}");
}

#[test]
fn a_block_local_assignment_is_not_a_write_of_the_global() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = close\nif close > 50\n    level = 5.0\n    level += 1\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [1], "only the global `level`");
    // `:=` on a name the block did not declare is the global's.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = close\nif close > 50\n    level += 1\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [2]);
    // The same for the collection a block makes for itself, and for a loop variable.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nsamples = array.new_float()\nif close > 50\n    samples = array.new_float()\n    samples.push(1.0)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [1]);
}

#[test]
fn a_mutation_through_an_alias_is_a_write_of_the_collection() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nalias = samples\nalias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [3], "`samples`, `alias = samples`, the push");
    // A number copied from another is a value, not an alias.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = close\ncopy = level\ncopy += 1\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [1]);
}

#[test]
fn a_function_that_changes_a_global_is_a_write_of_it() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\naddSample() =>\n    array.push(samples, close)\nrecord() =>\n    addSample()\nrecord()\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [2], "`samples` and `record()`");
    // One that only reads it is not.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\npeek() =>\n    array.size(samples)\nunused = peek()\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [1]);
}

#[test]
fn only_the_overload_called_decides_what_is_written() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nchecked(float v) =>\n    v\nchecked(array<float> a) =>\n    array.push(a, 1.0)\n    1.0\nlevel = close\nunused = checked(level)\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [1]);
}

#[test]
fn a_block_reading_its_own_variable_is_not_a_read_of_the_global() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nbad = close * 3\nlevel = close\nif close > 0\n    bad = close\n    level := bad\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [2], "`level` and the block, not the global `bad`");
    // Read before the block declares it, it is the global's.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nbad = close * 3\nlevel = close\nif close > 0\n    level := bad\n    bad = close\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [3]);
}

#[test]
fn a_local_alias_of_a_collection_writes_the_collection() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nif close > 0\n    alias = samples\n    alias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [2], "`samples` and the block");
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nfill(a) =>\n    alias = a\n    alias.push(close)\nfill(samples)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [2], "`samples` and `fill(samples)`");
}

#[test]
fn the_collection_of_a_builtin_is_found_by_label_or_position() {
    for call in [
        "array.push(value = close, id = samples)",
        "array.push(samples, value = close)",
    ] {
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\n{call}\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))"
        ));
        assert_eq!(sizes, [2], "{call}");
    }
}

#[test]
fn a_call_follows_the_overload_the_type_checker_chose() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nbad = close\nf(float v) =>\n    v * 2\nf(string s) =>\n    str.tostring(bad)\nplot(request.security(syminfo.tickerid, \"D\", f(close)))",
    );
    assert!(
        sizes.is_empty(),
        "{sizes:?}: `bad` belongs to the string one"
    );
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nbad = close\nf(float v) =>\n    v * 2\nf(string s) =>\n    str.tostring(bad)\nplot(request.security(syminfo.tickerid, \"D\", f(\"a\")))",
    );
    assert_eq!(sizes, [1]);
}

#[test]
fn builtins_and_the_requests_own_statement_need_no_slice() {
    assert!(
        slice_sizes(
            "//@version=6\nindicator(\"T\")\nx = request.security(syminfo.tickerid, \"D\", close)\nplot(x)"
        )
        .is_empty()
    );
    assert!(
        slice_sizes(
            "//@version=6\nindicator(\"T\")\nx = close\ny = request.security(syminfo.tickerid, \"D\", y[1])\nplot(y)"
        )
        .is_empty(),
        "a statement is not its own dependency"
    );
}

#[test]
fn a_function_local_is_not_a_read_of_the_global_with_its_name() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nup = close > open\nf() =>\n    up = close < open\n    up ? 1 : 0\nplot(request.security(syminfo.tickerid, \"D\", f()))",
    );
    assert!(sizes.is_empty(), "{sizes:?}");
}

#[test]
fn a_statement_that_makes_a_request_is_left_out() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\ndaily = request.security(syminfo.tickerid, \"D\", close)\nscaled = daily * 2\nplot(request.security(syminfo.tickerid, \"W\", scaled))",
    );
    // `scaled` is computed on the weekly series from the chart's `daily`.
    assert_eq!(sizes, [1]);
}

#[test]
fn the_expression_is_found_by_name_too() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nx = close * 2\nplot(request.security(syminfo.tickerid, \"D\", expression = x))",
    );
    assert_eq!(sizes, [1]);
}

#[test]
fn two_blocks_alias_names_do_not_merge() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nvar other = array.new_float()\nif close > 0\n    alias = samples\n    alias.push(close)\nif close > 0\n    alias = other\n    alias.push(array.get(other, 5))\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [2], "`samples` and the first block only");
}

#[test]
fn a_builtin_is_not_a_script_function_of_the_same_name() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nbad = close\nsma(float src, int len) =>\n    src + bad\nplot(request.security(syminfo.tickerid, \"D\", ta.sma(close, 2)))",
    );
    assert!(sizes.is_empty(), "{sizes:?}");
}

#[test]
fn a_read_only_query_is_not_a_write_of_the_collection() {
    for query in [
        "array.percentrank(samples, 0)",
        "array.stdev(samples)",
        "array.percentile_linear_interpolation(samples, 50)",
    ] {
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\narray.push(samples, close)\nunused = {query}\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))"
        ));
        assert_eq!(sizes, [2], "{query}: the declaration and the push");
    }
}

#[test]
fn a_user_method_has_its_own_effects_not_the_builtin_ones_of_its_name() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nbad = close\nlevel = close\nmethod push(float this, float v) =>\n    v + bad\nr = level.push(1.0)\nplot(request.security(syminfo.tickerid, \"D\", level))",
    );
    assert_eq!(sizes, [1], "`level` alone");
}

#[test]
fn a_matrix_predicate_is_not_a_write_of_the_matrix() {
    for query in [
        "matrix.is_zero(m)",
        "matrix.is_identity(m)",
        "matrix.is_symmetric(m)",
    ] {
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nm = matrix.new<float>(2, 2, 0.0)\nunused = {query}\nplot(request.security(syminfo.tickerid, \"D\", matrix.rows(m)))"
        ));
        assert_eq!(sizes, [1], "{query}: only the declaration");
    }
}

#[test]
fn a_namespaced_builtin_does_not_write_what_the_script_function_of_its_name_writes() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nsma(float src, int len) =>\n    array.push(samples, src)\n    src\nunused = ta.sma(close, 2)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [1]);
    // The script's own function does.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nsma(float src, int len) =>\n    array.push(samples, src)\n    src\nunused = sma(close, 2)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    assert_eq!(sizes, [2]);
}

#[test]
fn a_name_declared_in_a_block_of_a_function_is_not_a_read_of_the_global() {
    // `level` is read after the `if`, whose own `level` is gone: it is the global.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = close\nf() =>\n    if false\n        level = 1.0\n    level\nplot(request.security(syminfo.tickerid, \"D\", f()))",
    );
    assert_eq!(sizes, [1]);
    // Read only inside the block that declares it, it is the function's own.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = close\nf() =>\n    if true\n        level = 1.0\n        level\n    1.0\nplot(request.security(syminfo.tickerid, \"D\", f()))",
    );
    assert!(sizes.is_empty(), "{sizes:?}");
}

#[test]
fn an_alias_reassigned_in_an_inner_block_stands_for_both_sources() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar a = array.new_float()\nvar b = array.new_float()\nif close > 0\n    alias = a\n    if close > 1\n        alias := b\n    alias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(b)))",
    );
    assert_eq!(sizes, [3], "`b`, the block that fills it, and `a` it reads");
    // An `alias` the inner block declares is its own, not the outer one.
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar a = array.new_float()\nvar b = array.new_float()\nif close > 0\n    alias = a\n    if close > 1\n        alias = b\n    alias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(b)))",
    );
    assert_eq!(sizes, [1], "the block fills `a`, not `b`");
}

#[test]
fn a_write_in_the_block_of_the_request_before_it_is_part_of_the_slice() {
    let request = "request.security(syminfo.tickerid, \"D\", level)";
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 0.0\nvar float x = na\nif close > 0\n    level := close\n    x := {request}\nplot(x)"
    ));
    assert_eq!(
        sizes,
        [2],
        "`level` and the reassignment before the request"
    );
    // After the request in the block, it is not what the request sees.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 0.0\nvar float x = na\nif close > 0\n    x := {request}\n    level := close\nplot(x)"
    ));
    assert_eq!(sizes, [1]);
    // Nor is it when the request sits in the other branch.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 0.0\nvar float x = na\nif close > 0\n    level := close\nelse\n    x := {request}\nplot(x)"
    ));
    assert_eq!(sizes, [1]);
}

#[test]
fn a_name_an_expression_statement_declares_is_not_the_global() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nlevel = close\nx = if close > 0\n    level = 20.0\n    request.security(syminfo.tickerid, \"D\", level)\nelse\n    0.0\nplot(x)",
    );
    assert!(sizes.is_empty(), "{sizes:?}: the branch's own `level`");
}

#[test]
fn a_write_the_slice_cannot_replay_cancels_it() {
    let request = "x := request.security(syminfo.tickerid, \"D\", level)";
    // Run for each iteration, or only once, a block is not replayed as a statement of the slice.
    for header in ["for i = 0 to 2", "while close < 0", "once"] {
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nlevel = 0.0\nvar float x = na\n{header}\n    level := close\n    {request}\nplot(x)"
        ));
        assert!(sizes.is_empty(), "{header}: {sizes:?}");
    }
    // One that writes nothing the request reads leaves the slice alone.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 0.0\nvar float x = na\nfor i = 0 to 2\n    {request}\nplot(x)"
    ));
    assert_eq!(sizes, [1]);
    // A local it reads is not defined there.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = 0.0\nvar float x = na\nif close > 0\n    double = close * 2\n    level := double\n    {request}\nplot(x)"
    ));
    assert!(sizes.is_empty(), "{sizes:?}");
}

#[test]
fn a_rebinding_after_the_reader_is_not_part_of_the_slice() {
    let request = "plot(request.security(syminfo.tickerid, \"D\", snapshot))";
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = close\nsnapshot = level\nlevel := level + 1\n{request}"
    ));
    assert_eq!(
        sizes,
        [2],
        "`level` and `snapshot`; not the change after it"
    );
    // A rebinding before the reader is.
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = close\nlevel := level + 1\nsnapshot = level\n{request}"
    ));
    assert_eq!(sizes, [3]);
}

#[test]
fn a_rebinding_after_a_reader_that_sees_the_previous_bar_is_part_of_the_slice() {
    let request = "plot(request.security(syminfo.tickerid, \"D\", snapshot))";
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nvar level = 0.0\nsnapshot = level\nlevel := close\n{request}"
    ));
    assert_eq!(sizes, [3], "`var` keeps what the previous bar wrote");
    let sizes = slice_sizes(&format!(
        "//@version=6\nindicator(\"T\")\nlevel = close\nsnapshot = level[1]\nlevel := level + 1\n{request}"
    ));
    assert_eq!(sizes, [3], "`level[1]` is the previous bar's last value");
}

#[test]
fn a_change_to_a_collection_after_a_reader_is_part_of_the_slice() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nsamples = array.new_float()\nn = array.size(samples)\narray.push(samples, close)\nplot(request.security(syminfo.tickerid, \"D\", n))",
    );
    assert_eq!(sizes, [3], "the collection is the same one");
}

/// A script of `statements` top-level statements, `requests` of them requests, that mixes
/// reassignments in blocks, collections filled by builtins and by script functions.
fn large_script(requests: usize, statements: usize) -> String {
    let mut source = String::from(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nvar other = array.new_float()\nlevel = close\ndouble(x) =>\n    x * 2\nappend(a) =>\n    array.push(a, close)\n    1.0\ntotal() =>\n    array.push(samples, close)\n    level\n",
    );
    let every = (statements / requests).max(1);
    for index in 0..statements {
        match index % 5 {
            0 => source.push_str(&format!("v{index} = close * {index} + level\n")),
            1 => source.push_str(&format!(
                "if close > {index}\n    level := v{}\n    array.push(samples, v{})\n",
                index - 1,
                index - 1
            )),
            2 => source.push_str(&format!("w{index} = double(v{})\n", index - 2)),
            3 => source.push_str("append(other)\n"),
            _ => source.push_str(&format!("x{index} = array.size(samples) + total()\n")),
        }
        if index % every == every - 1 {
            source.push_str(&format!(
                "r{index} = request.security(syminfo.tickerid, \"D\", v{} + array.size(samples))\n",
                index - index % 5
            ));
        }
    }
    source
}

#[test]
#[ignore = "timing: cargo test --release -- --ignored --nocapture"]
fn the_slices_of_a_large_script_build_quickly() {
    let source = large_script(40, 500);
    let mut program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    crate::pine::runtime::call_binding::bind(&mut program);
    let runs = 20;
    let mut fastest = std::time::Duration::MAX;
    let mut total = std::time::Duration::ZERO;
    let mut slices = Slices::new();
    for _ in 0..runs {
        let started = std::time::Instant::now();
        slices = build(&program.statements);
        let elapsed = started.elapsed();
        fastest = fastest.min(elapsed);
        total += elapsed;
    }
    let sliced: usize = slices.values().map(|slice| slice.len()).sum();
    println!(
        "{} statements, {} requests, {} slices holding {} statements: best {:?}, mean {:?}",
        program.statements.len(),
        source.matches("request.security").count(),
        slices.len(),
        sliced,
        fastest,
        total / runs
    );
}

#[test]
fn an_alias_made_in_the_block_of_an_expression_writes_the_collection() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nvar alias = array.new_float()\nflag = if close > 0\n    alias := samples\n    1.0\nelse\n    0.0\nalias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
    );
    // `samples`, `alias`, the `flag` statement that assigns it, the push.
    assert_eq!(sizes, [4]);
}

#[test]
fn globals_copied_from_a_builtin_are_not_aliases() {
    let sizes = slice_sizes(
        "//@version=6\nindicator(\"T\")\na = close\nb = close\na.push(1.0)\nplot(request.security(syminfo.tickerid, \"D\", b))",
    );
    assert_eq!(sizes, [1], "only `b`, not the write to `a`");
}

#[test]
fn a_block_that_redeclares_a_name_does_not_pass_on_what_it_assigned_before() {
    // The runtime puts the outer `c0` back when the inner `if` ends, undoing `c0 := b`
    // (see `security_call`'s test of the same name), so the push below fills `a` only.
    let source = |read: &str| {
        format!(
            "//@version=6\nindicator(\"T\")\nvar a = array.new_float()\nvar b = array.new_float()\nif close > 0\n    c0 = a\n    if close > 1\n        c0 := b\n        c0 = c0\n    c0.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size({read})))"
        )
    };
    assert_eq!(
        slice_sizes(&source("a")),
        [3],
        "`a`, `b` (the block reads it) and the block"
    );
    assert_eq!(slice_sizes(&source("b")), [1], "`b` alone");
}
