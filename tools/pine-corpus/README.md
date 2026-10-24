# Pine corpus

Fetches the most popular open-source TradingView scripts (indicators and libraries, Pine v6
only) so the corpus test can check the engine against real-world code.

```bash
python3 tools/pine-corpus/fetch.py              # 20 indicators + 10 libraries
python3 tools/pine-corpus/fetch.py --indicators 50 --libraries 20
python3 tools/pine-corpus/fetch.py --append --indicators 20 --libraries 10   # 20 + 10 not yet in the manifest
```

```bash
python3 tools/pine-corpus/fetch.py --imports    # the libraries the cached scripts import
```

`--imports` finds each `import user/Library/version` of the cached scripts on TradingView's search page by library
name, fetches that version into `.pine-corpus/imports/user/Library/version.pine` (recursively), and records
metadata only in `src/pine/corpus/imports.json`. The corpus test resolves `import` from that folder. A library the
search does not return, or whose source is closed, is recorded as skipped and its importers report `PINE3040`.

TradingView's popularity order changes daily, so a plain re-run may return different scripts than the
manifest lists (and rewrites it). Use `--append` to widen the corpus without losing what is there.

- Sources are written to `.pine-corpus/` at the repo root, which is **gitignored**. Scripts
  carry their authors' licences (mostly MPL-2.0, some custom); this repo is GPL-3.0, so third-party
  source is never committed. Do not copy corpus code into tests — write original reductions.
- `src/pine/corpus/manifest.json` is committed and holds metadata only.
- `src/pine/corpus/expectations.json` records, per script, the diagnostics the engine is
  currently expected to report.
- Without the cache, the `pine::corpus_tests::corpus_matches_expectations` test skips. Point at another
  location with `PINE_CORPUS_DIR=/path cargo test`. Run just a few scripts with
  `PINE_CORPUS_ONLY=slug,slug cargo test`.
- The test reports every script that differs from `expectations.json` at once, with the first error's
  `line:col` and message. `PINE_CORPUS_REPORT=1` prints one line per script (status, warning count, time),
  `PINE_CORPUS_BARS=5000` runs longer histories (the default is 1000). Use `--release`: the debug build is
  about ten times slower and a script that is merely slow there trips the 10 s evaluation deadline.
- `expectations.json` is for the default run (1000 one-minute bars). `PINE_CORPUS_SPACING=900` (or 3600, 86400) is for exploring: scripts that check the chart's timeframe on purpose (`runtime.error("Use an intraday chart…")`) will then differ, and scripts that draw nothing on a 1-minute chart may start to draw.
- To reduce one failure, run a single file with
  `PINE_RUN_FILE=path/to/script.pine [PINE_RUN_BARS=n] cargo test --release run_one_file -- --nocapture`.
- A script that cannot run by design is recorded in `expectations.json` (one that calls `runtime.error`
  on this synthetic 1-minute chart is a `runtime-error`). A `request.footprint` script compiles and runs,
  but its footprint is `na` on a 1-minute chart (nothing finer is fetched): use `PINE_CORPUS_SPACING=300`
  to see it draw. Everything else must be `ok`: fix the engine, or make what it cannot do a warning.
- The listing pages and `pine-facade.tradingview.com` are not a documented API. The tool is
  rate limited (1 request/s), sends no credentials, skips closed-source scripts, and exits non-zero
  if the layout changes. Use it for local compatibility testing only.
