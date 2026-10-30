# Editor themes

What the Pine editor looks like: the colour of every kind of text and of the surfaces around it, and
the font the code is set in. Settings ▸ Script Manager chooses and edits them; every window's editor
follows at once.

**Status.** Built-in Dark, Light and Dracula; custom themes (create, duplicate, rename, delete,
edit, import, export); typography (font family, size, line height); a live preview that is the real
editor; saved with the rest of the app's data and synced by iCloud Sync (*Sync*, below).
*What is verified* says what has and has not been looked at.

## Roles

A theme gives a colour to each `SyntaxRole` (what a run of source is) and each `SurfaceColor` (a
surface that is not source text), both in `model/editor_theme/roles.rs`. Every built-in theme matches
both enums exhaustively, so a new role does not compile until each says what it looks like.

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

There are no per-namespace roles (`ta.*` against `math.*`): the compiler gives no reason to colour
them apart, so `ta.sma` and `math.max` share BuiltinFunction. Doc comments do not exist in Pine and
are not a role; a `//@…` directive is.

Surfaces: background, current line, selection and selected text, caret, matching bracket (fill and
border), same-name highlight, search match and the current one, indent guide and the active one,
gutter background and separator, line number and the current one, error and warning underline, the
suggestion list and its selected row. Each is painted by the real editor. Not offered because the
editor has nothing to colour: scrollbar markers, folding, an information underline, an error-line
background. An *inactive* selection is not offered either: egui paints the selection only while the
field has the focus.

Not themed on purpose: the debugger's, profiler's and change-gutter colours (`theme::editor_diff_*`,
`editor_debug_*`, `profiler_*`) and the jump flash. They mean something to the app, not to a colour
scheme. `theme::syntax` still exists for the few non-editor uses of a code-like colour.

## How a name gets its role

`script_manager::syntax::classify(source, table)` is the only classification of Pine text. With the
script's `SymbolTable` its answer for a name is final: a name the table resolves is the script's (a
variable, parameter, function or type), a name it does not place is looked up in the catalog. So a
local `hl2` is the script's inside its function and the builtin outside it, and a read of `close`
above a line that declares `close` is the builtin (the same rule `PINE7006` uses). Without a table
(chat code blocks, the change popover, the AI review) the declared names of the lexical pass stand in:
variables and functions, but not parameters. Half-typed code never fails; it resolves less.

Spans carry roles, not colours, and `layout_job` looks the colour up when the text is laid out. So a
colour change classifies nothing again, and the span cache (`EditorState::spans()`, keyed on the
text) is untouched by a theme change. `layout_job` joins neighbouring runs of equal colour into one
`LayoutSection` (egui shapes each section on its own), which is why giving user names and punctuation
the default colour, as Dark and Light do, costs nothing. Text typed in a frame is classified once, by
the layouter, and handed to the state afterwards (`Classified`), so the next frame finds the spans and
the symbol table cached.

## The model

`model/editor_theme/` is pure (no egui, no storage).

* **Built-in themes** (`builtin.rs`) are `const` palettes in code with stable ids `builtin.dark`,
  `builtin.light`, `builtin.dracula`. They are never stored, so an update can only improve them and a
  person can neither edit, overwrite nor lose one. A name that is a built-in's (or "Automatic") cannot
  be a custom theme's.
* **Custom themes** (`EditorTheme`) are a base (a built-in's id) and the colours they override, as
  `colour key → #RRGGBB[AA]` text, plus an optional typography override.

**Inheritance decision: one level deep.** A custom theme is a built-in plus overrides. Making a theme
from another custom theme, or duplicating one, copies the base and the overrides, so no theme ever
points at another custom theme: there is no cycle to prevent, no parent to delete from under a child,
and no chain to resolve. A colour set to the base's own value is not stored as an override, so a theme
stays as small as what was changed, and resetting a colour removes its override, which gives back the
*base's* colour, not a fixed default. A base this version does not know reads as Dark.

Overrides are kept as the text they were written as. A key this version does not know (a newer
version's colour) or a value it cannot read is carried along untouched and ignored when the palette is
built; fields a newer version added to a theme are kept (`extra`) and written back. A colour that cannot
be read costs that colour only, never the theme.

`ThemeBook` is the one place the rules live: names (trimmed, up to 48 characters, no control
characters, any Unicode, unique ignoring case against every theme, the built-ins and "Automatic",
`free_name` for `Name 2`), at most 200 custom themes, create/duplicate/rename/delete, set/reset a
colour, reset all, restore. Ids are wall-clock nanoseconds like every document id.

**The choices** (`EditorPrefs`, setting `editor.prefs`): which theme (`auto`, `builtin.dracula`,
`custom.<id>`) and the typography. **Automatic**, the default, is Dark in a dark app and Light in a
light app: what the editor did before themes, so an upgrade changes nothing. A selection that names
nothing (a deleted theme, a value from the future) is Automatic, and the correction is saved. Every
field is read on its own, so one value this version cannot read costs that field only; fields it does
not know are kept.

**Typography** (`Typography`): font family (none is the system's monospace font), size 9–32 points
(kept to half points; the field and the stepper clamp, and a value that is not a number is dropped),
and line height (Compact 1.15×, Default which is the font's own, Relaxed 1.5×, Loose 1.75×). A theme
may carry its own for the fields it names; what it does not name is the person's. Ligatures, font
weight and tab width are **not** settings: egui's text engine shapes with the font's default features
and has no weight axis for a plain font, and the editor indents with four spaces, so a control for any
of them would do nothing.

## Dark, Light and Dracula

Dark and Light are today's colours, exactly: `dark_and_light_match_the_editor_before_themes`
rebuilds the old values from the functions the editor used to call (and from the old `theme::syntax`
table, written out in the test) and compares every role and surface in both appearances. Roles that did
not exist before take the colour of what they were part of: user variables, parameters, user functions
and punctuation the default text colour, escapes the string colour, control keywords the keyword colour.
So an upgrade changes nothing a person can see.

Dracula follows the [Dracula specification](https://spec.draculatheme.com): keywords, operators and
escapes pink `#FF79C6`; user functions green `#50FA7B`; parameters orange `#FFB86C`; strings yellow
`#F1FA8C`; constants purple `#BD93F9`; comments `#6272A4`; types and built-ins cyan `#8BE9FD`; variables
and brackets the foreground `#F8F8F2`; selection `#44475A`; errors `#FF5555`. The specification gives no
colour for: numbers (purple, as the editor ports use for constants), namespaces (cyan, with the other
built-ins), directives (purple), colour literals (orange); nor for the current line (`#44475A`,
translucent), the gutter (`#21222C`, the palette's darker background), line numbers (`#6272A4`), search
matches (orange, translucent), the word highlight (cyan, translucent) or the bracket match (purple,
translucent). Those are our choices, and so is mapping its italics away: the editor draws one style.

## Where a theme is used

`ui::editor_appearance` resolves a theme into `Color32`s once (an `Appearance`, shared by `Arc`);
`current()` is what the editor wears and `editor_font()` the font. The host replaces them
(`publish`) and asks every window to repaint, so a change reaches every editor, the preview and every
detached window the same frame: only rendering inputs change, so cursor, selection, undo history,
scroll position, unsaved text and diagnostics are untouched and nothing is recompiled.

Code on the app's own background (the change popover, chat code blocks, the AI review) reads `chrome()`:
Dark or Light by the app's appearance whatever theme the editor wears, because a theme's colours are
chosen for its own background. It does use the editor's font family.

The selection and caret reach egui's text field through the visuals of the editor's own `Ui`
(`selection.bg_fill`, `selection.stroke.color` for the selected text, `text_cursor`), so extra carets,
which read the same visuals, agree.

## Fonts

The font a person picks is found among the installed fonts (`service::font_catalog`): the system's font
folders are walked on a background thread, reading from each file only its table directory and the
`name`, `OS/2` and `post` tables (kilobytes, not the font), and a family is listed once, by its regular
face, when it declares itself monospaced. `.ttc` collections are read member by member; hidden system
fonts (a name starting with a dot) and anything unreadable are skipped. The chosen file is read on
another thread (at most 64 MB) and given to egui as the `editor-code` family: the font, then the
monospace font and its fallbacks, so a character the font lacks still draws. egui compares fonts byte
for byte when they are set, so they are set when the choice changes and not otherwise.

A saved font that is not installed here keeps its name (it comes back when it is installed) and the
default draws meanwhile; the picker marks it "not installed". A font that cannot be read says why. A
font that is still loading, or one the catalog has not found yet, never blocks the editor. No font
file is bundled: the default stays the system's monospace font (SF Mono on a Mac), and JetBrains Mono
and the rest appear in the picker only where installed. The picker draws names in the app's font; the
live preview shows the chosen one (loading every installed font to draw its own name would read
hundreds of files for one list).

The editor's layout follows the font: line numbers are two points smaller than the code, the gutter
widens for five-digit line numbers and large fonts, the suggestion list and signature panel scale with
the font (limited to 0.9–1.5×, so a 32-point font does not make a 54-point row), and a line height other
than the font's own sets every row's height.

## Saving

Themes are the `editor_theme` document table (id, position, JSON); the choices are the `editor.prefs`
setting. Both go through `Store::replace_documents` / `set_setting`. The host (`EditorThemeHost`) holds
them in memory; a change takes effect at once and is **saved about 400 ms after the last one**, so
dragging a colour writes the database once, and on quit. Nothing here touches the store while Settings
draws, so it cannot deadlock with the store the Settings frame holds.

A theme row that cannot be read does not hide the others, but the table is then not rewritten (that
row would be lost): changes to themes are refused with the reason, while choosing a theme and the
typography still work and are saved. A value of `editor.prefs` that is not even an object is replaced by
the defaults. There are no migrations: the defaults are the old editor.

## Sync

Custom themes are `EntityKind::EditorTheme` (record `editor_theme.<id>`, the `editor_theme` table), merged
by last writer like the other templates and switched with the Templates category; the choices are the
portable setting `editor.prefs`, last writer. Built-in themes are code and are never uploaded. Rules that
follow from how it works:

* A theme record is applied only if its id is the record's (numeric) and at least 1000, it has a name, and it
  decodes as an `EditorTheme`; anything else waits in the sync inbox, as for every kind. Fields and colour
  keys a newer version wrote are kept and written back.
* A choice can arrive before the theme it names (the two are separate records). Until the theme arrives the
  editor is Automatic, **and the choice is not rewritten as Automatic**: that would be uploaded and undo the
  choice on the Mac that made it. The host reads the store again when sync reports `editor_theme` or
  `editor.prefs` changes (`EditorThemeHost::reload`); a Mac with changes it has not saved yet keeps them and
  reloads after saving, so its later edit wins.
* A font that is not installed on another Mac keeps its name there (it is part of the synced choice) and
  the default draws. A theme's own font and size travel with the theme.
* *Use iCloud's data* replaces the custom themes with iCloud's (the table is cleared bypassing the outbox,
  like the other templates).
* Deleting a theme here deletes it everywhere; a mass delete is held for confirmation by the usual rule.

Tested on the two-Mac harness (`sync/theme_tests.rs`): themes arriving whole and not echoing back, rename,
edit and delete, the later edit winning, records that are not themes kept not applied, future fields
surviving, the choices with a missing font, and a choice that arrives before its theme. **Not run against
real CloudKit**, like the rest of sync (see [iCloud Sync](icloud-sync.md#what-is-not-verified)). A change to
the synced records is a release-candidate change under `AGENTS.md`.

## The file format

`degenview-editor-theme`, version 1, JSON (`model/editor_theme/codec.rs`):

```json
{
  "format": "degenview-editor-theme",
  "version": 1,
  "name": "My Dracula",
  "base": "builtin.dracula",
  "syntax": { "default": "#F8F8F2", "keyword": "#FF79C6", "…": "…" },
  "editor": { "background": "#282A36", "selection": "#44475A", "…": "…" },
  "typography": { "fontFamily": "JetBrains Mono", "fontSize": 14.0, "lineHeight": "relaxed" }
}
```

An export is complete (every colour, resolved, so it looks the same whatever a later version does to
the built-in it started from), human-readable, and holds nothing machine-specific: no paths, ids or
times. `typography` holds only what the theme sets. Colours are `#RRGGBB` or `#RRGGBBAA` (no shorthand,
no names). The file is `<name>.degenview-theme.json`.

Import reads at most 256 KB, refuses anything that is not this format and a version this build reads
(with "update DegenView"), a name it cannot use and a base it does not have; every colour and
typography value is checked on its own, and a bad one is skipped and reported (not guessed). Colours
equal to the base's are dropped, so the imported theme overrides only what differs. **An import never
replaces a theme**: it gets a new id and the first free name (`Name 2`), and it is worn once imported.
Where there are no file dialogs (Windows) Import is not offered and Export writes to the downloads folder.

## The page

Settings ▸ Script Manager (after Appearance): the theme tiles (Automatic, the built-ins, your own; each
a miniature of the editor in its colours, a Built-in/Custom caption, a mark for the one worn, a menu
and a right-click menu: Edit, Duplicate, Rename, Export, Delete for yours, Duplicate & Customize and
Export for the built-ins), **Create Theme** (a name and which theme to start from, checked as you
type), **Import…**, the typography rows (a searchable font list, a size you can type or step, a line
height), Theme Customization, and the live preview.

*Edit Current Theme* (for a custom theme; the others offer *Duplicate & Customize*, which makes a copy
and opens it) replaces the page with the theme editor: the real editor pinned above (beside at 700
points and wider), tabs Syntax / Editor / Typography with the number of colours changed, a search over
names, purposes and keys, and the colours in groups: a swatch that opens the app's colour picker (hex,
RGB, opacity where the surface is an overlay, system colours, recents), the name with a tooltip, the hex
value, and a reset that is there only while the colour differs from the base's. A group's menu resets
the group. **Reset Changes** puts the theme back as it was when the editor opened, **Reset Theme…**
(after asking) puts it back to its base, **Done** leaves; changes apply as they are made, so there is
no Apply. A notice, never a block, names colours that are close to the colour behind them (WCAG contrast
below 2). Keyboard: the tiles are one tab stop each and the arrow keys move between them, Return picks;
Return and Escape answer the sheets; Escape closes the pickers.

Narrow windows: the smallest Settings window is 520 × 400. Below 470 points the tabs become plain titles
with the search under them, below 500 the two resets share a menu, and the tiles go two to a row.

## What is verified

Unit tests: the roles and keys; the Dark and Light equality above; every classification case
(shadowing, parameters, named arguments, escapes, CRLF, astral characters, half-typed code); the model
(names, inheritance, flattening, overrides, resets, unreadable and future data, the 200-theme ceiling);
the codec (round trip, every kind of bad file, an import never replacing a theme); the store (round
trip, a row that cannot be read, future fields); the host (publishing, debounced saves, quitting,
fonts: read off thread, missing, unreadable, stale, waiting for the list); the font catalog on synthetic
fonts (styles, collections, hostile files); the page and the editor through egui (every tab at every
size in both appearances, keyboard navigation, the sheets, delete and reset confirmations, import and
export results); the real editor painting each surface and role colour, and repainting on a change
without touching text, caret or span cache; the gutter and line height at 9 and 32 points.

Pictures: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib -- settings_script_manager::tests::gallery
settings_theme_editor::tests::gallery --ignored` writes the page and every tab of the editor at three
sizes in both appearances; they were looked at, and the layout was changed for what they showed (the
tile miniatures, a preview that left no room for the colours, tabs that ran into the search at 320
points).

A release build was launched against an empty home folder: it ran for minutes, created the `editor_theme`
table in a fresh database and did not crash. **No one has used the page in a real window**: the
pictures above are rendered by the tests, not captured from the app, and driving the real window from
a script was not possible here (the macOS automation prompt was never answered). Open Settings ▸
Script Manager, pick each theme, edit a colour, change the font and size, and open a script with a
second window before calling it done.

Not verified: the colour picker's low-alpha hex round trip (the picker works on premultiplied colours,
so a hex typed with a low alpha can come back a point or two off in a channel); Windows and Linux
rendering; fonts other than the ones on the machine it was built on; iCloud (not synced).

## Performance

Measured in a release build on an Apple Silicon Mac, a synthetic script of repeated blocks (calls,
user functions, strings, comments, control flow;
`cargo test --release --lib -- syntax::tests::timings --ignored --nocapture`):

| Script | `analyze` + `classify` (a keystroke) | `classify` on a cached table | `layout_job` (a frame) |
| --- | --- | --- | --- |
| 82 lines | 0.22 ms | 0.12 ms | 0.01 ms |
| 802 lines | 3.0 ms | 2.1 ms | 0.09 ms |
| 4,802 lines | 20.7 ms | 8.3 ms | 0.96 ms |

Before themes a keystroke classified twice (the layouter, then the next frame's span cache); the
lexical classification alone, which is most of what that did, measures 1.4 ms for the 802-line script
(7.1 ms for the 4,802-line one), so about 2.8 ms for the keystroke. It now analyses and classifies once,
at about the same cost for an ordinary script. The old figure was not measured on the old code: it is
inferred from the lexical pass. A very large script pays more per keystroke than it did; a frame without
typing costs only the layout.

A theme switch resolves a 40-colour palette and swaps an `Arc`: the host's test makes 6,000 switches
among 150 themes in a debug build in a fraction of the 0.26 s the whole host suite takes. A colour
drag makes no database writes until it pauses.
