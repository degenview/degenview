# Decoupling review (November 2026)

A second architecture pass after [the first refactor series](plan.md). It asks four questions:
how tightly components are coupled, whether concerns are separated, what would break if
components were extracted into crates, and whether the code follows clean-code rules. Numbers come
from the source on `main` at `339e200b` (v0.6.0-rc.32), counted by script (test files and inline
test modules left out unless stated). The stacked pull requests #638–#645 fix what could be fixed
without changing behaviour. The rest is listed under [Left to do](#left-to-do), most important
first.

## Shape of the code

About 420,000 lines of non-test Rust in one crate (528,000 with tests). By top-level module:
`ui` 181k, `model` 70k, `pine` 63k, `pine_studio` 52k, `service` 45k, `mcp` 28k, `workspace` 24k,
`host` 24k, `sync` 19k, `store` 12k, `trading` 5k, `app` 4k.

The first series left the main layering in good shape. `model/` and `pine/` import no I/O,
coordinator or view code. Views don't live in hosts. `layout_tests.rs` enforces both, and its debt
lists are empty. Below that level this pass found cycles and back-references that would each stop
a crate split.

## Findings

### 1. Module cycles below the layering test's resolution

The test checked only a few directions. Measured import edges (files that name the other module
outside tests):

| Edge | Files | What it was | Status |
| --- | --- | --- | --- |
| `store → sync` and `sync → store` | 8 / 9 | `store` builds on `sync::record` (policy, ids, projections), and `sync::record` imported `store` for `scan_scripts` and `StoredAsset` | Fixed (#639, #640): `record < store < engine` is now a strict order, and the test holds `sync/record` off `store` |
| `store → service` and `service → store` | 2 / 5 | `store::asset_store` used the image service to verify bytes it never verifies; `frame_cache` cached a `service::net::proxy` type | Fixed (#639, #641): `store` imports no `service` |
| `store → pine_studio` | 1 | `store::scripts` imported the editor's name rules and library registry, the one item the first plan named before a crate split | Fixed (#642) |
| `service → mcp`, `service → sync` | 1 / 1 | The seal key size lived in the MCP transport; the iCloud container id lived in the sync engine | Fixed (#640, #643) |
| `crash_report → store`, `crash_report → host` | 1 / 1 | Only to find the data folder and name the build | Fixed (#638): `paths` and `build_info` are leaves |
| `model ↔ pine` | 5 / 26 | `pine` uses `Kline`, `Timeframe`, `ChartKind`, `market::bucket`, the synthetic ticker, the volume-profile POC and alert-message rendering from `model`; `model::pine::*`, `model::alert::pine` and `session_stats` use engine types from `pine` | **Open**, the main crate blocker (see Left to do) |
| `trading → host`, `pine_studio → host` | 2 / 7 | Venue adapters over the paper and broker hosts; the studio reaching hosts for the Pine worker and themes | Acceptable today: both are coordinator-level code |

### 2. Coupling through god objects and globals

| Type | Fields | Methods | Spread |
| --- | --- | --- | --- |
| `ChartWorkspace` | 40 | 311 | 33 `impl` files; named in 12 files outside `workspace/` (MCP host, app, tab links) |
| `Store` | 2 | 164 | 30 `impl` files; no trait; `SharedStore = Arc<Mutex<Store>>` named 148 times and `&Store` 70 times |
| `PineStudio` | 26 (12 `pub(crate)`) | 97 | 11 files |
| `DegenViewApp` | 26 | 34 | 9 files, all in `app/` |
| `McpHost` | 19 | 42 | 1 file |

`Store` is the widest seam: every layer from `service` to `ui` takes the concrete SQLite type, and
any crate that holds data (sync, MCP, hosts) would have to depend on the whole store crate. Fields
are private everywhere, so coupling goes through methods, not shared state.

About 100 process-wide singletons (`static`, `OnceLock`, `LazyLock`, `thread_local!`) hold state in
production code. Most are in `service/`: work queues, the HTTP agent, rate limiters, provider
catalogs and caches, Keychain stores. Others hold theme and editor appearance in `ui/`, the native
bridge's queues, and the Pine builtin registry. Each one ties a would-be crate to the process. The
Pine registry is fine because it is immutable. The mutable ones (`store::scripts::registry`,
`host::pine` security cache, `ui::theme`) need an owner passed in before they can leave the crate.

### 3. Separation of concerns

- **Views doing I/O.** `ui/pine/script_catalog.rs` rescans the Scripts folder synchronously on the
  UI thread (at most every two seconds while a menu or the Scripts page is open). 14 `ui/` files
  read or write settings through `Store` directly (pickers' favorites and recents, template
  managers). Pine Studio's preview writes the inputs file and a zoom setting from `tick()`.
- **Background work by hand.** Pine Studio spawned a thread and built a channel for one answer
  (fixed in #644, and now checked by a test). The assistant's streaming run keeps its own thread
  for a reason the test records. Nine places in `workspace/` build an `mpsc` channel around
  `work_queue::submit` instead of returning a `Pending`. They follow the rule, which is about views,
  but they repeat what `Pending` already does, including the repaint and the dropped-job case.
- **Error decisions on text.** The Settings page read CloudKit's wording to recognise an undeployed
  schema (moved to the sync transport in #645). The other four places read a third party's text at
  the boundary where it arrives, which is where that belongs: osascript's `-128`, ImgBB's messages,
  rmcp's parameter error. 142 functions still return `Result<_, String>` (service 89, sync 31, store 14,
  model 8), and 63 `map_err(|e| e.to_string())` calls throw away the error type early.

### 4. Clean code

- **Copy-pasted helpers.** 15 identical `now_ms` functions, 2 byte-identical `hex` and 5 inline
  hex loops, 3 `civil_from_days`, 3 `plural`. All fixed in #643. Still duplicated: 4 `finite`, 2
  `finite_or`, 2 `round_to`, 2 `days_in_month`, 3 `parse_iso8601` with different leniency.
- **Lint suppression.** 55 `clippy::too_many_arguments`, 19 `cfg_attr(<os>, allow(dead_code))`
  where the rule asks for `cfg` gating, and 2 plain `dead_code` (`model/webhook/policy.rs`,
  `model/indicators/catalog.rs`).
- **Long functions.** `ui::icons::paint` is 2,121 lines (one icon `match`). `workspace::grid::render_grid`
  is 757, `pine_studio::editor::text_view` 466. 19 functions are over 250 lines.
- **Naming drift.** `model/drawings` against `drawing` everywhere else; `model/indicators` against
  `mcp/indicator`; `workspace/alerts.rs` and `workspace/liquidations.rs` against the singular domain
  names; `ui/editor_appearance` against `editor_theme`.

## What the stack does

| PR | Change | Rule the layering test now holds |
| --- | --- | --- |
| [#638](https://github.com/degenview/degenview/pull/638) | `paths` (data and cache folders) and `build_info` (version, build id) become leaf modules; the crash reporter no longer needs the store or the updater host. | Leaves import nothing; `crash_report` imports no store, host or service code; the test reads single-file modules and checks its list against `lib.rs` |
| [#639](https://github.com/degenview/degenview/pull/639) | `StoredAsset` is a `model` value; `Ingested::stored` and `ingest::verified_bytes` make and check one | |
| [#640](https://github.com/degenview/degenview/pull/640) | `scan_scripts` moves into `sync::engine`; the iCloud container id moves into `build_info` | `sync/record` imports no `store`; `service` imports no `sync` |
| [#641](https://github.com/degenview/degenview/pull/641) | The Settings host caches the proxy setting itself | `store` imports no `service` |
| [#642](https://github.com/degenview/degenview/pull/642) | Script names go to `model::pine::script_name`, the library registry to `store::scripts::registry` | `store` imports no `pine_studio` |
| [#643](https://github.com/degenview/degenview/pull/643) | One `clock::now_ms`, `model::encoding::{hex, hex_decode_array, sha256_hex}`, `format::plural`, one calendar | `clock` is a leaf; `service` imports no `mcp` |
| [#644](https://github.com/degenview/degenview/pull/644) | The model list goes through `work_queue::Pending` | `ui/` and `pine_studio/` start no threads or channels (one listed exception) |
| [#645](https://github.com/degenview/degenview/pull/645) | CloudKit's schema error is read at the sync boundary | |

## Left to do

In order of how much each one blocks a crate split or invites bugs:

1. **Break `model ↔ pine`.** Put a `core` crate under both. It would hold `Kline`, `Timeframe`,
   `TimeUnit`, `ChartKind` and its construction rules, `market::bucket`, the synthetic ticker,
   volume-profile POC/value-area and alert-message rendering. Then `pine` depends on `core` only,
   and `model` on `core` and `pine`. The unpushed `refactor/market-crate` branch already moves
   `Kline`, `Timeframe`, `bucket`, the calendar and the viewport into `crates/market`. Finish that
   branch and the rest follows. Add `("pine", ["model"])` to the test once it holds.
2. **A storage seam.** Give each domain that persists a small trait in its own layer (`AlertStore`,
   `PaperStore`, `SettingsStore`, …) and have `Store` implement them. Hosts, sync and MCP take the
   trait, so they can move to crates that don't depend on SQLite. This is the plan's "Store as
   per-domain repositories", now the largest single coupling.
3. **Views off the store.** The 14 `ui/` files that read or write settings take their values from a
   host and hand actions back. The script catalog rescans on a worker and keeps showing its last
   list meanwhile. This needs a real-window check that no empty list flashes on first open.
4. **`workspace/` channels to `Pending`.** The nine hand-built channels (candles, older pages,
   overlays, snapshots, link previews, pictures) go through `start_for`. This touches the candle
   paths, so it needs a GUI pass on a release build.
5. **Typed errors in `service` and `sync`.** Replace `Result<_, String>` in `sync::record::scripts`
   (the `ScriptSink` trait), `sync::record::validate`, `service::keychain::secret_service`,
   `service::update::archive` and `portable_binary` with per-module `thiserror` enums. The text
   stays the same at the edge where it is shown.
6. **`cfg` instead of `allow(dead_code)`** for the 19 platform cases. Each needs a Linux or Windows
   build to prove, so do them per platform with CI watching.
7. **Mutable singletons with an owner**: the library registry, the security cache and the theme
   atomics, passed in where they are used, before the Pine engine or UI kit leaves the crate.
8. **Naming**: rename `model/drawings` to `drawing`, `model/indicators` to `indicator`, and
   `workspace/alerts.rs` and `liquidations.rs` to singular names, move-only in one PR each, after
   the open feature stacks merge, since every one of them touches these paths.
