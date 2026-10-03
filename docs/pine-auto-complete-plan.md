# Task: Implement TradingView-Style Pine Script Autocomplete / IntelliSense

Implement a production-quality **autocomplete / code-completion system for DegenView's Pine Script IDE**, modeled as closely as practical on TradingView's Pine Editor autocomplete behavior while remaining a native macOS editor.

As the user types Pine code, DegenView should intelligently suggest contextually relevant:

- built-in functions
- built-in variables
- built-in constants
- namespaces
- namespace members
- user-declared variables
- user-declared constants
- user-defined functions
- function parameters
- imported library aliases
- imported library exports
- Pine keywords where contextually appropriate
- type names where supported
- named constants such as `color.*`, `plot.*`, `strategy.*`, etc.
- callable signatures
- function parameter information
- documentation/detail text where metadata exists

The experience should feel like a real language-aware IDE, not a text-prefix dropdown.

Examples:

```pine
ta.r
```

should offer things such as:

```text
rma(...)
roc(...)
rsi(...)
```

Typing:

```pine
ta.rsi(
```

should expose the function signature / parameter information.

Typing:

```pine
myA
```

after:

```pine
myAverage = ta.sma(close, 20)
```

should suggest:

```text
myAverage
```

Typing inside a function should suggest that function's parameters and visible locals.

Autocomplete must understand lexical scope and user shadowing.

---

# 1. Inspect the current editor architecture first

Do not begin by adding string-prefix matching directly to `PineTextView`.

DegenView already has substantial editor infrastructure.

Relevant architecture includes:

```text
Pine/
├── Language/
│   ├── Lexer/
│   ├── AST/
│   ├── Parser/
│   ├── Compiler/
│   ├── Analysis/
│   ├── PineBuiltins.swift
│   └── PineSymbolCatalog.swift
│
├── Editor/
│   ├── PineTextView...
│   ├── PineSyntaxClassifier...
│   ├── PineSyntaxHighlighter...
│   ├── PineLexicalSnapshot...
│   ├── PineEditorContext...
│   ├── PineEditorPairing...
│   ├── PineIndentationEngine...
│   ├── PineEditorCommands...
│   ├── PineDelimiterMatcher...
│   ├── PineEditorDecorations...
│   └── PineLayoutManager...
│
└── View/
```

The existing highlighting/editing pipeline is approximately:

```text
Pine source
    ↓
PineLexicalSnapshot
    │
    ├── lexer tokens
    ├── strings/comments
    └── bracket pairs
          │
          ├── PineSyntaxClassifier
          │     ├── PineSymbolCatalog
          │     └── PineHighlightScopes
          │
          └── PineEditorContext
                ↓
        editing assistance
```

The current architecture explicitly performs **one lex per text version**, and that lexical snapshot is shared with editor functionality. Preserve that design.

Do NOT introduce:

```text
AutocompleteLexer
AutocompleteTokenizer
AutocompleteParser
```

that independently re-tokenizes the entire source on every keystroke.

Reuse the existing lexical snapshot and language metadata.

---

# 2. Core design principle

Autocomplete should be:

```text
lexically aware
+
scope aware
+
symbol aware
+
context aware
+
fast enough for every keystroke
```

It should NOT merely do:

```swift
allSymbols.filter {
    $0.hasPrefix(currentWord)
}
```

The candidate set must depend on what the user is currently writing.

Examples:

```pine
ta.
```

should strongly restrict suggestions to members of `ta`.

```pine
color.
```

should suggest color members.

```pine
strategy.
```

should suggest strategy members.

```pine
foo
```

should consider visible user declarations plus appropriate global Pine symbols.

---

# 3. Architectural target

Implement a pure completion engine separated from AppKit presentation.

Conceptually:

```text
PineTextView
    │
    │ source + caret
    ▼
PineLexicalSnapshot
    │
    ▼
PineCompletionContext
    │
    ├── prefix
    ├── replacement range
    ├── namespace/member access
    ├── lexical scope
    ├── enclosing function
    ├── argument position
    └── suppression state
    │
    ▼
PineCompletionEngine
    │
    ├── PineSymbolCatalog
    ├── user symbols
    ├── scope information
    ├── imports
    └── ranking
    │
    ▼
[PineCompletionItem]
    │
    ▼
PineCompletionController
    │
    ▼
native completion popover
```

Keep:

```text
language intelligence
```

separate from:

```text
completion UI
```

The completion engine should be deterministic and unit-testable without constructing an `NSTextView`.

---

# 4. Reuse PineSymbolCatalog

DegenView already has:

```text
PineSymbolCatalog.swift
```

containing built-in:

```text
variables
constants
functions
namespaces
```

composed from existing compiler/runtime builtin tables.

This should be the canonical starting point for built-in completion.

Do NOT maintain another hand-written list like:

```swift
let autocompleteFunctions = [
    "ta.sma",
    "ta.ema",
    "ta.rsi",
    ...
]
```

That would drift from:

```text
compiler
runtime
syntax highlighting
autocomplete
```

Autocomplete and syntax highlighting should agree about built-in symbols.

---

# 5. Enrich symbol metadata where necessary

Autocomplete requires more information than syntax coloring.

Extend shared symbol metadata where appropriate so completion items can know:

```text
name
qualified name
kind
namespace
signature
parameters
return type
documentation/summary
deprecated state
availability
callable/non-callable
```

Do this without coupling editor models to runtime implementation details.

Remember the architecture constraint that `Pine/Model` and `Model/Script` must remain free of compiler/runtime types because the alert agent compiles them.

Place editor-specific metadata in the appropriate Pine language/editor layer.

---

# 6. Completion item model

Introduce a value model conceptually similar to:

```swift
struct PineCompletionItem: Identifiable, Equatable, Sendable {
    let id: ...
    let label: String
    let insertionText: String
    let kind: PineCompletionKind
    let detail: String?
    let documentation: String?
    let replacementRange: NSRange
    let priority: ...
}
```

Adapt this to repository conventions.

Possible completion kinds:

```text
function
variable
constant
parameter
local
namespace
keyword
type
library
libraryMember
```

Do not use display strings to infer semantics later.

---

# 7. Completion context

Create a pure context representation.

Conceptually:

```swift
struct PineCompletionContext {
    let caretOffset: Int
    let replacementRange: NSRange
    let prefix: String

    let memberBase: String?
    let isMemberAccess: Bool

    let enclosingScope: ...
    let argumentContext: ...
    let lexicalState: ...
}
```

Again, adapt to the real architecture.

The completion engine should receive enough context that it does not need to poke at `NSTextView`.

---

# 8. Trigger behavior

Autocomplete should appear automatically as the user types identifiers.

Examples:

```pine
ta
ta.
ta.r
clos
myVar
str.
math.
```

It should also support explicit invocation using the standard macOS completion command:

```text
Control-Space
```

if not conflicting with existing editor shortcuts.

Explicit invocation should be able to show candidates even when the current prefix is empty, where context makes sense.

---

# 9. Do not trigger everywhere

Suppress automatic completion in contexts where code completion is inappropriate.

At minimum:

```text
comments
string literals
```

should not produce ordinary symbol completion.

Examples:

```pine
// ta.rs|
```

must not open the Pine function list.

```pine
label.new(text="ta.rs|")
```

must not offer `ta.rsi`.

Use `PineLexicalSnapshot` to determine these states.

Do not scan raw characters trying to guess whether the caret is inside a string/comment.

---

# 10. Member completion

Member/namespace completion is essential.

Typing:

```pine
ta.
```

should show members belonging to:

```text
ta
```

not every Pine symbol.

Likewise:

```pine
math.
str.
color.
plot.
strategy.
barstate.
syminfo.
timeframe.
```

should show the appropriate members supported by DegenView.

The exact namespaces must come from the actual symbol catalog.

Do not hard-code this list solely for the UI.

---

# 11. Partial member completion

Typing:

```pine
ta.r
```

should narrow:

```text
ta.rma
ta.roc
ta.rsi
...
```

to actual supported members.

The visible label can be:

```text
rsi
```

while detail may show:

```text
ta.rsi(source, length) → series float
```

Do not insert:

```text
ta.ta.rsi
```

when the user already typed `ta.`.

Replacement ranges must be context-correct.

---

# 12. Global built-ins

When typing an unqualified identifier:

```pine
clo
```

suggest appropriate global built-ins such as:

```text
close
```

when they exist in `PineSymbolCatalog`.

Likewise:

```pine
ope
```

could suggest:

```text
open
```

Do not show namespace members as though they were globals unless Pine actually permits them unqualified.

---

# 13. Built-in variables

Support completion for built-in variables such as the ones DegenView actually implements.

Examples may include:

```text
open
high
low
close
volume
time
bar_index
```

plus namespace-qualified built-ins.

Use the canonical catalog.

---

# 14. Built-in functions

Functions should show useful signature information.

Example candidate:

```text
rsi
ta.rsi(source, length) → series float
```

or equivalent native UI.

The completion row should not just say:

```text
rsi
```

if signature metadata is available.

---

# 15. Function insertion

When completing a callable such as:

```pine
ta.rs|
```

to:

```pine
ta.rsi
```

prefer insertion behavior consistent with TradingView/native IDE expectations.

If appropriate:

```pine
ta.rsi(
```

can be inserted for callable functions.

But avoid producing duplicate parentheses.

Example:

```pine
ta.rs|()
```

must not become:

```pine
ta.rsi(()
```

Inspect characters after the replacement range before inserting punctuation.

---

# 16. Parenthesis-aware insertion

Cases to handle:

```pine
ta.rs|
ta.rs|(
ta.rs|()
```

Completion should produce syntactically sensible results in all cases.

Integrate with the existing:

```text
PineEditorPairing
```

behavior rather than fighting it.

Autocomplete insertion should be one coherent editor edit.

---

# 17. User-declared variables

Autocomplete must discover declarations from the user's source.

Example:

```pine
fastLength = input.int(10)
slowLength = input.int(20)

fastEMA = ta.ema(close, fastLength)
slowEMA = ta.ema(close, slowLength)

plot(fast|)
```

should suggest:

```text
fastEMA
fastLength
```

with sensible ranking.

---

# 18. User-defined functions

Example:

```pine
myAverage(source, length) =>
    ta.sma(source, length)

value = myA|
```

should suggest:

```text
myAverage
```

and identify it as a user function.

Its completion detail should ideally show:

```text
myAverage(source, length)
```

---

# 19. Function parameters

Inside:

```pine
myAverage(source, length) =>
    ta.sma(sou|)
```

autocomplete should suggest:

```text
source
```

because it is a visible function parameter.

Parameters should rank highly inside their function scope.

---

# 20. Local variables

Local declarations should participate in autocomplete.

Conceptually:

```pine
foo() =>
    localValue = close
    localV|
```

should suggest:

```text
localValue
```

within that scope.

Outside the function, it must not be suggested if Pine scoping rules make it inaccessible.

---

# 21. Lexical scope

Autocomplete must respect Pine lexical scope.

Candidate collection should conceptually walk:

```text
innermost scope
      ↓
parent scope
      ↓
global user scope
      ↓
imports
      ↓
built-ins
```

with shadowing applied.

Do not offer inaccessible locals from unrelated functions/blocks.

---

# 22. User shadowing

DegenView's highlighter already has:

```text
PineHighlightScopes
```

for user shadowing.

Reuse or generalize this scope analysis.

Example:

```pine
close = 123
plot(clo|)
```

If Pine semantics allow that declaration/shadowing in the relevant context, autocomplete must treat the user declaration as the effective `close`.

Do not independently implement a different shadowing algorithm for autocomplete and highlighting.

Highlighting and completion should agree about symbol resolution.

---

# 23. Build a reusable editor symbol index

If `PineHighlightScopes` does not expose enough information, generalize it into a reusable source-symbol representation rather than adding a parallel autocomplete-only scanner.

Conceptually:

```text
PineSourceSymbolIndex
├── global declarations
├── functions
├── parameters
├── locals
├── imports
├── scope ranges
└── shadow relationships
```

Then:

```text
syntax classifier
autocomplete
future go-to-definition
future hover
future rename
```

can share it.

Do not overbuild a full Language Server Protocol implementation.

---

# 24. Tolerate incomplete code

Autocomplete operates while the user is typing, so the source will frequently be invalid.

Examples:

```pine
ta.rs
foo =
if barstate.
myFunction(
```

Completion must continue working.

Do NOT require:

```text
successful full parse
successful type check
successful compilation
```

before offering completions.

Use tolerant lexical/scope analysis.

AST/compiler information may enhance results when available, but completion cannot depend entirely on valid compilation.

---

# 25. Incremental responsiveness

Autocomplete must feel immediate.

Target normal completion computation in a few milliseconds for ordinary scripts.

Do not:

```text
compile the entire Pine program
execute Pine
rebuild chart history
run the strategy
hit the database
perform network requests
```

on each keystroke.

Completion is an editor operation.

---

# 26. Reuse one lex per version

The architecture already deliberately uses:

```text
PineLexicalSnapshot
```

as the single lexical snapshot for a text version.

Autocomplete must consume it.

Do not call:

```swift
PineLexer(...)
```

again from the completion engine for the same source version unless profiling and architecture prove a specific incremental reason.

---

# 27. Snapshot invalidation

Completion results must correspond to:

```text
source version
+
caret location
```

If the source changes while asynchronous completion work is pending, stale results must not appear.

Use a lightweight generation/version identity if completion work can cross asynchronous boundaries.

---

# 28. Ranking

Implement deterministic ranking.

A useful priority model is:

```text
exact prefix match
↓
visible local/parameter
↓
visible user declaration
↓
relevant namespace member
↓
built-in symbol
↓
keyword/contextual item
```

But namespace context should dominate when explicitly requested.

For:

```pine
ta.r
```

do not inject unrelated local variables between `ta.rsi` and `ta.roc`.

---

# 29. Prefix matching

At minimum support:

```text
case-appropriate prefix matching
```

consistent with Pine's identifier rules.

Do not add fuzzy matching so aggressive that:

```text
rsi
```

matches unrelated symbols.

If fuzzy matching is implemented, keep it conservative and rank exact-prefix matches first.

TradingView-like predictability matters more than clever fuzzy search.

---

# 30. Recently used candidates

Do not introduce a persistent machine-learning/history ranking system.

If simple session-local recency can improve ordering without making results unpredictable, it may be added later.

Initial ranking should be deterministic from source/context.

---

# 31. Deduplication

Do not show duplicate logical candidates.

If a user declaration shadows a builtin:

```text
foo
```

show the effective visible declaration according to Pine semantics.

Do not show:

```text
foo   local
foo   builtin
```

unless there is a legitimate language reason both are addressable.

---

# 32. Keywords

Suggest Pine keywords only when contextually useful.

Examples may include:

```text
if
else
for
while
switch
import
export
method
type
```

according to the Pine grammar actually implemented by DegenView.

Do not dump every keyword after every character.

---

# 33. Declaration context

When the caret is in a declaration position, prioritize candidates appropriate to that grammar context.

Example:

```pine
myValue = inp|
```

may suggest:

```text
input.*
```

where supported.

After:

```pine
import |
```

completion should prefer import/library-related candidates rather than OHLC variables.

---

# 34. Imports

DegenView already supports:

```pine
import user/Library/version
```

through `PineLibraryRegistry`, `PineLibraryResolver`, and `PineLibraryLinker`.

Autocomplete must integrate with imports.

Do not implement a second library registry.

---

# 35. Imported alias completion

For:

```pine
import user/MyLibrary/1 as lib

lib.
```

suggest exported symbols from that library.

Do not show private/internal library declarations that Pine import semantics do not expose.

Use compiler/linker/library metadata where possible.

---

# 36. Imported function signatures

If an imported library exports:

```pine
export myEMA(float source, int length) =>
```

then:

```pine
lib.my|
```

should offer something like:

```text
myEMA(source, length)
```

with library/member identity.

---

# 37. Import-path completion

If feasible using the existing local library registry, support completion while writing imports.

Example:

```pine
import user/|
```

could suggest local Script Manager libraries available through `PineLibraryRegistry`.

Do not query disk repeatedly on every keystroke.

Use the registry's in-memory state.

This is secondary to symbol completion but should be architecturally supported.

---

# 38. Namespace discovery

Typing:

```pine
ta
```

may suggest:

```text
ta
```

as a namespace.

Accepting it may insert:

```pine
ta.
```

if that matches the chosen UX.

Once the dot exists, immediately refresh completion with namespace members.

Likewise for:

```text
math
str
strategy
color
```

where actually supported.

---

# 39. Dot trigger

Typing:

```text
.
```

after a known namespace or imported alias should immediately trigger member completion.

Example:

```pine
ta.|
```

should open the member list without requiring another character.

---

# 40. Unknown member base

For:

```pine
somethingUnknown.
```

do not show every global completion as though it were a member.

Either:

```text
show no completion
```

or show only candidates supported by actual semantic information.

Avoid misleading suggestions.

---

# 41. Named constants

Namespaces containing constants should autocomplete correctly.

Example:

```pine
color.r|
```

could suggest supported color constants such as:

```text
red
```

if present in the canonical catalog.

Likewise for enums/named constants represented by DegenView's builtin tables.

---

# 42. Type-sensitive completion

If DegenView's existing type checker can cheaply provide reliable context, use it to improve suggestions.

But do not make autocomplete depend on successful type checking.

Type-aware ranking is an enhancement layer.

Architecture:

```text
lexical/scope candidates
        +
optional semantic/type information
        ↓
ranked completions
```

not:

```text
compile succeeds?
    │
    ├── yes → completion
    └── no  → nothing
```

---

# 43. Function argument assistance

When the caret is inside a function call, detect:

```text
which function is being called
current argument index
```

where possible.

Example:

```pine
ta.sma(close, |
```

should know the user is entering the second argument.

This information should power parameter/signature help.

---

# 44. Signature help

Implement a compact TradingView-like signature-help UI for callable symbols.

Example:

```text
ta.sma(source, length) → series float
               ^^^^^^
```

or native equivalent.

When the caret moves between arguments, highlight/emphasize the current parameter.

Do not put signature help inside the completion candidate array as fake completion items.

It is related but separate UI state.

---

# 45. Nested calls

Signature detection must handle:

```pine
ta.ema(
    ta.sma(close, 20),
    |
)
```

and understand the active call/argument using existing bracket-pair information.

Do not count commas by naïvely scanning the raw line.

Use `PineLexicalSnapshot` bracket/token information.

---

# 46. Strings/comments in arguments

A comma inside:

```pine
foo("a,b", |
```

must not be interpreted as an extra argument.

Likewise comments must not confuse argument counting.

Use lexical tokens.

---

# 47. Overloads

If DegenView supports multiple signatures/overloads for a Pine builtin, completion/signature metadata must represent them.

Signature help should allow the best matching overload or expose multiple signatures in a compact way.

Do not flatten incompatible overloads into one fake signature.

---

# 48. Documentation

Where builtin metadata has documentation or concise descriptions, show it in a secondary area.

Example:

```text
ta.rsi(source, length) → series float

Relative Strength Index.
```

Keep documentation concise.

Do not embed large Pine documentation pages into the popup.

---

# 49. Documentation source

Do not maintain a huge duplicate documentation database solely for autocomplete.

Reuse existing symbol metadata where available.

If documentation must be added, attach it to canonical Pine symbol metadata so future:

```text
hover
autocomplete
function help
```

can share it.

---

# 50. Completion popup

Implement a native macOS completion popup anchored to the caret.

It should visually resemble a modern IDE/TradingView completion list while fitting DegenView's UI.

Requirements:

```text
appears near caret
does not move editor text
stays within screen/window bounds
supports dark/light appearance
compact rows
selected row
kind icon
primary label
secondary signature/detail
```

Do not use a SwiftUI `.sheet`.

Autocomplete is transient editor UI.

---

# 51. AppKit integration

Because the Pine editor is based on:

```text
PineTextView
```

prefer AppKit-native positioning/interaction where it produces more reliable caret behavior.

A lightweight:

```text
NSPanel
NSPopover
custom child window
```

or equivalent may be appropriate.

Inspect current editor architecture first.

Do not create an entire second editor view layered over `NSTextView`.

---

# 52. Popup anchoring

Use the text system/layout manager to obtain the caret's screen/window rectangle.

The popup should follow the insertion point as:

```text
user types
editor scrolls
caret moves
window moves/resizes
```

If insufficient room exists below the caret, open above it.

---

# 53. Keyboard navigation

While completion is open:

```text
↓
```

selects the next candidate.

```text
↑
```

selects the previous candidate.

```text
Return
```

accepts the selected completion.

```text
Tab
```

accepts the selected completion where consistent with editor conventions.

```text
Escape
```

dismisses completion.

Typing normal characters continues editing and refreshes candidates.

---

# 54. Editor command precedence

Completion keyboard handling must take precedence only while the popup is active.

For example:

```text
Tab
```

normally participates in editor indentation.

When completion is visible with a selected candidate:

```text
Tab
```

may accept completion.

When completion is not visible:

```text
Tab
```

must retain existing indentation behavior.

Likewise:

```text
Return
```

must not invoke normal newline indentation if it is being used to accept completion.

---

# 55. Undo behavior

Accepting one completion must be one native undo step.

Example:

```pine
ta.rs
```

accepting RSI:

```pine
ta.rsi(
```

then:

```text
Cmd-Z
```

should return to:

```pine
ta.rs
```

in one undo operation.

Use the existing:

```text
PineEditorEdit
```

/ editor edit application architecture where appropriate.

Do not manually mutate the text storage in several independent undo operations.

---

# 56. Selection replacement

If text is selected and explicit completion is invoked, define predictable replacement behavior.

For ordinary automatic completion, derive the replacement range from the identifier around/before the caret.

Do not accidentally delete:

```text
namespace prefix
previous expression
function arguments
```

outside the intended token.

---

# 57. Replacement range examples

For:

```pine
ta.rs|
```

replacement should generally target:

```text
rs
```

not:

```text
ta.rs
```

if insertion text is the member name.

For:

```pine
clos|
```

replace:

```text
clos
```

with:

```text
close
```

For:

```pine
myA|
```

replace only the current identifier fragment.

Centralize this logic.

---

# 58. Completion acceptance characters

Typing certain punctuation may accept a selected candidate where appropriate.

For example, if:

```text
ta
```

has namespace `ta` selected and the user types:

```text
.
```

the editor may accept:

```text
ta
```

and then insert:

```text
.
```

before immediately showing member completion.

Only implement commit characters when behavior is deterministic.

Do not create surprising text mutations.

---

# 59. Mouse interaction

Users must be able to click a completion candidate.

A single click may select.

Double click or single-click acceptance can follow the chosen native convention.

Mouse interaction with the popup must not:

```text
move the editor caret unexpectedly
dismiss the editor
trigger chart gestures behind the editor
```

---

# 60. Popup dismissal

Dismiss completion when:

```text
Escape
focus leaves editor
caret moves to incompatible context
user clicks elsewhere
source changes so no candidates remain
editor/workspace closes
```

Do not leave orphan completion panels after switching scripts.

---

# 61. Scrolling

The completion list must support enough candidates without becoming enormous.

Use a bounded popup height and scroll the list.

Ensure keyboard selection automatically scrolls into view.

---

# 62. Candidate icons

Use subtle kind-specific icons.

For example:

```text
ƒ   function
v   variable
C   constant
N   namespace
p   parameter
L   local
```

Prefer appropriate SF Symbols/native iconography.

Do not make icons visually dominant.

---

# 63. Visual differentiation

User-defined symbols and built-ins should be distinguishable through kind/detail text, not loud color coding.

Example:

```text
myAverage       function    user
rsi             function    ta
close           variable    built-in
```

Keep the list compact.

---

# 64. Match highlighting

Highlight the matching prefix inside candidate labels where practical.

For:

```text
ta.r
```

the `r` portion of:

```text
rsi
rma
roc
```

may be emphasized.

Do not perform expensive attributed-string reconstruction across the whole editor.

Only popup rows are affected.

---

# 65. Empty-prefix completion

Explicit completion invoked after:

```pine
ta.|
```

should show all relevant `ta` members.

Explicit completion at:

```pine
value = |
```

may show visible locals, globals, built-ins, and appropriate namespaces.

Automatic completion should be more conservative with an empty prefix.

---

# 66. Automatic trigger threshold

Do not open a massive global completion list after every single character unless it feels appropriate during testing.

A reasonable policy might be:

```text
member access after "." → immediate
explicit invocation → immediate
identifier prefix → after first or second valid identifier character
```

Tune based on actual editor feel.

Keep the threshold centralized.

---

# 67. No artificial delay

Do not add a long debounce such as:

```text
500 ms
```

that makes autocomplete feel sluggish.

If debouncing is needed, keep it very short and only to avoid redundant UI refreshes.

Pure candidate generation should be fast enough for interactive typing.

---

# 68. Source symbol extraction

Create a lightweight, tolerant mechanism for identifying user declarations.

It should recognize the declaration constructs supported by DegenView's current Pine grammar.

At minimum:

```text
variables
functions
function parameters
imports/aliases
```

and other declaration forms already implemented.

Prefer sharing AST/parser traversal helpers when a valid parse exists, with a lexical fallback for incomplete code.

---

# 69. Do not regex the language

Do NOT build user-symbol discovery from a pile of regular expressions such as:

```swift
"([A-Za-z_]\\w*)\\s*="
```

Pine syntax, comments, strings, multiline declarations, function syntax, and scopes make that fragile.

Use lexer/parser/scope infrastructure.

---

# 70. Hybrid valid/incomplete-source strategy

A robust design can use:

```text
valid/mostly valid source
    ↓
parser/AST-derived symbol information

currently incomplete region
    ↓
lexical tolerant augmentation
```

The editor should retain useful completion even while the current line is unfinished.

Do not throw away the last useful semantic index just because the user typed half a function call.

---

# 71. Source-versioned symbol index

Associate the user symbol index with a source version.

Conceptually:

```text
PineEditorAnalysisSnapshot
├── lexicalSnapshot
├── symbolIndex
└── optional semantic data
```

Only recompute what is needed when source changes.

Avoid repeatedly traversing the entire document for every caret movement when the text has not changed.

---

# 72. Caret movement without source mutation

If the user only presses:

```text
←
→
↑
↓
```

the lexical snapshot and symbol index should be reused.

Only the completion context/ranking changes.

---

# 73. Diagnostics coexistence

Autocomplete must coexist with:

```text
syntax highlighting
diagnostic underlines
current-line highlight
indent guides
delimiter matching
```

Do not wipe temporary attributes or text storage attributes when displaying completions.

The popup should not require mutating syntax-highlight attributes.

---

# 74. Auto-pair coexistence

Autocomplete must cooperate with existing:

```text
()
[]
{}
""
```

pairing behavior.

Example:

```pine
ta.rs|
```

accepting a callable should not produce duplicate parentheses if the editor pairing engine also responds.

Define one owner for the insertion transaction.

---

# 75. Indentation coexistence

Accepting a completion must not accidentally invoke:

```text
PineIndentationEngine
```

unless a newline is actually inserted.

Existing Return/Tab behavior must remain intact when the completion popup is closed.

---

# 76. Delimiter matching coexistence

Completion UI must not interfere with:

```text
PineDelimiterMatcher
```

or bracket highlighting.

Moving the caret after accepting a callable should still update delimiter matching normally.

---

# 77. Snippet scope

Do not implement a full generic snippet engine as part of this task.

Simple callable insertion such as:

```pine
ta.rsi(
```

is sufficient.

Do not introduce:

```text
tab stops
multi-cursor placeholders
snippet variables
VS Code snippet grammar
```

unless an existing system already supports them.

---

# 78. Function parameter placeholders

Likewise, do not insert fake text like:

```pine
ta.rsi(source, length)
```

and force the user to delete placeholders unless there is an existing placeholder/snippet mechanism.

Prefer:

```pine
ta.rsi(
```

plus signature help.

---

# 79. User-defined callable insertion

For:

```pine
myA|
```

accepting:

```text
myAverage(source, length)
```

should normally insert:

```pine
myAverage(
```

using the same callable insertion rules as built-ins.

---

# 80. Variables do not receive parentheses

Accepting:

```text
close
myAverageValue
bar_index
```

must insert only the identifier.

Completion item kind determines insertion behavior.

Do not infer callable status from capitalization or naming.

---

# 81. Namespaces

Accepting a namespace may insert:

```text
ta.
```

and immediately reopen completion for its members.

If this interaction proves awkward with native text input, inserting only:

```text
ta
```

and triggering after the user types `.` is acceptable.

Choose one consistent behavior.

---

# 82. Current token handling

Identifier parsing must follow Pine lexer rules.

Do not use generic whitespace splitting.

The engine needs to understand:

```text
ta.rsi
foo_bar
libraryAlias.function
```

according to actual tokenization.

---

# 83. Unicode

Follow whatever identifier Unicode semantics the existing Pine lexer supports.

Do not invent a broader autocomplete identifier grammar than the compiler accepts.

---

# 84. Error recovery

Autocomplete must never crash the editor because:

```text
source is malformed
parser throws
library disappears
metadata is incomplete
caret offset is unusual
UTF-16 range conversion fails
```

Return no candidates or fall back to lexical completion safely.

---

# 85. Range correctness

`NSTextView` and Swift `String` use different indexing models.

Be meticulous with:

```text
UTF-16 NSRange
String.Index
lexer offsets
```

Reuse existing editor range conversion helpers.

Autocomplete bugs that replace the wrong text around Unicode characters are unacceptable.

---

# 86. Explicit completion command

Add a native command for manually requesting completion.

Prefer:

```text
Control-Space
```

if available.

Route it through the editor responder chain.

Do not implement it as a global app shortcut that fires when the Pine editor is not focused.

---

# 87. Escape precedence

While completion is open:

```text
Esc
```

first closes completion.

It should not close the Script Manager or trigger unrelated editor/chart cancellation.

A second Escape can follow existing responder behavior.

---

# 88. Return precedence

While a candidate is selected:

```text
Return
```

accepts it.

If no candidate is selected or completion is closed:

```text
Return
```

continues using the existing newline/indent engine.

---

# 89. Tab precedence

While completion is active:

```text
Tab
```

may accept the selected candidate.

Otherwise Tab retains existing Pine indentation behavior.

Test this explicitly.

---

# 90. Completion after acceptance

If acceptance produces:

```pine
ta.rsi(
```

dismiss the symbol completion popup and show signature help.

If acceptance produces:

```pine
ta.
```

immediately refresh to member completion.

Transitions should feel continuous.

---

# 91. Signature help lifecycle

Signature help should appear when:

```text
caret enters a known function's argument list
completion inserts a callable
user types "(" after a known callable
```

Update it when argument position changes.

Dismiss when:

```text
caret leaves call
call becomes unresolvable
focus leaves editor
```

---

# 92. Completion vs signature popup

Do not stack two large overlapping panels.

If both completion and signature help are relevant, position them coherently.

For example:

```text
signature help
above completion list
```

or a compact signature header attached to the completion UI.

Keep the UI readable.

---

# 93. Builtin metadata coverage

Audit `PineSymbolCatalog`.

Every supported built-in should ideally have enough metadata for completion.

At minimum ensure completion works for the currently implemented families such as:

```text
ta.*
math.*
str.*
color.*
strategy.*
```

and all other actual namespaces in the repository.

Do not limit autocomplete to a hand-picked demo set.

---

# 94. Catalog/compiler consistency

Add tests ensuring important autocomplete symbols are actually recognized by the compiler/runtime.

Autocomplete should not suggest:

```text
functions DegenView cannot compile
```

unless explicitly marked as unsupported/documentation-only, which should generally be avoided.

The IDE should describe the language DegenView actually implements.

---

# 95. Constants consistency

Likewise, constants and namespace members used by completion should come from the same canonical tables already shared by compiler/runtime/highlighting where possible.

The architecture already intentionally centralizes these in:

```text
PineBuiltins.swift
PineSymbolCatalog.swift
```

Preserve that advantage.

---

# 96. TradingView compatibility research

Before finalizing interaction details, inspect the current TradingView Pine Editor autocomplete behavior.

Compare:

```text
automatic trigger timing
namespace completion
completion list appearance
function signatures
parameter hints
user declaration completion
keyboard acceptance
Escape behavior
```

Reproduce the interaction semantics where they make sense in a native macOS editor.

Do not copy TradingView visual styling pixel-for-pixel if it conflicts with DegenView/macOS conventions.

---

# 97. Tests: built-in prefix

Given:

```pine
plot(ta.rs|)
```

verify completion contains:

```text
rsi
```

if `ta.rsi` is supported.

Verify unrelated namespaces are absent.

---

# 98. Tests: namespace members

Given:

```pine
ta.|
```

verify candidates are members of `ta`.

Given:

```pine
math.|
```

verify candidates are members of `math`.

---

# 99. Tests: global variable

Given:

```pine
plot(clo|)
```

verify:

```text
close
```

is suggested.

---

# 100. Tests: user variable

Given:

```pine
myAverage = ta.sma(close, 20)
plot(myA|)
```

verify:

```text
myAverage
```

is suggested.

---

# 101. Tests: function parameter

Given:

```pine
myFunc(source, length) =>
    ta.sma(sou|)
```

verify:

```text
source
```

is suggested and ranks ahead of less relevant globals.

---

# 102. Tests: local scope

Given two functions:

```pine
foo() =>
    fooLocal = close
    fooLo|

bar() =>
    barLocal = close
```

verify:

```text
fooLocal
```

is suggested inside `foo`.

Verify:

```text
barLocal
```

is not.

---

# 103. Tests: shadowing

Create a legal user declaration that shadows a builtin.

Verify completion and syntax classification resolve the same effective symbol.

This test should exercise shared scope analysis rather than two unrelated implementations.

---

# 104. Tests: user function

Given:

```pine
myAverage(source, length) =>
    ta.sma(source, length)

x = myA|
```

verify:

```text
myAverage
```

appears as a callable user function with the correct parameter list.

---

# 105. Tests: imported library

Given a registered library and:

```pine
import user/TestLib/1 as test

x = test.|
```

verify only appropriate exported library members appear.

---

# 106. Tests: comments

Given:

```pine
// ta.rs|
```

verify automatic completion does not appear.

---

# 107. Tests: strings

Given:

```pine
x = "ta.rs|"
```

verify ordinary symbol completion does not appear.

---

# 108. Tests: incomplete code

Given:

```pine
x = ta.rs|
if
foo(
```

or other incomplete surrounding source, verify completion still works where lexical context is recoverable.

No successful compile should be required.

---

# 109. Tests: replacement range

Given:

```pine
ta.rs|
```

accept `rsi`.

Verify result is exactly:

```pine
ta.rsi(
```

or the chosen callable insertion form.

Not:

```pine
ta.ta.rsi(
```

and not:

```pine
ta.rsi((
```

---

# 110. Tests: undo

Accept one completion.

Press undo.

Verify the source returns to its exact pre-completion state in one undo step.

---

# 111. Tests: auto-pair

Exercise completion when:

```text
(
```

would normally trigger pairing.

Verify completion + pairing never creates duplicate delimiters.

---

# 112. Tests: signature help

Given:

```pine
ta.sma(close, |
```

verify signature help identifies:

```text
ta.sma
```

and the active argument:

```text
length
```

---

# 113. Tests: nested signature help

Given:

```pine
ta.ema(ta.sma(close, 20), |
```

verify the active call is:

```text
ta.ema
```

not:

```text
ta.sma
```

---

# 114. Tests: commas inside strings

Given a supported callable containing:

```pine
foo("a,b", |
```

verify the comma inside the string does not increment argument position.

---

# 115. Tests: stale completion

Start completion for source version A.

Change source to version B before results apply.

Verify A's candidate result cannot appear over B.

---

# 116. Tests: caret movement

Generate a lexical/symbol snapshot.

Move the caret without changing source.

Verify the snapshot/index is reused rather than lexing the document again.

Use injectable instrumentation rather than wall-clock timing.

---

# 117. Tests: performance

Add a representative large Pine script.

Measure completion generation for:

```text
global prefix
namespace prefix
local prefix
```

using deterministic benchmarks where the project supports them.

Avoid fragile CI timing assertions.

Instead verify expensive operations such as:

```text
compiler execution
runtime execution
network
database
```

are never invoked by completion.

---

# 118. Tests: Unicode ranges

Include Unicode before the completion location.

Verify replacement ranges remain correct.

Do not assume Swift character count equals `NSRange.length`.

---

# 119. Tests: popup keyboard behavior

UI-level tests should verify:

```text
Down → selection moves
Up → selection moves
Return → accepts
Tab → accepts
Esc → closes
typing → filters
```

and normal editor behavior returns after dismissal.

---

# 120. Tests: focus loss

Open autocomplete.

Switch script/workspace/window.

Verify the popup disappears and does not remain attached to an obsolete text view.

---

# 121. Tests: catalog consistency

Add representative tests proving completion recognizes built-ins from:

```text
PineSymbolCatalog
```

rather than a separate autocomplete list.

If a new builtin is added to the canonical catalog, completion should receive it automatically wherever metadata allows.

---

# 122. Do not implement

Do NOT turn this task into:

- a full Language Server Protocol implementation
- AI code completion
- cloud completion
- network-backed suggestions
- code generation
- whole-line prediction
- GitHub Copilot-style ghost text
- full snippet grammar
- multi-cursor snippets
- automatic source rewriting
- auto-import of arbitrary libraries
- a second Pine lexer
- a second Pine parser
- a second built-in symbol database
- compilation on every keystroke
- Pine execution on every keystroke
- documentation scraping at runtime

This is deterministic Pine language autocomplete.

---

# 123. Future-friendly architecture

Structure the editor analysis so future IDE features can reuse it.

Conceptually:

```text
PineEditorAnalysisSnapshot
│
├── PineLexicalSnapshot
│
├── PineSourceSymbolIndex
│
└── optional semantic information
     │
     ├── autocomplete
     ├── signature help
     ├── hover          [future]
     ├── go to definition [future]
     ├── find references  [future]
     └── rename           [future]
```

Do not implement those future features now.

But avoid designing autocomplete as a dead-end one-off scanner.

---

# 124. Suggested files

After inspecting the actual repository, additions may resemble:

```text
Pine/Editor/
├── PineCompletionItem.swift
├── PineCompletionContext.swift
├── PineCompletionEngine.swift
├── PineCompletionRanking.swift
├── PineSourceSymbolIndex.swift
├── PineSignatureHelp.swift
├── PineCompletionController.swift
└── PineCompletionPanel.swift
```

These names are illustrative.

Do not create excessive one-type-per-file fragmentation if the existing project style groups closely related editor types.

Existing files likely requiring modification include:

```text
PineSymbolCatalog.swift
PineLexicalSnapshot...
PineEditorContext...
PineTextView...
PineTextView+Editing...
PineSyntaxClassifier...
PineHighlightScopes...
```

Inspect first.

---

# 125. Suggested data flow

Target:

```text
User types
    │
    ▼
PineTextView
    │
    ▼
existing source-version update
    │
    ▼
PineLexicalSnapshot
    │
    ├───────────────┐
    ▼               ▼
syntax          source-symbol
classifier          index
                    │
                    ▼
caret ──────→ PineCompletionContext
                    │
                    ▼
             PineCompletionEngine
                    │
        ┌───────────┼────────────┐
        ▼           ▼            ▼
   user symbols  builtins     imports
        │           │            │
        └───────────┼────────────┘
                    ▼
                 ranking
                    │
                    ▼
          [PineCompletionItem]
                    │
                    ▼
          completion controller
                    │
                    ▼
        native caret-anchored popup
```

For member access:

```text
ta.r|
   │
   ▼
memberBase = ta
prefix = r
   │
   ▼
PineSymbolCatalog.members(of: "ta")
   │
   ▼
prefix filter
   │
   ▼
rma
roc
rsi
...
```

For user symbols:

```text
myAverage = ...
...
plot(myA|)
       │
       ▼
scope at caret
       │
       ▼
visible declarations
       │
       ▼
myAverage
```

---

# 126. Implementation order

Implement approximately in this order:

1. Audit `PineSymbolCatalog`.
2. Audit `PineLexicalSnapshot`.
3. Audit `PineHighlightScopes`.
4. Audit `PineEditorContext`.
5. Audit `PineTextView` input/command handling.
6. Define completion item/kind models.
7. Generalize/create source symbol index.
8. Implement scope-aware visible user symbols.
9. Implement completion context extraction.
10. Implement global builtin completion.
11. Implement namespace/member completion.
12. Implement user variable completion.
13. Implement user function completion.
14. Implement parameter/local completion.
15. Implement user shadowing.
16. Implement imported library member completion.
17. Implement deterministic ranking.
18. Implement replacement ranges.
19. Implement callable insertion.
20. Integrate with pairing/undo.
21. Build caret-anchored native popup.
22. Add keyboard navigation.
23. Add mouse selection.
24. Add explicit Control-Space invocation.
25. Add automatic trigger behavior.
26. Implement signature help.
27. Add documentation/detail presentation.
28. Add source-version/stale-result protection.
29. Test malformed/incomplete source.
30. Profile large-script behavior.
31. Add comprehensive unit/editor integration tests.

Do not start with the popup.

Get completion semantics correct in a pure engine first.

---

# 127. Final implementation report

After implementation, report:

## Files

List:

```text
files added
files modified
```

## Existing architecture reused

Explain specifically how autocomplete reuses:

```text
PineLexicalSnapshot
PineEditorContext
PineSymbolCatalog
PineHighlightScopes / generalized symbol index
PineEditorEdit
PineTextView
PineLibraryRegistry
```

## Completion sources

Document support for:

```text
built-in variables
built-in constants
built-in functions
namespaces
namespace members
user variables
user functions
parameters
locals
imports
library exports
keywords
```

## Scope

Explain:

```text
scope discovery
visibility
shadowing
incomplete-code recovery
```

## Ranking

Document the deterministic ranking algorithm.

## UI

Document:

```text
automatic triggers
dot trigger
Control-Space
caret anchoring
keyboard navigation
mouse interaction
dismissal
dark/light appearance
```

## Editing

Explain:

```text
replacement ranges
function insertion
parenthesis handling
pairing integration
undo behavior
Tab/Return precedence
```

## Signature help

Explain:

```text
call detection
argument-index detection
nested calls
parameter highlighting
```

## Performance

Report:

```text
lexes per source version
symbol-index lifecycle
completion latency
large-file behavior
absence of compile/runtime/network/database work
```

## Tests

List tests for all required scenarios.

---

# 128. Final acceptance criteria

The feature is complete when these interactions work naturally:

### Builtin

User types:

```pine
plot(ta.rs
```

A completion popup appears containing:

```text
rsi
```

with function/signature information.

Accepting it produces sensible Pine source such as:

```pine
plot(ta.rsi(
```

without duplicate parentheses.

---

### Namespace

User types:

```pine
ta.
```

The popup immediately contains only appropriate `ta` members.

---

### Built-in variable

User types:

```pine
plot(clo
```

and sees:

```text
close
```

---

### User variable

Given:

```pine
fastEMA = ta.ema(close, 20)
slowEMA = ta.ema(close, 200)

plot(fast
```

the user sees:

```text
fastEMA
```

---

### User function

Given:

```pine
myAverage(source, length) =>
    ta.sma(source, length)

value = myA
```

the user sees:

```text
myAverage(source, length)
```

---

### Function scope

Given:

```pine
myAverage(source, length) =>
    value = ta.sma(sour
```

the user sees:

```text
source
```

with high priority.

---

### Imported library

Given:

```pine
import user/MyLibrary/1 as lib

value = lib.
```

the user sees the library's exported members.

---

### Signature help

After:

```pine
ta.rsi(
```

the user sees a compact signature.

After:

```pine
ta.rsi(close,
```

the second parameter is identified as active.

---

### Invalid source

Given temporarily invalid code:

```pine
x = ta.rs
foo(
if
```

autocomplete still works where context can be recovered.

---

### Comments and strings

No ordinary symbol completion appears while typing:

```pine
// ta.rsi
```

or:

```pine
"ta.rsi"
```

---

### Keyboard

While the popup is open:

```text
↑ / ↓    navigate
Return   accept
Tab      accept
Esc      dismiss
typing   refilter
```

When the popup is closed, all existing Pine editor keyboard behavior remains unchanged.

---

# 129. Required architectural confirmations

At completion, explicitly confirm:

```text
Autocomplete does not maintain a second Pine builtin list.

Autocomplete does not run Pine compilation on every keystroke.

Autocomplete does not run Pine execution.

Autocomplete performs no network requests.

Autocomplete performs no database work.

PineLexicalSnapshot remains the shared lexical source for each text version.

Builtin completion comes from PineSymbolCatalog/shared language metadata.

User declarations are scope-aware.

User shadowing is consistent with syntax highlighting.

Autocomplete works on incomplete source.

Imported-library completion uses the existing library infrastructure.

Completion insertion is one undo operation.

Autocomplete cooperates with auto-pairing.

Autocomplete cooperates with indentation.

Autocomplete cooperates with delimiter matching.

Strings and comments suppress ordinary symbol completion.

Member completion understands namespaces.

User variables, functions, parameters, and locals are suggested according to visibility.

The completion engine is independent of AppKit and unit-testable.

The popup is presentation only; it is not the source of completion semantics.
```

The result should make DegenView's Pine editor feel like a **language-aware Pine IDE comparable to TradingView's editor**, rather than a text editor with a dictionary dropdown.

