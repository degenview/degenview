# iPhone and iPad Release Plan

## Summary

Add a new universal iOS application target, `DegenViewMobile`, supporting both iPhone and iPad while retaining the existing `DegenView` macOS target. Both targets will share the same models, market-data services, calculations, stores, view models, chart renderer, and most SwiftUI views. Platform-specific app lifecycle, window management, input handling, file dialogs, image conversion, and editor implementations will remain separate.

A separate mobile target is preferable to converting the existing target into one multiplatform target because the app contains substantial AppKit lifecycle and event-monitoring behavior, a macOS login-item target, native window tabs, and desktop-specific UI. Shared source membership still provides one codebase without filling the code with platform conditionals.

Use:

- Product name: `DegenView`
- Target and scheme name: `DegenViewMobile`
- Bundle identifier: `com.cryptocharts.app`
- Deployment target: iOS/iPadOS 18.0
- Targeted device families: iPhone and iPad
- SwiftUI lifecycle and Swift 5
- Existing asset catalog initially, extended with required iOS icon variants
- The same App Store Connect record as macOS for universal purchase

Mobile data will remain local to each device for the first release. No CloudKit synchronization or cross-device migration is included.

Mobile price alerts will run only while the relevant scene is active. They must not claim background or always-on delivery. iOS background refresh is system-scheduled and cannot replace the macOS login-item evaluator.

## Target and Source Organization

### Xcode targets

Keep these targets:

- `DegenView`: existing macOS app.
- `DegenViewAlertAgent`: existing macOS-only login item.
- `DegenViewTests`: existing macOS unit tests.

Add:

- `DegenViewMobile`: universal iOS/iPadOS application.
- `DegenViewMobileTests`: tests for shared code and mobile adapters.
- `DegenViewMobileUITests`: focused navigation, chart gesture, persistence, and adaptive-layout tests.

The mobile app must not depend on or embed `DegenViewAlertAgent`.

Because the project uses ordinary PBX groups rather than synchronized folder groups, every new Swift file must be added manually to the file reference, group, build-file, and source-phase sections of `project.pbxproj`.

### Source layers

Organize source membership into three logical layers:

1. Shared domain and infrastructure, compiled by both app targets:

   - All models.
   - API and WebSocket data sources.
   - Indicators, chart plotting geometry, replay, Pine engine/runtime.
   - Portfolio and paper-trading accounting.
   - Codable persistence and caches.
   - Shared view models.
   - SwiftUI views that contain no AppKit/UIKit dependency.
   - Renderer-neutral chart styles and drawing models.

2. macOS-only sources:

   - macOS app entry and `NSApplicationDelegate`.
   - `WindowCoordinator`, `WindowAccessor`, and window-frame restoration.
   - Native tab grouping and restoration.
   - AppKit chart event monitoring.
   - macOS line-numbered editor implementation.
   - Login-item alert service and agent integration.
   - macOS-only file panels, clipboard, workspace links, and menu commands.

3. iOS-only sources:

   - Mobile app entry and scene root.
   - Adaptive workspace navigation.
   - Touch and pointer chart interaction layer.
   - iOS line-numbered editor implementation.
   - Foreground alert runtime.
   - Mobile settings and document import/export adapters.

Do not extract a Swift package in the first migration. Direct shared target membership minimizes module/API churn while the boundaries are still being established. Reassess an internal package after both targets build cleanly and the shared API has stabilized.

## Required Refactoring

### 1. Separate workspaces from macOS window state

The current `ChartTab`, `TabsStore`, and `ContentViewModel` mix reusable dashboard state with `NSWindow.tabGroup` restoration.

Refactor them into:

- `Workspace`: the platform-neutral replacement or semantic rename for `ChartTab`, retaining the existing UUID, name, saved-view reference, ticker configurations, timeframe, layout, candle count, replay state, and chart/portfolio kind.
- `WorkspaceStore`: owns ordered workspaces, creation, deletion, updates, saved-view hydration, and debounced persistence.
- `MacWindowSessionStore`: owns only macOS window groups, ordering, detached-window state, and window restoration metadata.
- `WorkspaceSceneModel`: owns the active workspace within one SwiftUI scene.

Preserve existing user data:

- Decode the current `tabs.json` and `TabsSnapshot`.
- Migrate its `tabs` to the new workspace snapshot.
- Migrate `windowGroups` to the macOS-only session snapshot.
- Leave the original file intact until both new files have been written successfully.
- Make migration idempotent and cover missing, malformed, partially migrated, and duplicate-ID cases.
- Mobile starts with its own empty local store and does not attempt to access the Mac’s container.

The invariant remains one workspace view model per scene. No workspace state becomes an app-global view model.

### 2. Split domain state from AppKit event handling

`ContentViewModel` currently owns reusable chart state as well as `NSEvent`, `NSView`, `NSWindow`, hit-region tables, occlusion observers, and pointer-coordinate conversion.

Retain reusable responsibilities in a platform-neutral `WorkspaceViewModel`:

- Chart creation, removal, replacement, and reordering.
- Timeframe, layout, and candle-count state.
- Fetching, refresh, WebSocket subscription, and CoinGecko symbol synchronization.
- Replay lifecycle.
- Active drawing tool and crosshair/ruler/trend-line state.
- Saved-view load/save and unsaved-change tracking.
- Workspace persistence.
- Explicit active/inactive lifecycle.

Expose semantic interaction methods rather than native events:

- `setSceneActive(_:)`
- `zoomCandleCount(by:)`
- `zoomPriceScale(chartID:translation:)`
- `selectReplayBar(chartID:location:)`
- `begin/update/endDrawing(chartID:location:)`
- `begin/update/endRuler(chartID:location:)`
- `updateCrosshair(chartID:location:)`
- `cancelActiveInteraction()`

Move all `NSEvent`, `NSView`, `NSWindow`, hit testing, and coordinate conversion into a macOS-only `MacChartInteractionController`.

On mobile, implement the same semantic calls using SwiftUI gestures. `ChartPlot` remains the authority for converting view coordinates into candle indexes and prices.

Use `scenePhase` to start refresh timers and WebSockets only while a scene is active and to suspend them when inactive.

### 3. Make chart interaction input-independent

Keep the current SwiftUI `Canvas`, `ChartPlot`, axes, indicators, candles, price overlays, Pine visuals, and line-chart drawing code shared.

Add a shared chart interaction overlay with these mobile mappings:

- Pinch horizontally: change visible candle count.
- Vertical drag starting in the Y-axis gutter: zoom the price scale.
- Tap while replay selection is armed: select the replay bar.
- Drag while trend-line or ruler mode is armed: create or adjust the active drawing.
- Single-finger movement while crosshair mode is armed: move the crosshair.
- Tap an existing drawing: select it and reveal delete/edit actions.
- Cancel button or switching tool/timeframe: clear draft interactions exactly as on macOS.
- Apple Pencil follows the same drawing gestures on iPad.
- Trackpad and mouse input on iPad use the same SwiftUI gestures; hover enhancement is optional and must not be required for functionality.

Tool mode must disambiguate chart dragging from scrolling. While a drawing, ruler, replay-selection, or crosshair tool is armed, the chart consumes the relevant gesture; otherwise, the surrounding dashboard scroll view wins.

### 4. Replace platform image dependencies

`ImageCache` and `TickerIconView` currently expose `NSImage`.

Refactor `ImageCache` to cache and return validated image `Data` rather than an AppKit image. Add small platform adapters that convert the data to `NSImage` or `UIImage` at the view boundary.

Preserve:

- Memory and disk caching.
- Existing cache keys and TTL behavior.
- Immediate cache hits for reused rows.
- Monogram fallback and fixed icon sizing.
- Icon resolution keyed by `ChartViewModel.iconKey`.

### 5. Extract platform services

Introduce narrow abstractions for the remaining native operations:

- `AppActivityObserving`: scene activation/wake handling.
- `ExternalURLOpening`: implemented with SwiftUI `openURL` on mobile and the existing macOS behavior.
- `ClipboardWriting`: `NSPasteboard` on macOS and `UIPasteboard` on mobile.
- `DocumentTransferService`: SwiftUI `fileImporter`/`fileExporter` on mobile; migrate the paper-trading CSV export away from direct `NSOpenPanel`.
- `NotificationDelivery`: shared authorization and notification construction with platform-specific settings labels.
- `SecureCredentialsStore`: shared Security-framework implementation with platform-appropriate accessibility attributes.

Keep the current shared Keychain access-group identifier where signing supports it, but do not enable synchronizable Keychain items. Credentials remain local per device.

Refactor `ColorHex` to use a platform-neutral RGBA representation with thin `NSColor`/`UIColor` conversion adapters.

### 6. Provide platform-specific source editors

Keep Pine parsing, compilation, runtime, diagnostics, scripts, and editor view models shared.

Split the native editor into:

- macOS `NSViewRepresentable` backed by `NSTextView`.
- iOS `UIViewRepresentable` backed by `UITextView`.

The iOS implementation must retain:

- Monospaced editing.
- Line numbers.
- Syntax highlighting.
- Inline diagnostic ranges and messages.
- Save/apply/revert behavior.
- Keyboard shortcuts when a hardware keyboard is attached.
- Source selection that does not lose unsaved edits during rotation or scene changes.

## Mobile Application Design

### Scene and workspace navigation

Use a SwiftUI `WindowGroup` with value-based workspace IDs and enable iPad multiple-scene support.

iPhone:

- Launch into a workspace browser when multiple workspaces exist; open the sole workspace directly when only one exists.
- Use `NavigationStack` for workspace, portfolio, alerts, scripts, and settings navigation.
- Present search, chart settings, orders, and editors as sheets or full-screen covers according to available height.
- Do not build a custom desktop-style tab strip.

iPad:

- Use `NavigationSplitView` with workspaces, favorites, portfolio, alerts, scripts, and settings in the sidebar.
- Open a workspace in a separate native scene through `openWindow(value:)`.
- Each window receives its own `WorkspaceSceneModel` and activity lifecycle.
- Persist workspace ordering, but allow iPadOS to manage placement and scene layout rather than reproducing `NSWindow.tabGroup`.

### Dashboard layout

Preserve vertical and grid layout choices adaptively:

- Compact-width portrait: one chart column; grid mode may become a tighter single-column presentation.
- Compact-width landscape: grid may use two columns when each chart remains readable.
- Regular width: vertical mode uses one column; grid mode uses two adaptive columns.
- Chart cards use content-driven minimum heights instead of macOS window minimum sizes.
- Reordering uses drag-and-drop where practical and an explicit Edit/Reorder mode as the accessible fallback.
- Favorites appear in the iPad sidebar and as a dedicated destination or drawer on iPhone.
- The macOS tool sidebar becomes a horizontal chart-tool bar above the chart list or in the bottom safe area.

### Feature mapping

All current feature domains remain available:

- Binance, CoinGecko, DEXScreener/GeckoTerminal, Alpaca, and Polymarket search and data.
- Candles, line charts, logarithmic scale, timeframes, indicators, styles, and price formatting.
- Candle-count zoom, price-scale zoom, crosshair, trend lines, ruler, and replay.
- Saved views, named workspaces, favorites, and card reordering.
- Portfolio dashboard, transactions, accounting, remapping, CSV import, and CSV export.
- Paper accounts, positions, orders, chart trading overlay, journal, and CSV export.
- Pine script manager, editor, inputs, compilation, diagnostics, and chart output.
- Alpaca credential entry through Keychain.
- Local caches and app settings.
- Price-alert rule editing, history, health display, in-app banners, and foreground evaluation.

Adaptive presentations:

- Paper-trading manager: resizable lower pane on iPad; full-screen sheet with tabs on iPhone.
- Portfolio tables: multi-column table on regular-width iPad; card/list rows with drill-down detail on iPhone.
- Script manager/editor: split view on iPad; navigation destinations on iPhone.
- Chart settings and ticker search: sheet on iPad; full-screen navigation presentation on compact iPhone.
- Replay controls: horizontal bar where space permits; two-row compact controls on narrow screens.

### Foreground-only mobile alerts

Reuse the shared alert models, quote coordinator, crossing engine, persistence, history, and notification content.

Add `ForegroundAlertRuntime` for iOS/iPadOS:

- Start quote subscriptions when at least one scene is active.
- Stop subscriptions when all scenes are inactive.
- Prevent multiple active scenes from evaluating the same rules twice by keeping one process-wide runtime owner.
- Deliver an in-app banner and optionally a local notification while active.
- Reconcile quote gaps when the app returns to the foreground, but do not retroactively claim a timely notification.
- Replace macOS login-item controls with the label “Alerts run while DegenView is open.”
- Hide registration, login-item health, retry-registration, and login-item system-settings actions.
- Preserve history and rule state across launches.
- Retain macOS background behavior unchanged.

Rename `macOSNotificationsEnabled` to a platform-neutral `localNotificationsEnabled` while decoding the previous persisted key for backward compatibility.

## Implementation Sequence

### Phase 1: Establish boundaries without changing macOS behavior

- Add the workspace/session split and persistence migration.
- Extract native service protocols and platform-neutral image/color representations.
- Separate `WorkspaceViewModel` from `MacChartInteractionController`.
- Move macOS-only files into explicit target membership.
- Keep the existing macOS UI and behaviors visually unchanged.
- Run the complete macOS unit and manual window/tab test suites before adding mobile UI.

Exit criterion: the macOS app builds and all current tests pass with no feature or persistence regression.

### Phase 2: Add the universal mobile target

- Add `DegenViewMobile`, its scheme, generated Info settings, iOS entitlements, assets, and test targets.
- Enable iPhone and iPad device families and iPad multiple scenes.
- Add shared source files to the mobile target and exclude macOS files.
- Add a minimal mobile app entry, workspace browser, scene lifecycle, settings route, and empty state.
- Resolve all shared-code availability and concurrency errors before implementing feature screens.

Exit criterion: an empty mobile app builds for generic iOS device and iOS Simulator, launches on both form factors, and persists a blank workspace.

### Phase 3: Deliver charts and market workflows

- Add adaptive dashboard, ticker search, favorites, chart settings, and chart card controls.
- Add mobile chart gestures and drawing/replay tools.
- Verify every data source through REST and supported WebSocket paths.
- Implement scene-phase subscription suspension and reconnection.
- Add rotation, Split View, Stage Manager, Dynamic Type, and hardware keyboard handling.

Exit criterion: users can create, configure, save, restore, reorder, and interact with complete chart workspaces on iPhone and iPad.

### Phase 4: Port advanced feature surfaces

- Port portfolio and CSV workflows.
- Port paper trading and chart overlays.
- Port Pine manager/editor and diagnostics.
- Port alerts using the foreground runtime.
- Complete mobile settings, Keychain credentials, document transfer, and external links.

Exit criterion: every feature listed in the mapping has a reachable mobile workflow and platform-appropriate presentation.

### Phase 5: Release hardening

- Add complete iOS/iPadOS app icons, launch appearance, privacy descriptions, and App Store metadata.
- Profile chart rendering, memory use, image caching, and many-workspace restoration on physical devices.
- Test poor connectivity, WebSocket interruption, API throttling, background/foreground transitions, and low-memory relaunch.
- Run TestFlight on representative iPhone and iPad sizes.
- Add the iOS platform to the existing App Store Connect app record and validate universal-purchase configuration.
- Release mobile only after macOS and mobile build/test pipelines pass from the same commit.

## Public Interfaces and Compatibility

The refactor should introduce or stabilize these shared interfaces:

- `Workspace` and `WorkspaceStore` for platform-neutral dashboard state.
- `WorkspaceSceneModel` for per-scene ownership.
- Semantic chart interaction methods independent of `NSEvent` or UIKit events.
- `setSceneActive(_:)` as the only entry point controlling refresh and WebSocket lifecycle.
- `ImageCache` APIs returning image data instead of `NSImage`.
- Platform service protocols for clipboard, URL opening, document transfer, activity observation, notifications, and credentials.
- `ForegroundAlertRuntime` conforming to the same runtime control interface as the macOS alert host.
- Platform-neutral `localNotificationsEnabled` settings with backward decoding of `macOSNotificationsEnabled`.

Compatibility rules:

- Existing ticker, saved-view, drawing, portfolio, paper-trading, alert, script, and cache Codable formats remain readable.
- Workspace migration must preserve every existing macOS tab and window group.
- Platform-only presentation state must not enter shared domain models.
- Mobile storage is local and independent; no CloudKit containers or synchronizable Keychain items are introduced.
- API clients must continue using the existing source protocols, caches, and CoinGecko rate limiter.
- macOS native tab behavior and the `newWindowForTab(_:)` responder method remain untouched.

## Test Plan

### Shared unit tests

Run existing tests against the refactored shared implementation and add coverage for:

- Workspace migration from every existing `TabsSnapshot` form.
- Idempotent migration and corrupt-file quarantine.
- Semantic chart interactions producing the same candle counts, prices, drawings, rulers, and replay selections as desktop input.
- Scene activation starting and stopping timers and streams exactly once.
- Multi-scene CoinGecko active-symbol union behavior.
- Foreground alert ownership across multiple iPad scenes.
- Image-data cache hits, failures, persistence, and monogram fallback.
- Backward decoding of alert settings.
- Platform-neutral CSV import/export data generation.

### Mobile UI tests

Cover:

- First launch and blank workspace creation.
- Add, edit, remove, and reorder tickers from every source.
- Save/load/rename workspaces and unsaved-change indication.
- Vertical/grid layout across portrait, landscape, iPad Split View, and Stage Manager sizes.
- Pinch candle zoom and Y-axis price zoom.
- Crosshair, trend line, ruler, replay selection, and cancellation behavior.
- Portfolio import/export and transaction editing.
- Paper order placement and chart overlay updates.
- Pine script editing, diagnostics, applying, and persistence.
- Foreground alert trigger, banner, history, pause/resume, and deletion.
- Scene backgrounding suspending traffic and foregrounding reconnecting once.
- Multiple iPad windows maintaining independent workspace settings.
- VoiceOver labels, Dynamic Type, keyboard navigation, and reduced-motion behavior.

### Build validation

Required commands after implementation:

```bash
xcodebuild test \
  -project DegenView.xcodeproj \
  -scheme DegenView \
  -destination 'platform=macOS'

xcodebuild build \
  -project DegenView.xcodeproj \
  -scheme DegenViewMobile \
  -destination 'generic/platform=iOS'

xcodebuild test \
  -project DegenView.xcodeproj \
  -scheme DegenViewMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro'

xcodebuild test \
  -project DegenView.xcodeproj \
  -scheme DegenViewMobile \
  -destination 'platform=iOS Simulator,name=iPad Pro 13-inch (M5)'
```

Also run `git diff --check` and confirm the project file opens without missing references or duplicate build entries.

## Acceptance Criteria and Assumptions

- “Same codebase” means one Xcode project and shared Swift source files, not duplicated mobile copies.
- “All functionality” means feature parity at the domain level with adaptive mobile presentations.
- Exact `NSWindow` tabbing, drag-out/merge behavior, menu commands, hover-only affordances, and login-item background execution are platform-specific and are not reproduced on iPhone.
- iPad receives native multi-window workspace support; iPhone receives workspace navigation inside its main scene.
- Mobile alerts are explicitly foreground-only.
- Persistence is local-first; cross-device synchronization is deferred.
- iOS/iPadOS 18.0 is the minimum version, matching the project’s existing iPhone deployment setting.
- The macOS target remains independently shippable throughout the migration.
- Existing uncommitted repository changes must be preserved and reconciled rather than overwritten during implementation.
