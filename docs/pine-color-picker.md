# Pine colour swatches and picker

A small square in front of every constant colour in the Pine editor; a click opens a picker that
rewrites the colour in the source. Code: `pine_studio/editor/color_sites.rs` (detection, values, rewriting;
no egui), `pine_studio/editor/swatches.rs` (swatches, the picker's life, applying edits),
`ui/color_picker/inline.rs` (the picker's widgets, shared painting and HSV maths with
`ui/color_picker.rs`).

**Status.** Detection, rewriting, undo and the picker's interactions are covered by tests that drive
the real editor with synthetic input. It has **not** been compared with TradingView side by side (see
*What is verified*).

## What gets a swatch

`color_sites::map` walks lexer tokens, never the AST (the AST has no span for a call and drops
statements it cannot parse) and never a regex. Comments and strings are not tokens, so nothing in
them matches. A site is the **outermost** expression that evaluates to a constant:

| Form | Value |
| --- | --- |
| `#RRGGBB`, `#RRGGBBAA` | the lexer's value (6 digits are opaque; 8 digits carry **alpha**, not a transparency) |
| `color.red` and the other 16 constants (`builtins::COLOR_NAMES`) | `builtins::color` |
| `color.rgb(r, g, b[, t])` | `builtins::rgb`, so channels truncate and clamp as at run time |
| `color.new(<colour>, t)` | `builtins::with_transparency` |

Numeric arguments are literals, unary minus, `+ - * /` and parentheses. Anything else (a variable, a
call, `na`, a division by zero, more than 24 levels of nesting) makes that expression **unresolved**
and shows no swatch: a colour that depends on data is never drawn as if it were fixed. The scan then
carries on inside it, so `color.new(c, dyn)` still previews a constant `c`, and `cond ? color.green :
color.red` has two swatches. `input.color(`, `x.color` and a `color` type annotation are not colours.
A half-typed expression is simply not a site.

Variables holding a colour (`myColor`, or a constant folded from one) get no swatch; only literal
expressions do.

## Where the swatch goes

The room is a section's `leading_space` in the layout job (`syntax::layout_job_with_gaps`), so the
text, every char index and every diagnostic offset are unchanged, and a line is simply wider by the
swatch (`Metrics::gap`, from the font size) where it has one. The square is placed from the galley's
own glyph positions, in the room just before the colour, and only for rows on screen.

Things that assumed one cell per character were audited: the indent guides sit left of any colour;
the inline debug values and find/word/bracket highlights read galley positions; column selection
(`carets::cell_at`) takes the room out of the column (`swatches::room_before`). A caret placed at the
first character of a colour draws after the room.

The swatch is registered above the text field with `Sense::click_and_drag`, so its press never moves
the caret. The theme preview in Settings draws swatches but opens no picker.

## Rewriting

`color_sites::rewrite(site, text, Picked)` returns the new text of the site only. `Picked` holds the
channels the user moved *in this gesture*; an untouched channel keeps its original spelling, so
opening and closing never drifts (`255 / 2` stays `255 / 2`, `40.5` stays `40.5`).

| Site | Colour changed | Transparency changed |
| --- | --- | --- |
| `#RRGGBB` | `#RRGGBB` (case of the original digits kept) | `#RRGGBBAA`; back to 6 digits at 0 |
| `#RRGGBBAA` | new digits, alpha byte kept | alpha byte from the percent |
| `color.rgb(r, g, b)` | only the arguments whose value changed | `, t` appended (nothing at 0) |
| `color.rgb(r, g, b, t)` | as above | the fourth argument |
| `color.new(base, t)` | recursively the base, in its own style | the `t` argument only |
| `color.red` | the exact colour of another constant is written as it; else `color.rgb(r, g, b)` (inferred) | `color.new(color.red, t)` |

Pine transparency (0 opaque … 100 invisible), the alpha byte and the strip's opacity convert in one
place (`transparency_to_alpha`, `alpha_to_transparency`, `inline::transparency_percent`) with the
rounding of `color.new`; the percent round-trips for every value 0..=100.

## The picker's life

`EditorState::swatches.open` is the open picker: the colour's start (char index; edits are inside the
expression so it does not move), an `Hsv` kept while it is open (a grey has no hue of its own), and the
hash of the text it last wrote. It closes when the hash differs (typing, undo, an outside edit,
`replace_source`), the colour is gone, its swatch scrolls out of view, Esc, or a press outside the
picker and the swatches. A press on another swatch retargets it; a second click on the same one closes
it. Cmd/Ctrl-Z while it is open (and no drag going) closes it and hands the key to the text.

A **gesture** (a drag, a click, an arrow key) begins at the first change. It keeps the text and the
site as they were then; every frame `rewrite` runs on *that* text, so a change in length never leaves a
stale offset. The caret after the colour moves with the text. Esc during a drag restores the original
text, undo history and cursor.

**Undo.** There is no second stack: it is egui's text-field undoer. `UndoGesture::begin` adds the
text as it stands as an undo point; every frame `hold` puts the history back to that, because egui cuts
a new step whenever the text rests for a second, which a slow drag would do mid-gesture. After release
the normal flow records the result. One gesture is one Cmd-Z, redo restores it
(`a_gesture_is_one_undo_step_however_long_it_takes` rests 1.5 s mid-drag).

## What is verified

- By tests: everything above, including 100 open/close cycles without drift, identical colours edited
  one at a time, Unicode and CRLF, an incomplete expression, the picker kept inside the window.
- **Not verified against TradingView:** pixel sizes of the swatch and picker, the picker's placement
  rule, the strips' direction (opaque at the top is inferred), the choice of `color.rgb(r, g, b)` when a
  named colour changes, whether TradingView writes hex or a constructor, and whether the function
  documentation under its picker is part of the picker (it is treated as a separate hover panel and not
  drawn here). The attached reference screenshot was not available to this implementation.
- The picker was exercised headlessly. See the pull request for what was looked at in a running build.

## Performance

The scan runs once per text change (cached by hash like the syntax spans, and done in the layouter for
text typed this frame) and costs one more lexer pass. While a gesture is going the text changes every
frame, which re-classifies the script exactly as typing does; swatches are painted only for rows on
screen.
