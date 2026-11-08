# Indicator picker, catalog and bundled indicators

How a chart gets an indicator: the picker modal, the one catalog every add-indicator surface draws
from, the Pine indicators that ship inside the app, and the rules for adding one.

## The picker

The toolbar's **Indicators** pill (right of the timeframe selector, `ToolbarAction::OpenIndicators`)
opens `ui::indicator_picker`, a modal owned by `ChartWorkspace::indicator_picker` and shown beside
the Add Chart sheet. It adds to the **focused** chart (`ChartWorkspace::indicator_target` names it;
a portfolio card is not a target) through `apply_indicator_action`, so undo is one step.

- **Sidebar**: Yours (Favorites, My scripts, Open Pine Studio), Built-in
  (Overlays, Separate pane), Library (Discover, All indicators), Families. Counts sit on the right.
  Searching lists matches across every section (`sections::search_blocks`) and ignores the sidebar.
  Discover is a fixed "Popular" row and the rest in a seeded order (the Shuffle button bumps the
  seed, so a shuffle is repeatable and needs no random source). `sections` is pure and tested.
- **Cards** show a generated chart (`preview.rs`): fixed synthetic bars, natives computed on the spot,
  each bundled script run once by a worker thread on first open and cached; a saved script, or one
  whose run failed, gets a glyph. Never run a script on the UI thread.
- **Detail panel** (dropped below 900pt of window): preview, description, author and license, a
  neutral "Add to chart" (never the accent colour), Favorite. A native indicator already on the
  chart offers "Remove from chart"; a script or bundled one "Add another copy".
- **Keys**: ↑/↓ move a row, ←/→ a card (left to the search field while it holds text), Return adds
  and closes, Shift-Return adds and stays open, F stars the selected one (only when the search
  field is not focused), Esc or the Done button (bottom right) closes. Return never removes. The keys are taken before the search
  field is drawn (`read_keys`), like the chart-type menu's.
- **Availability** (`availability`): no focused market chart, or a line chart for anything but a
  native other than Volume, is blocked with the reason shown; blocked entries are dimmed and Return
  does nothing.
- **Favorites** are `native:rsi` / `bundled:macd` / `script:<id>` strings in the
  `indicatorPicker.favorites` setting (portable, last writer wins, see `sync::policy`); a key this
  build no longer knows is dropped on load. Not verified against real CloudKit.

Pictures: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib indicator_picker::tests::gallery -- --ignored`
(a minute or two: it gives the preview worker time to finish).

## The catalog

`model::indicator_catalog` is pure data and search. A `CatalogEntry` is one row: a native built-in
(`BuiltIn`), a bundled Pine indicator (`pine_studio::indicators`) or a saved script. Its `EntryKey`
(`Native`, `Bundled`, `Script`) is the stable identity that tells one row from another;
`EntryKey::stored`/`parse` is the text the favorites keep.

`search` is case-insensitive and ignores surrounding spaces. A title that starts with the query comes
first, then a title or alias that contains it, each group in catalog order. Surfaces that group rows
into sections (the picker's sections) partition stably afterwards, so the rank survives inside a
section.

The chart header's "+" menu (`ui::add_indicator_menu`) is gone: the toolbar's Indicators button opens
the picker for the focused chart, and nothing else adds an indicator from a card. The picker lists the
native built-ins not yet applied, the bundled indicators and the saved scripts; on a line chart only
the natives that run there. A bundled row's tooltip is its family and summary, and it shows "On chart"
when the chart already has an instance (a bundled indicator can be added again). A pick is an
`Action` (`Add`, `AddScript`, `AddBundled`) that `ChartWorkspace::apply_indicator_action` applies as
one undo step; it refuses scripts and Volume on a line chart, so any surface can call it.

Families (`Family`: Trend, Momentum, Volatility, Volume & Flow, Structure) are what an indicator
measures. Every family must have at least one entry (a test checks it). Natives get their family,
summary and aliases in `CatalogEntry::native`.

## Bundled indicators

`pine_studio/indicators.rs` lists 20 Pine indicators, each a `.pine` file in `pine_studio/indicators/`
included with `include_str!`. They are not the demo scripts (`pine_studio::demo`, which are copied
once into the user's library and then belong to the user): a bundled indicator is always available
and is never written to the Scripts folder.

Adding one to a chart makes an ordinary Pine instance (own pane or overlay, inputs, styles,
visibility, undo, persistence, the MCP Pine tools). Its script id is `builtin:<id>`
(`indicators::script_id`); `indicators::from_script_id` goes back. The instance stores the source it
was added with, like every script, so a later release that changes a bundled script does not change
charts that already have it.

Rules for a bundled indicator:

- Original work written from the published definition of the study, under the app's license. Do not
  paste code from TradingView, a community script or another project. Vela (Apache-2.0) was read for
  names and default lengths only.
- It compiles with **zero diagnostics, warnings included**, and draws something on the test bars. Both
  are tests in `pine_studio::indicators`; the overlay flag in the table must agree with the script's
  `indicator(overlay = ...)`.
- An `id` is stable for ever: favorites and instances on saved charts refer to it. Never rename or
  reuse one.
- If a `ta.*` it needs is missing from the engine, write it out in the script. Do not extend the
  engine for a bundled indicator.
- `author` and `license` are shown in the picker's detail panel. They exist so a differently
  licensed pack can sit beside these later.

## Not bundled: LuxAlgo's Library scripts

LuxAlgo publishes its Pine indicators under CC BY-NC-SA 4.0. That cannot go into a GPL-3.0-only
program (the license forbids added restrictions, and share-alike needs the same Creative Commons
license), their Terms of Service forbid crawling and storing site material without written
permission, and there is no bulk source API. LuxAlgo's license page asks to be emailed
(partners@luxalgo.com) about redistribution or embedding. Until that is settled none is included.
