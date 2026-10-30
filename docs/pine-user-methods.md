# User-defined functions and methods

How the Pine engine declares, type-checks, resolves and runs the script's own functions and
methods. A method is an ordinary function whose first parameter is the receiver of
`receiver.name(…)`; it shares the declaration node, the callable table, the call machinery and the
per-call-site history of a function. There is no second interpreter.

Read this before changing `declaration_rules.rs`, `callables.rs`, `types.rs`, the checker's call
handling in `type_checker.rs`, `user_methods.rs`, or `Session::invoke_with`.

## Pipeline

1. **Parser** (`parser.rs`). `method name(…) =>` is `Statement::Function { kind: FunctionKind::Method }`.
   A `Parameter` keeps its `qualifier` and a `TypeRef` (`array<float>`, `map<K, V>`, `lib.Zone`,
   `chart.point`, `float[]`), and a `TypeField` keeps its `TypeRef`. The lexer joins `a.b.c` into one
   dotted name, so `levels.push(x)` is `Call { name: "levels.push" }`; only a receiver that is not a
   plain path is `Expression::MethodCall`.
2. **Declaration rules** (`declaration_rules.rs`, no types needed):

   | Code | Rule |
   | --- | --- |
   | `PINE3060` | a method has a receiver, and its first parameter declares a type |
   | `PINE3061` | functions and methods are declared in the global scope |
   | `PINE3062` | every parameter of an exported function or method declares a type |
   | `PINE3024` | two declarations of a name need different *required* parameters: a different count, or a parameter whose type or qualifier differs. Optional parameters and parameter names do not count |
3. **Type checker** (`type_checker.rs`) with `types.rs` and `callables.rs`:
   - `Ty` is the structural type (`array<float>`, `map<K, V>`, a user type, an enum, `Any`).
     `Inferred::Typed` carries one by interned handle (`TyId`), so `Inferred` stays `Copy`; its `kind`
     is what the older checks see (None for user types, enums, maps, matrices, which they never judged).
   - `Callables` lists the top-level functions and methods; a callable's id is its position among them.
     `resolve` ranks the candidates for a call: exact type fit, then widened (`int` for `float`), then
     unknown; a lower qualifier beats a higher one. One best fit is `Selected`. A tie that rests on
     something unknown is `Dynamic` (left to the run); a tie of exact fits is `Ambiguous`.
   - `Ty::Any` fits everything. Nothing is reported unless the checker is sure.
   - Typed sources: literals, builtin series, parameters, user-type fields (also through a path,
     `this.items`), `Type.new`, `array.new_<T>`/`from`/`copy`/`slice`, element reads (`get`, `first`,
     `last`, `pop`, `shift`, `remove`), `for … in` elements, and the value of each user function's last
     statement.
   - A call through a variable (`x.name(…)`, `x` in scope or a builtin series such as `close`) is a
     method call on `x`'s type; so is a call on a builtin value of a namespace (`color.red.name()`)
     unless the whole name is a registered builtin function (`strategy.closedtrades.exit_price`,
     `timeframe.change(timeframe)` with a parameter named `timeframe`).
   - Result: `CompiledProgram::resolutions` (call-site number to callable id) and the diagnostics below.
4. **Binding** (`call_binding::bind`, when the session is built) writes each resolution on the call
   node as `target`. Only the script's own calls are bound: a library's call sites are numbered apart
   and its bodies stay unbound.
5. **Run** (`user_methods.rs`, `Session::invoke_with`). A call with a `target` runs
   `Session::callables[target]` directly. A dotted name's path is the receiver, read once with
   `resolve_identifier` and passed as the first parameter (`Supplied::Expressions { receiver, .. }`);
   no hidden variables, no name lookup. A call without a target finds its function by name (below).

## Diagnostics

| Code | When |
| --- | --- |
| `PINE3063` | a function or method reaches itself through resolved calls. A plain call to a name declared once counts even when its arguments do not fit |
| `PINE3064` | `x.name(…)` where `x`'s type is known, the script declares no method of that name that takes it, and the type has no builtin method of that name (skipped when the script imports a library) |
| `PINE3065` | every method of that name refuses the receiver's type |
| `PINE3066` | wrong argument count, a name no parameter has, or an argument of the wrong type. Only for methods |
| `PINE3067` | an argument that changes bar to bar for a `simple` or `const` parameter |
| `PINE3068` | two overloads fit equally well and every argument's type is known |

A user method wins over a builtin of the same name only when it takes the call as written
(TradingView's `fill` example); otherwise the builtin runs.

## Calls the checker cannot settle

`call_user_method` / `call_overloaded` still find the function by name, for an untyped receiver
(`tag(x) => x.describe()`), a library's methods, and the debugger's watch expressions. They keep the
candidates whose first parameter accepts the receiver's value and whose other parameters take the
arguments *written* (count and names, so nothing is evaluated to decide), and only when several
remain evaluate the arguments and take the first that fits. A `na` receiver of unknown type fits every
overload, so the first is chosen; a statically typed `float(na)` is resolved at compile time and does
not have this problem. Across 19 corpus scripts that declare methods, 250 of 273 calls to a declared
method name were resolved statically (91.6%; a builtin call that shares a method's name counts as
unresolved, so this is a floor).

## What is verified, and against what

Verified against TradingView's documentation (Methods, User-defined functions and Objects pages):
the `method` keyword and the explicit type of the first parameter; global scope only; overload validity
(required parameters; names and optional parameters do not count); recursion is not allowed; every
written call has its own scope and history, a call in a branch advances its history only when it runs,
a call in a loop is one scope; qualifiers apply to value types only and reference types are series;
exported parameters are typed; objects are assigned by reference and `copy()` is shallow; the `fill`
overload applies only when all its arguments are given. The tests in `user_method_tests.rs` reproduce
the Methods page's `maintainQueue`/`calcBB`, `getType`, `fill`/`featureScale` and the Objects page's
`deepCopy`, adapted to a few bars.

**Not checked against TradingView itself** (no access from here), so ours or inferred:
- every error message and code (TradingView documents none);
- how a tie between overloads is broken: the rank (exact, widened, unknown; lower qualifier first) and
  the ambiguity error are this app's;
- calling a method as a function (`area(rect)`) is accepted because the engine always did; the
  documentation only shows the dot form;
- a method called on `na`;
- `Ty::Any` inside collections: `array.new<T>` for a library type, `map.new<K, V>` and
  `matrix.new<T>` do not carry their element types, so overloads over those collections are not told
  apart at compile time.

## Known limits

- A library's types and methods are invisible to the checker; with any `import` the unknown-method
  error is off, and calls into a library's methods run by name.
- Static resolution depends on what the checker infers. A receiver it cannot type (many `ta.*`, `str.*`
  and `request.*` results, tuple elements) stays dynamic.
- The editor does not offer user methods after a `.` and does not jump from `x.method()` to its
  declaration (`script_manager::symbols` does not resolve members, by design).
- Not opened in a real window: the Script Manager and a chart were not used to apply a method script.

## Performance

`tools/pine-bench.sh` workloads `user_functions*` / `user_methods*` compute the same thing (equal
checksums), so a pair isolates the call syntax. 5000 one-minute bars, medians of 6 interleaved rounds,
before this work and after:

| Workload | Before | After |
| --- | --- | --- |
| functions, primitives | 20.78 ms | 18.53 ms |
| methods, primitives | 27.58 ms | 18.36 ms |
| functions, array receiver | 23.08 ms | 22.08 ms |
| methods, array receiver | 25.77 ms | 22.15 ms |
| functions, object receiver | 13.16 ms | 12.35 ms |
| methods, object receiver | 15.62 ms | 12.27 ms |

A method costs what its function twin does. The other workloads moved within run-to-run noise.
Allocation counts and peak memory were not measured (`PINE_BENCH_FEATURES=alloc-count`). Check a change
to `session.rs`, `user_methods.rs` or `call_binding.rs` with `tools/pine-bench.sh`.

## Tests

`user_method_tests.rs` is the suite: documentation examples, declaration and call diagnostics,
per-call-site history (two sites, a conditional call, a call in a loop, a stateful builtin), reference
mutation, a library's `export method`, bound-versus-by-name agreement (`Session::ignore_routes`
disables both `route` and `target`), and realtime rollback, a strategy condition and an alert
compared with their function twins through `ExecutionController`. The community corpus
(`cargo test --features corpus`, and `PINE_CORPUS_DUAL=1`) is the guard against false errors.
