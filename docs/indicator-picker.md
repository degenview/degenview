# Indicator catalog and bundled indicators

How a chart gets an indicator: the one catalog every add-indicator surface draws from, the Pine
indicators that ship inside the app, and the rules for adding one.

## The catalog

`model::indicator_catalog` is pure data and search. A `CatalogEntry` is one row: a native built-in
(`BuiltIn`), a bundled Pine indicator (`pine_studio::indicators`) or a saved script. Its `EntryKey`
(`Native`, `Bundled`, `Script`) is the stable identity that tells one row from another.

`search` is case-insensitive and ignores surrounding spaces. A title that starts with the query comes
first, then a title or alias that contains it, each group in catalog order. Surfaces that group rows
into sections (the chart's "+" menu) partition stably afterwards, so the rank survives inside a
section.

The chart's "+" menu (`ui::add_indicator_menu`) lists the native built-ins not yet applied, then the
bundled indicators, under "Built-in", and the saved scripts under "Scripts"; on a line chart only the
natives that run there. A bundled row's tooltip is its family and summary, and it shows "On chart"
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
