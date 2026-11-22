# Changelog fragments

A pull request with a change a person using DegenView would notice and care about adds one file
here, `changes/<short-slug>.md`. `tools/release.sh` collects the fragments into `CHANGELOG.md` and
deletes them when it cuts a release, so two pull requests never edit the same lines.

A fragment holds one change per line, and nothing else (no headings, bullets or blank-line prose):

```
new: Charts can be compared against another market.
fixed: Charts no longer go blank after switching to a stock.
fixed-rc: The Update button no longer opens an empty sheet.
```

## What gets a line

The list is what a trader would tell a friend about the update, so it is short.

- A capability that was not there before: `new:`.
  - "Charts can be compared against another market."
- A bug a person could have hit in a release: `fixed:`.
  - "Pine scripts with loops now show the right values when a loop runs a different number of times
    from bar to bar."
- A bug that only ever existed in release candidates, never in a stable release: `fixed-rc:`. These
  are listed for people running a candidate, and hidden (an HTML comment) in the notes of a stable
  release, because people coming from a stable version never had the bug. So no change may contain
  `<!--` or `-->`.

No fragment for:

- Polish: spacing, colours, icons, a button that moved or was renamed, copy tweaks.
  - "The Watchlist button now sits at the right end of the toolbar."
- Refactors, performance work with no visible effect, tests, CI, docs, tooling.

## One change, one fragment

- Before adding a fragment, look in `changes/` for one about the same change. If there is one, edit
  its line (sharpen the wording) instead of adding another.
- A feature built over a stack of pull requests gets its line from the pull request that makes it
  usable, not one per pull request.
- A follow-up that finishes or fixes something not yet released adds nothing; at most it edits that
  change's line. Nobody saw the broken state.
- After a release, a fix is its own fragment: `fixed:` when the bug reached a stable release,
  `fixed-rc:` when it only reached release candidates.
- `tools/changelog.sh check` refuses the same text on two lines, in one file or two.

## Writing the line

- Write for a trader, not a developer: what they see or can now do, in plain words. No function,
  module or crate names, no issue numbers.
- One line, starting with a capital and ending with a period.
- Several changes in one pull request are several lines, in one file or more.

## Before a release

`tools/release.sh <version> --dry-run` prints the section the fragments make. Read it as the person
updating would: merge or reword entries that describe one change, and drop the ones that are not
worth a line. Do that in a normal pull request before cutting the release.

`tools/changelog.sh check` is the only validator of the fragments; CI's lint job and `cargo test`
both run it.
