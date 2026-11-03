# Pine engine code review

Date: 2026-11-02  
Scope: `src/pine/`, with emphasis on lexing, parsing, AST design, semantic analysis, call
resolution, interpreter structure, runtime state, diagnostics, and automated tests.  
Review type: static review plus the focused Pine unit/integration suite; no implementation changes.

## Executive summary

This is a real compiler front end and interpreter, not a collection of regular expressions or an
ad-hoc evaluator. The source passes through an indentation-aware lexer, a recursive-descent
statement parser and Pratt expression parser, explicit statement/expression AST enums, structural
and type-checking passes, call binding, and a bar-by-bar interpreter. The AST models the language
well enough to support functions, methods, user-defined types, control flow, history expressions,
debugging and profiling. Rust's exhaustive matches also make additions to the node set relatively
safe.

The overall design is stronger than the typical embedded scripting engine. Particularly good parts
are the explicit diagnostics and resource limits, the separation of pure Pine code from UI code,
the working/committed state model for realtime rollback, opt-in debugger/profiler hooks, static call
binding with a dynamic compatibility fallback, and the breadth of end-to-end tests. The focused
suite completed with 805 passing tests and no failures.

There are nevertheless concrete correctness gaps. Numeric literals are converted through `f64`
even when they are integers, and malformed exponents silently become zero. Function return-type
inference is source-order-dependent, while structured variable annotations are deliberately not
checked for compatibility. Multiline strings preserve UTF-16 offsets but produce incorrect line
and column coordinates after the first physical line. These are the most important findings.

The AST is proper as an execution AST, but it is not a full-fidelity syntax tree. Most compound-node
ranges are start anchors rather than complete spans, parameters and several declaration components
have no locations, and statement ends live in a parallel side table. This is workable for execution
but explains why editor tooling must re-lex source and makes future diagnostics and transformations
more fragile than necessary. Several other smells—late mutation of `CompiledProgram`, duplicated
tree walkers and builtin registries, and the very broad `Session` state—are maintainability debt
rather than evidence that the engine is fundamentally unsound.

Severity meanings used below:

- **Medium:** a plausible script can be accepted or interpreted incorrectly, diagnostics can depend
  on irrelevant source ordering, or user-facing source locations can be wrong.
- **Low:** bounded correctness, maintainability, extensibility, or test-coverage risk.

No high-severity issue was found. The runtime is local, resource-bounded, and returns diagnostics
rather than exposing an obvious trust-boundary or memory-safety failure.

## Findings

### Medium — Numeric lexing can silently change valid integers and malformed numbers

`TokenKind::Number` stores every numeric token as an `f64`. `Lexer::lex_number` parses the source
text with `parse::<f64>()` and replaces every parse failure with `0.0`
(`src/pine/lexer.rs:205-235`). The parser later converts an integer-looking token from that `f64`
to `i64` (`src/pine/parser.rs:751-759`).

This has two distinct effects:

- Integer literals above `2^53` lose low bits before they ever reach `Literal::Int`. For example,
  `9007199254740993` cannot be represented by `f64` and is parsed as a different integer even though
  it is well inside `i64`'s range.
- The scanner accepts an exponent marker with no exponent digits. Text such as `1e` or `1e+` reaches
  `parse::<f64>()`, fails, and becomes the float literal `0.0` without a lexical diagnostic.

The `i64` boundary check is also performed in floating-point space. In particular,
`i64::MAX as f64` rounds to `2^63`, so the comparison is not an exact range check. Rust's float-to-int
cast saturates instead of recovering the original source integer.

Recommendation: keep the raw numeric spelling or split integer and float token payloads. Parse an
integer-looking token directly as `i64`; report overflow or malformed syntax explicitly. Require at
least one digit after an exponent marker and parse floats with a fallible path that produces a
diagnostic rather than a value. Add boundary tests around `2^53`, `i64::{MIN,MAX}`, malformed
exponents, multiple dots, and non-finite results.

### Medium — Function return inference depends on declaration order

The checker pre-collects every callable, but initializes every return to `Unknown`, visits the
program once in source order, and records a function's result only after checking that function's
body (`src/pine/type_checker.rs:87-130`, `260-290`). A selected call reads the callee's return value
as it exists at that moment (`src/pine/type_checker.rs:883-893`). There is no second pass or fixed
point over the call graph.

Consequently, a function calling another function declared later gets an unknown result even after
the later function is checked. That unknown propagates through its caller permanently and can
suppress otherwise certain type errors. Reordering the same declarations so the callee appears
first can produce stronger diagnostics. The runtime call target is still resolved, so this is mainly
a static-analysis soundness and consistency problem rather than an execution failure.

Recommendation: separate callable signature collection from body inference and solve return types
over the non-recursive call graph. A topological pass is sufficient because recursion is rejected;
alternatively iterate until no return changes. Then perform expression/assignment diagnostics using
the stable return table. Add paired tests in which the same chain is declared in forward and reverse
order and assert identical inferred types, resolutions and diagnostics.

### Medium — Structured annotations do not enforce assignment compatibility

Declarations annotated as `Object`, `Map`, or `Matrix` take a special path that explicitly does not
judge whether the initializer fits the annotation. The following branch also retains any
`Inferred::Typed` initializer without comparing it with other annotations, including arrays
(`src/pine/type_checker.rs:340-363`). The requested structured annotation is therefore not compared
with the initializer.

This means annotations that should be useful compiler contracts can accept a known incompatible
structured value. Related weak spots include tuple declarations, which infer the expression but
assign `Unknown` to every target without checking tuple shape or element types
(`src/pine/type_checker.rs:254-259`). Dynamic fallback is appropriate where the engine genuinely
does not know a type, but it should not discard a mismatch between two known `Ty` values.

Recommendation: convert both the declared `TypeRef` and inferred initializer to `Ty` and use the
existing `Ty::accepts` relation whenever both are known. Preserve the permissive path only for
`Ty::Any` or inaccessible library types. Represent tuple types, or at minimum validate known tuple
arity and propagate each known element type. Add negative tests for array/map/matrix/user-type
cross-assignments and tuple arity/type mismatches.

### Medium — Multiline strings produce incorrect physical line and column locations

The lexer joins up to nine physical source lines into one `SourceLine` so Pine multiline strings can
be scanned (`src/pine/lexer.rs:44-53`, `461-490`). `SourceLine`, however, carries one line number and
computes every range column as an index into the joined character vector
(`src/pine/lexer.rs:517-555`). Newline characters affect the UTF-16 prefix, so absolute offsets stay
useful, but the line never advances and the column never resets.

Tokens or diagnostics after the first embedded newline are therefore reported on the string's
starting line with an accumulated column. This affects editor underlines, problem navigation,
debugger source maps and any API consumer that reads `SourcePosition.line`/`column`, even when its
`offset` is correct.

Recommendation: scan the original source with a cursor that tracks byte/character position,
physical line, column and UTF-16 offset continuously. If pre-joining remains desirable, keep a
per-character physical-position map for joined text. Add tests for a closing quote and subsequent
token on the second and later physical lines, including astral characters before and inside the
string.

### Low — The AST has start anchors, not reliable full spans

The AST is structurally sound, but its `SourceRange` fields have inconsistent extent. Unary nodes
keep only the operator token range; calls retain the callee/member range rather than the closing
parenthesis; history, ternary and binary nodes reuse the left/base range
(`src/pine/parser.rs:729-743`, `883-917`, `957-1021`). Statement ranges are intentionally start
anchors, with ends maintained separately in `CompiledProgram::statement_ends`
(`src/pine/parser.rs:76-82`). Parameters, loop variables, tuple targets, type fields and enum members
do not all carry their own ranges (`src/pine/ast.rs:322-425`).

This is sufficient for many runtime errors and profiler identities, but it is not a full source AST.
It forces consumers to understand special range semantics and is the reason editor symbol analysis
has to re-lex source. The side table keyed only by starting UTF-16 offset is also an implicit node
identity separate from the AST.

Recommendation: introduce a uniform `Span` on every syntax node and declaration name, covering the
complete construct while retaining a separate anchor/name span where diagnostics need it. Put the
statement end on the statement node or a stable node id rather than an offset-keyed parallel table.
This need not become a lossless CST unless formatting or source rewriting is planned; complete,
consistent AST spans are enough.

### Low — The AST resource limit counts statements, not AST nodes

`Limits::ast_nodes` is described and diagnosed as an AST node limit, but the parser increments
`statement_count` by the number of statements returned from each line and compares only that count
with the limit (`src/pine/parser.rs:21-37`, `150-175`). Expression nodes, arguments, parameters,
type references, switch arms and nested syntax do not increment it.

The independent token limit still bounds total input, so this is not an unbounded parser issue.
It does mean the configured limit has misleading semantics and cannot constrain a script containing
few statements with very large expression trees.

Recommendation: increment one parser-owned node budget whenever any AST node or auxiliary syntax
node is constructed, or rename the setting and diagnostic to `statements` if that is the intended
guard. Test a deeply nested expression and a wide argument/tuple expression against a small custom
limit.

### Low — `CompiledProgram` is not final until a session mutates it

`compiler::compile` returns `CompiledProgram` with call resolutions in a side map and `route` and
`target` fields still unset on call nodes (`src/pine/compiler.rs:112-139`). `Session::new` takes
ownership, makes the program mutable and runs `call_binding::bind`
(`src/pine/session.rs:239-252`). Binding moves the resolution map out of the program and decorates
the AST in place (`src/pine/call_binding.rs:140-153`). A `CompiledProgram` therefore has different
observable contents before and after session construction, despite its name, and session creation
performs part of compilation.

The split was introduced for a worthwhile runtime optimization and is well tested, but the phase
boundary is surprising. It also keeps both a resolution side table and target fields as transient
representations of the same fact.

Recommendation: make binding the final compiler pass and return an immutable executable program.
If debugger expressions need an unbound form, give that form a distinct type or constructor. Keep
source AST and executable annotations separate if consumers need the pristine tree, for example
`ParsedProgram` followed by `ExecutableProgram`.

### Low — Tree traversal is duplicated and not semantically uniform

The shared traversal module builds temporary `Vec`s for children and blocks and implements several
recursive visitors independently (`src/pine/traversal.rs:6-215`). The type checker duplicates its
own `assigned_names` and `nested_blocks` implementations instead of using the shared versions
(`src/pine/type_checker.rs:1318-1345`). Binding has another complete mutable walk
(`src/pine/call_binding.rs:196-353`). Rust will force each exhaustive `match` to compile after a new
enum variant, but it will not ensure that all passes assign the same semantic meaning to that node.

One visible inconsistency is that `for_each_call` reports `Expression::Call` but not
`Expression::MethodCall`; it merely descends into a method's receiver and arguments
(`src/pine/traversal.rs:139-157`). Plain-path method syntax is encoded as `Call`, while a complex
receiver is `MethodCall`, so generic “every call” validators can behave differently based only on
receiver syntax.

Recommendation: provide allocation-free `walk_expression`/`walk_statement` visitor primitives,
with explicit callbacks for both call forms and one canonical definition of child and block edges.
Build specialized searches on those primitives. Add traversal completeness tests using one fixture
that contains every statement/expression variant and both plain and complex method receivers.

### Low — Builtin registration and dispatch have several manually synchronized sources of truth

Builtin names live in per-family lists, the central registry, call routing tables, type signatures,
editor catalogs and the runtime dispatch chain. `call_binding::route_of` explicitly mirrors runtime
chain order and must be kept synchronized (`src/pine/call_binding.rs:1-15`, `75-127`). This is a
classic parallel-table smell: adding a builtin or family requires coordinated edits that the type
system cannot enforce.

The project mitigates this unusually well. Registry subset/superset guards, bound-versus-unbound
execution tests and dual corpus execution turn many synchronization mistakes into test failures.
That makes the current design defensible for performance, but the maintenance cost remains and
correctness depends heavily on running the right test configurations.

Recommendation: move name, signature/type behavior and route into declarative descriptors owned by
each builtin family, then derive the registry and routing table. Keep runtime handlers specialized;
the goal is a single registration fact, not one generic slow dispatch path. Retain the current dual
execution tests as a backstop.

### Low — `Session` is a broad mutable subsystem boundary

`Session` has dozens of fields, most publicly mutable, spanning program metadata, functions,
methods, imports, market facts, security data, rollback state, logs, alerts, footprints, broker
state, drawing limits, cancellation and debugger/profiler hooks (`src/pine/session.rs:71-141`). Its
implementation is more than 3,600 lines even after builtin families were extracted. `RuntimeState`
similarly aggregates variables, histories, call inputs, TA state, reference heaps, visuals, alerts
and the strategy broker (`src/pine/state.rs:73-120`).

Some aggregation is inherent: realtime rollback requires all transactional script state to move
together, and the comments show that performance has been measured carefully. The smell is not the
existence of a single execution owner; it is the large public mutation surface and the number of
unrelated responsibilities directly represented on that owner. This makes invariants such as
“outside rollback state,” “per bar,” “per call site,” and “per session” easier to violate during
future work.

Recommendation: keep one `Session` owner but group fields into private components such as
`Executable`, `ExecutionState`, `MarketContext`, `ExternalServices`, and `Instrumentation`.
Expose narrow methods rather than public maps. Preserve `RuntimeState` as one rollback unit, but
consider substructures within it so cloning and ownership policy remain explicit.

## What is done well

- The pipeline is easy to identify: normalization, lexing, parsing, import linking, constant/input
  extraction, structural checks, type checks, binding and execution (`src/pine/compiler.rs:45-139`).
- The statement AST and expression AST are explicit algebraic data types rather than stringly typed
  syntax nodes. Pratt binding powers keep precedence and associativity local to the operators.
- Diagnostics have stable codes, severity, category and source ranges. Lexical, syntax, semantic,
  unsupported, resource, runtime and cancellation failures are distinguished.
- Source, token, instruction, call-depth and deadline limits are first-class. Runtime evaluation is
  cooperatively cancellable.
- Realtime behavior has a principled transaction model: a working state can be discarded on an
  unconfirmed tick and committed on close. Reference containers, TA state and broker state live in
  that rollback unit, while logs and derived footprint data are deliberately outside it.
- Static call resolution is an optimization rather than a semantic fork. The by-name chain remains
  as a fallback, and tests compare routed/targeted execution with it.
- Debugging and profiling reuse the interpreter and are gated off the hot path when unattached.
- Unsupported Pine behavior is generally diagnosed or warned about instead of silently ignored.
- The tests cover parsing, execution, rollback, methods/overloads, calls, debugger/profiler behavior,
  resource limits and builtin registry consistency. The optional community corpus and benchmark
  tooling are appropriate for compatibility and performance work.

## Validation performed and limits

Ran:

```text
cargo test pine:: --lib
```

Result: 805 passed, 0 failed, 2 ignored, 4,430 filtered out, in 10.39 seconds. The ignored tests were
the explicit benchmark and stress/measurement cases.

Not run:

- `cargo test --features corpus`; the roughly 295-script community corpus is intentionally excluded
  from the default suite.
- `tools/pine-bench.sh`; this review made no hot-path implementation change.
- Full-repository tests, clippy or formatting checks; the only change is this Markdown report.
- Differential tests against TradingView or a real TradingView session. Several behaviors are
  documented in the repository as inferred rather than verified.

The findings above come from current repository behavior and code structure. Recommendations are
not implementation changes and should be split into focused work, with numeric/source-position
correctness and type-inference consistency addressed before the broader architectural cleanup.
