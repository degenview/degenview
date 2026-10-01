# Pine corpus

Fetches the most popular open-source TradingView scripts (indicators and libraries, Pine v6
only) so `PineCorpusTests` can check the engine against real-world code.

```bash
python3 tools/pine-corpus/fetch.py              # 20 indicators + 10 libraries
python3 tools/pine-corpus/fetch.py --indicators 50 --libraries 20
```

- Sources are written to `.pine-corpus/` at the repo root, which is **gitignored**. Scripts
  carry their authors' licences (mostly MPL-2.0, some custom); this repo is GPL-3.0, so third-party
  source is never committed. Do not copy corpus code into tests — write original reductions.
- `DegenViewTests/PineCorpus/manifest.json` is committed and holds metadata only.
- `DegenViewTests/PineCorpus/expectations.json` records, per script, the diagnostics the engine is
  currently expected to report.
- Without the cache, `PineCorpusTests` skips. Point at another location with
  `PINE_CORPUS_DIR=/path xcodebuild test ...`.
- The listing pages and `pine-facade.tradingview.com` are not a documented API. The tool is
  rate limited (1 request/s), sends no credentials, skips closed-source scripts, and exits non-zero
  if the layout changes. Use it for local compatibility testing only.
