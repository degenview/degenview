# Changelog fragments

Every pull request with a change a person using DegenView would notice adds one file here,
`changes/<short-slug>.md`. `tools/release.sh` collects the fragments into `CHANGELOG.md` and deletes
them when it cuts a release, so two pull requests never edit the same lines.

A fragment holds one change per line, and nothing else (no headings, bullets or blank-line prose):

```
new: The toolbar puts the Watchlist button at the far right.
fixed: Charts no longer go blank after switching to a stock.
```

- `new:` for something that was not there before, `fixed:` for something that was broken.
- Write for a trader, not a developer: what they see or can now do, in plain words. No function,
  module or crate names, no issue numbers.
- One line, starting with a capital and ending with a period.
- Several changes in one pull request are several lines, in one file or more.
- Internal-only work (refactors, tests, CI, docs, tooling) has no fragment.

`tools/changelog.sh check` validates the fragments; `cargo test` does too.
