# Plan: deliver Pine `alert()` events as user notifications

> **Status: implemented.** Where this plan and the code differ, the code and
> `pine-compatibility.md` ("Script alerts") win. Changes from the plan: no `registerMigration`
> (the schema is one idempotent `createSchema`); `freq_all` events carry no DB dedupe key; the
> live-tail sections were superseded by `PineExecutionHost`; re-arm re-pins symbol and timeframe
> as well as the source hash.

## Context

Exploration changed the shape of this task. **`alert(message, freq)`, `alertcondition`, and the `alert.freq_*` constants already exist**
(`Pine/Runtime/PineRuntimeSession+Alerts.swift`, `Pine/Language/PineBuiltins.swift:34`). They only *record* a `PineAlertEvent`
into `PineVisualOutput.alerts`, which `ChartSettingsSheet` lists. Nothing notifies. Three facts drive the design:

1. **The app never runs Pine in realtime mode.** `ChartViewModel.reevaluatePine` (`:558`) re-runs the *whole* script via
   `PineEvaluation.run` → `PineRuntimeSession.evaluate(bars:)`, which executes every bar (including the forming one) as
   `.historical` (`PineRuntimeSession.swift:103`). It fires on every WebSocket tick (`applyKlineUpdate`, `:1087`) and every 5 s REST refresh.
   `barstate.isrealtime` is never true; `isconfirmed` is true on the forming bar. `.realtimeTick/.realtimeClose` exist only for tests.
2. **Pine runs only in the app.** `DegenViewAlertAgent` compiles only `Pine/Model` + `Model/Script` value types, no compiler/runtime, and
   polls prices itself. Only Binance has a live stream; hidden tabs suspend it (occlusion gating).
3. **No webhook/HTTP-POST/dispatcher infra exists.** Delivery is hard-wired to `UNUserNotificationCenter` in `AlertRuntimeHost.deliverPending`,
   and `AlertTriggerEvent`/`AlertBanner` are price-alert-shaped. Persistence is GRDB with JSON-payload tables, one migration (`v1`).

> **Update:** the live-tail model below is superseded. Pine now runs through a persistent per-chart
> `PineExecutionHost` (see `pine-compatibility.md`, Execution model): realtime bars are real
> `.realtimeTick`/`.realtimeClose` executions with rollback, `varip`, and `barstate.*`, and
> `PineAlertEvent` already carries `frequency`, `isRealtime`, `isConfirmed`. Alerts for delivery arrive
> as `PineExecutionUpdate.alerts` (realtime executions only) through `ChartViewModel.pineAlertHandler`, with a
> per-bar ledger for `once_per_bar`. Sections 1–2 below (`liveTail`, `evaluate(bars:liveTail:)`,
> `reevaluatePine(live:)`) are not needed; the router, guard, store, dispatcher and UI sections still apply.

**Decisions (confirmed with user):** app-side execution, structured so it can move to a worker later · live bar runs as a *live tail* inside
the existing full re-run (no persistent incremental session) · channels = macOS notification + in-app banner behind a dispatcher protocol,
**no webhook** (documented seam only) · editing a script **pauses** its alert until re-armed.

## Architecture

```
Pine script → PineRuntimeSession (alert() → PineAlertEvent{frequency,isRealtime,isConfirmed})
   → ChartViewModel.reevaluatePine(live:) result
   → PineAlertCoordinator.ingest(PineAlertRun)            [@MainActor, thin]
   → PineAlertRouter.route(run, subscriptions, guard)     [pure, unit-tested]
        · drop !isRealtime / run without live tail
        · match active PineAlertSubscription (chart+symbol+timeframe+sourceHash)
        · PineAlertFrequencyGuard.admit(...)
   → PineAlertStore (history row, unique dedupe key)  +  PineAlertDispatcher
        └ channels: UserNotificationChannel, InAppBannerChannel   (future: webhook)
```
Pine knows nothing about channels. Owner/user field is omitted: the app has no accounts (documented).

## Changes

### 1. Pine language/runtime (`DegenView/Pine/`)
- **New** `Pine/Model/PineAlertFrequency.swift` — `enum PineAlertFrequency: String, PineNamedConstant` (`all/oncePerBar/oncePerBarClose`,
  `pinePrefix = "alert."`, follows `PineSize`). `PineBuiltins.constants` derives the three names from it (replaces hand-listed strings at `:34`).
  Agent-shared folder → add to both Sources phases.
- `Pine/Model/PineAlertEvent.swift` — add `frequency: PineAlertFrequency`, `isRealtime: Bool`, `isConfirmed: Bool` (defaulted so existing
  tests compile). `time` stays bar-open time.
- `PineRuntimeSession+Alerts.swift` — use the enum; stamp `isRealtime/isConfirmed` from `context.flags`; unresolvable `freq` at runtime →
  runtime diagnostic (not silent default); message `na` → empty string per existing `PineFormat`.
- `PineRuntimeSession.evaluate(bars:liveTail: PineBarPhase? = nil)` — all bars `.historical` except the last, which uses `liveTail`
  (`.realtimeTick`/`.realtimeClose`) when supplied. `PineEvaluation.run(..., liveTail:)` passes it through.
- **Compile-time validation** — new `Analysis/PineAlertCallValidator.swift` run from `PineCompiler.compile` next to `PineStructureValidator`
  (reuse `PineStatement.forEachCall`): `alert` needs 1–2 args, only names `message`/`freq`; message type must not be a known non-string
  (extend `PineBuiltinTypes.call` for `alert` → void, and check arg types via `PineTypeChecker+Expressions.swift:153`); `freq` must be an
  `alert.freq_*` constant or matching string literal; `alertcondition` arity. Unknown-typed args pass (engine is lenient). Use the next free
  `PINE20xx/30xx` codes after checking `PineDiagnostic` usage. Expose `PineCompiledProgram.alertCallSites` (count + frequencies) for the UI.
- `Pine/Editor/PineSyntaxHighlighter.swift:22-32` — add `alert|alertcondition` to the builtin list.

### 2. Live-tail wiring (`ViewModel/ChartViewModel.swift`)
- `reevaluatePine(compiled:live:)`; `applyKlineUpdate` passes `.realtimeClose(isNew: false)` when `kline.isClosed`, else
  `.realtimeTick(isNew: appended)`, only when not in replay. REST-refresh / config / input / theme / initial-load runs pass `nil` (pure history).
- Capture `(chartID, symbolKey "<source>:<ticker>", timeframe, scriptID?, sourceHash)` at launch. In the completion handler, **ingest alerts
  before the `pineGeneration` check** so a superseded run cannot swallow a bar-close event (detached tasks aren't cancelled, so runs complete).
- `applyPineDraft` compares new source hash to subscriptions → `PineAlertCoordinator.sourceChanged(chartID:hash:)` (edit or removal ⇒ `.scriptChanged`).
- `ContentViewModel.removeTicker` → coordinator deletes that chart's subscriptions. Observe `.localScriptsDidChange` → subscriptions whose
  `scriptID` no longer exists in `ScriptStore` become `.scriptDeleted`.

### 3. Alert domain (app target only, `Model/` + `Service/` + `ViewModel/`)
- `Model/PineAlertSubscription.swift` — `id, chartID, scriptID?, scriptName, symbolKey, timeframe, sourceHash, note, state(active|paused|scriptChanged|scriptDeleted), createdAt`.
- `Model/PineAlertNotification.swift` — deliverable: subscriptionID, scriptID, chartID, symbol, timeframe, barTime, barIndex, message, frequency, triggeredAt, isConfirmed.
- `Service/PineAlertFrequencyGuard.swift` — pure struct keyed `subscription|site|barOpenMs|mode`; `all` always admits; `oncePerBar` admits first per key;
  `oncePerBarClose` requires `isConfirmed && isRealtime`, then first per key. Pruned to recent bars; seeded from the store's recent keys at launch.
- `Service/PineAlertRouter.swift` — the pure function above. Extra safeguard: drop events whose bar ended before `subscription.createdAt`.
- `Service/PineAlertDispatcher.swift` (actor) + `protocol PineAlertChannel`; `UserNotificationChannel` (respects `AlertNotificationSettings` from
  `AlertStore.shared.settings`; requests authorization when a first subscription is enabled; presented in foreground by the existing
  `AlertNotificationDelegate`) and `InAppBannerChannel`. Fan-out per channel in its own task; failures logged, never thrown into Pine or blocking each other.
- `ViewModel/PineAlertStore.swift` (`@MainActor ObservableObject`, `init(database: .shared)`) + `ViewModel/PineAlertCoordinator.swift`.

### 4. Persistence
- `AppDatabase+Schema.swift`: **new** `registerMigration("v2")` (never edit `v1`): document table `pine_alert_subscription` via
  `createDocumentTable`; `pine_alert_event(id PK, subscription_id, timestamp, dedupe_key TEXT UNIQUE, payload)` + index on
  `(subscription_id, timestamp)`. Add `DocumentTable.pineAlertSubscription`.
- **New** `Service/AppDatabase+PineAlerts.swift`: load/save subscriptions, `insertPineAlertEvent(_:dedupeKey:) -> Bool` (`INSERT OR IGNORE`, survives
  restarts), recent events, prune to newest 500. Separate tables from the price-alert snapshot, so `replaceSnapshot` is unaffected.

### 5. UI (extend, don't fork)
- `View/PineAlertEditor.swift` sheet, opened from a **"Create Alert…"** button in `ChartSettingsSheet.scriptsTab` beside the existing alert
  list. Shows read-only script / symbol / timeframe, the detected `alert()` call sites + frequencies, a note, enabled toggle. Disabled with a hint when the
  applied script has no `alert()` calls.
- `View/AlertsCenterView.swift`: add a "Script Alerts" filter/section — subscriptions with status chip (Active / Script changed / Script deleted / Chart shows other symbol),
  enable/re-arm toggle, delete, plus recent script-alert history. `GlobalAlertBanner` also renders Pine banners.

### 6. Project + docs
- `project.pbxproj`: four hand-edits per new file (PBXBuildFile, PBXFileReference, group children, Sources). `PineAlertFrequency` also gets an
  "Agent Sources" entry. New tests go in the `DegenViewTests` group + phase (mirror commit `e55edec`).
- Docs: `docs/pine-compatibility.md` (~`:107`, replace "They never notify"), `docs/usage.md` (`:33-34`, `:66-69`), `docs/architecture.md`
  (pipeline + tables), README feature line. Document: live-tail model, historical suppression, dedupe key, edit/delete/symbol/timeframe/removal behavior, limitations below.

## Behavior decisions to document
- **Symbol/timeframe change:** subscription is bound to its symbol+timeframe; it stays enabled but fires only while its chart shows that pair (UI shows "chart shows other symbol").
- **Script edit:** source-hash pinned; edit ⇒ `.scriptChanged`, silent until user re-arms (re-arm re-pins hash, clears guard keys). **Script deleted:** `.scriptDeleted`, disabled. **Script cleared / chart removed:** cleared ⇒ `.scriptChanged`; chart removed ⇒ subscription deleted.
- **Historical:** three layers — runtime flags `isRealtime=false` on all non-live bars; coordinator ignores runs with no live tail; router drops `!isRealtime`. Replay/backtest never passes a live tail.

## Known differences from TradingView (state plainly in docs/report)
- Client-side only: alerts need the app running with the chart's tab visible (occlusion pauses the stream) and a Binance source; no server worker. A bar close missed while disconnected is not alerted retroactively.
- Full re-run per tick: `varip` state and `barstate.isnew` are not carried across ticks; the live bar is always "new" within a run. `freq_all` fires per *live-tick* execution, not per REST refresh.
- Alpaca live bars don't re-evaluate Pine today (`applyLiveBar`), so no live alerts for stocks. No webhook/email/push. No user accounts.
- Can't verify TradingView parity beyond documented semantics.

## Tests (new `PineAlertTests.swift`, `PineAlertRoutingTests.swift`, plus migration case in `AppDatabaseTests`)
Language: parse/compile `alert` (positional + named), missing arg, wrong message type, bad/non-const freq, `alertcondition` arity, constants fold. Runtime: dynamic message
(`"Long " + syminfo.ticker + " @ " + str.tostring(close)`), each frequency, multiple calls/same bar, multiple call sites, `na` message.
Router/live (drive `evaluate(bars, liveTail:)` per simulated tick): required spec scenarios — 5,000-bar historical load ⇒ zero notifications; green closes once, red none, re-eval of closed bar no duplicate,
message has ticker + close; intrabar 99/101/102/103 ⇒ `once_per_bar` fires at update 2 only, `freq_all` fires each qualifying update, `once_per_bar_close` only at close; next bar re-arms; multiple scripts/symbols/timeframes don't suppress each other; script reload/edit ⇒ paused, re-arm; disabled vs enabled; guard seeded from store after "restart"; DB unique key backstop. Dispatcher: fake channels, one failing channel doesn't block or throw.

## Verification
1. `xcodebuild test -project DegenView.xcodeproj -scheme DegenView -destination 'platform=macOS'` — all suites incl. new.
2. `xcodebuild ... build` for **both** `DegenView` and `DegenViewAlertAgent` schemes (agent must still compile after `Pine/Model` changes).
3. `xcrun swift-format lint --strict` on every new/changed Swift file (including untracked).
4. Manual (AGENTS.md flow, note the debug Keychain-prompt hang in memory): Binance BTC 1m chart → apply the two spec scripts → Create Alert → confirm nothing fires on load, banner + macOS notification at bar close / first qualifying tick, edit script ⇒ "Script changed", quit/relaunch ⇒ subscription persists and no duplicate for the same bar.
5. Final report lists files, flow, frequency modes, historical suppression, dedupe, edit/delete behavior, channels, TradingView differences, and actual test/build/lint results.
