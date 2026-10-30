# Editor themes

What the Pine editor looks like: the colour of every kind of text and of the surfaces around it.
Built in stages; this file says what each stage does and does not do.

**Status.** Stage 1 (this file's first version) puts every colour the editor paints behind a theme and
ships the Dark and Light themes, which equal what the editor drew before themes existed. Nothing
selects a theme yet, nothing is stored, and there is no Settings page: the editor wears Dark or Light
by the app's appearance, as it always has. Custom themes, fonts, import/export, the Settings page and
iCloud Sync follow in later stages and are not described as working until they are.

## Roles

A theme gives a colour to each `SyntaxRole` (what a run of source is) and each `SurfaceColor` (a
surface that is not source text), both in `model/editor_theme/roles.rs`. A theme is matched
exhaustively over both enums, so a new role does not compile until every theme says what it looks like.

Only distinctions the tooling can make reliably are roles:

| Role | Decided by |
| --- | --- |
| Comment, Directive (`//@version=6`) | the comment scan, which respects strings |
| String, Escape (`\n`, `\"`) | the lexer; an escape is a backslash and the character after it |
| Number, ColorLiteral, Literal (`true`, `false`, `na`) | the lexer |
| Keyword (`var`, `import`, `and`…), ControlKeyword (`if`, `for`, `in`, `to`…) | the lexer, and the name tables for `in`/`to`/`import`… |
| Operator, Punctuation (brackets, commas, dots) | the lexer |
| UserVariable, Parameter, UserFunction, Type (a script's own), Namespace (an import alias) | the symbol table (`symbols::analyze`), by scope |
| BuiltinVariable, BuiltinConstant, BuiltinFunction, Namespace, Type | the symbol catalog, as whole dotted chains |

There are no per-namespace roles (`ta.*` against `math.*`): the compiler gives no reliable reason to
colour them apart, so `ta.sma` and `math.max` share BuiltinFunction.

## How a name gets its role

`script_manager::syntax::classify(source, table)` is the only classification of Pine text. With the
script's `SymbolTable` its answer for a name is final: a name the table resolves is the script's (a
variable, parameter, function or type), a name it does not place is looked up in the catalog. So a
local `hl2` is the script's inside its function and the builtin outside it, and a read of `close`
above a line that declares `close` is the builtin (the same rule `PINE7006` uses). Without a table
(chat code blocks, the change popover, the AI review) the declared names of the lexical pass stand in:
variables and functions, but not parameters. Half-typed code never fails; it resolves less.

Spans carry roles, not colours (`Span::role`), and `layout_job` looks the colour up when the text is
laid out. So a colour change classifies nothing again, and the span cache (`EditorState::spans()`,
keyed on the text) is untouched by a theme change. `layout_job` joins neighbouring runs of equal
colour into one `LayoutSection` (egui shapes each section on its own), which is why giving user names
and punctuation the default colour, as Dark and Light do, costs nothing.

Text typed in a frame is classified once, by the layouter, and handed to the state afterwards
(`Classified`), so the next frame finds the spans and the symbol table cached.

## Appearance

`ui::editor_appearance::Appearance` is a theme resolved into `Color32`s, one array lookup per role.
`current()` is what the editor wears; `chrome()` is for code shown on the app's own background (the
change popover, chat code blocks, the AI review), which stays Dark or Light by the app's appearance
whatever the editor wears, because a theme's colours are chosen for its own background.

The editor reads `current()` once per frame (`editor::show`) and takes every colour of its text,
gutter, current line, selection, caret, brackets, same-name highlights, search matches, indent guides,
diagnostics and suggestion popups from it. The selection and caret reach egui's text field through the
visuals of the editor's own `Ui`, so extra carets (which read the same visuals) agree.

Not themed on purpose: the debugger's, profiler's and change-gutter colours (`theme::editor_diff_*`,
`editor_debug_*`, `profiler_*`) mean something to the app rather than to a colour scheme, and the
jump flash. `theme::syntax` still exists for the few non-editor uses of a code-like colour (the
debugger's value types, empty-state samples).

## Dark and Light are today's colours

Every colour of both is what the editor drew before. `dark_and_light_match_the_editor_before_themes`
rebuilds the old values from the functions the editor used to call (and from the old `theme::syntax`
table, written out in the test) and compares every role and surface in both appearances. Roles that did
not exist before take the colour of what they were part of: user variables, parameters, user functions
and punctuation the default text colour, escapes the string colour, control keywords the keyword colour.
So an upgrade changes nothing a person can see. Only Dracula (a later stage) tells a user's function
from a builtin.

## Performance

Measured in a release build on an Apple Silicon Mac, a synthetic script of repeated blocks (calls,
user functions, strings, comments, control flow;
`cargo test --release --lib -- syntax::tests::timings --ignored --nocapture`):

| Script | `analyze` + `classify` (a keystroke) | `classify` on a cached table | `layout_job` (a frame) |
| --- | --- | --- | --- |
| 82 lines | 0.22 ms | 0.12 ms | 0.01 ms |
| 802 lines | 3.0 ms | 2.1 ms | 0.09 ms |
| 4,802 lines | 20.7 ms | 8.3 ms | 0.96 ms |

Before this change a keystroke classified twice (the layouter, then the next frame's span cache); the
lexical classification alone, which is most of what that did, measures 1.4 ms for the 802-line script
(7.1 ms for the 4,802-line one), so about 2.8 ms for the keystroke. It now analyses and classifies once,
at about the same cost for an ordinary script. The old figure was not measured on the old code: it is
inferred from the lexical pass. A very large script pays more per keystroke than it did; a frame without
typing costs only the layout.

## What is verified

Unit tests: the roles and their ordering, the Dark and Light equality above, every classification case in
the table (including shadowing, parameters, named arguments to the script's functions, escapes, CRLF,
astral characters and half-typed code), that a colour change leaves the spans alone, that the layout
covers the source and never changes it, and, by running the real editor headlessly, that the background,
gutter, current line, error underline, line numbers, selection, caret and role colours are the ones the
appearance names, that switching the app's appearance repaints without touching the text, caret or span
cache, and that typed text is classified once.

Not verified: the editor in a real window (a later stage looks at it with the Settings page), Linux and
Windows.
