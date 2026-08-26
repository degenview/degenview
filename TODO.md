# DegenView Roadmap

This document tracks potential features, architectural improvements, and maintenance
work. Items are grouped by area rather than priority.

## Charts and Layout

- [ ] **Add horizontal price-alert lines to charts.** Display active price alerts as
  horizontal trend lines over the relevant chart, with clear labels and distinct
  styling for above-price and below-price alerts.
- [ ] **Add a global indicator visibility toggle.** Provide a quick way to hide or show
  every enabled indicator on a chart without removing or reconfiguring them.
- [ ] **Support layouts with more than two columns.** Extend the tab layout options for
  larger screens and grids containing many charts.
- [ ] **Use chart titles as drag handles for layout rearrangement.** Reorder charts by
  dragging their title or header instead of making the entire chart surface draggable.
- [ ] **Add TradingView-style chart dragging.** Allow users to pan through chart history
  naturally by dragging the plot area, with appropriate boundary and loading behavior.

## Watchlists and Market Data

- [ ] **Support multiple favorites lists.** Let users create, rename, reorder, and delete
  separate lists of favorite markets or symbols.
- [ ] **Support Coinbase WebSocket feeds.** Add live Coinbase market-data streams behind
  the existing data-source abstraction, including connection lifecycle and reconnect
  handling.
- [ ] **Add a financial-news and events view.** Surface relevant market-moving events,
  such as Federal Reserve meetings, economic releases, and major crypto news.

## Portfolio

- [ ] **Fix decimal-place handling in Portfolio Manager.** Format prices, quantities,
  values, and profit/loss consistently across assets with very different magnitudes,
  without losing meaningful precision.

## Pine Script

- [ ] **Improve Pine Script support.** Expand language and runtime compatibility,
  improve diagnostics, and document the currently supported subset.
- [ ] **Evaluate extracting the Pine engine into an external dependency.** Determine
  whether the engine should become a standalone package, including API boundaries,
  versioning, testing, and the tradeoffs of introducing an external dependency.

## Persistence and Sessions

- [ ] **Migrate persistent data from JSON to SQLite.** Design a schema and migration path
  for tabs, saved views, alerts, portfolio data, and other persisted state while
  preserving existing user data.
- [ ] **Restore the previous tab session after relaunch.** Verify and harden restoration
  of tab contents, ordering, window grouping, layouts, and per-chart state when the app
  closes and reopens.

## Quality and Maintenance

- [ ] **Write proper documentation.** Add setup and usage guides, document major
  features, and keep architecture and contributor guidance current.
- [ ] **Increase unit-test coverage.** Add tests for uncovered business logic, edge
  cases, persistence migrations, data-source failures, and regressions.
- [ ] **Clean up the codebase.** Address code smells and oversized functions, clarify
  responsibilities, remove duplication, investigate memory leaks, and profile likely
  performance bottlenecks.
- [ ] **Adopt and run a Swift linter.** Establish project-wide lint rules, fix existing
  violations (including in `AlertsCenterView`), and integrate linting into the normal
  development workflow.
