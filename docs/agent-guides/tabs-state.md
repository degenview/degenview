# Tabs, windows, and state guidance

## Ownership

- Each tab owns one `ChartWorkspace`; never treat it as app-global. `DegenViewApp` owns `TabSlot`s
  containing a workspace, portfolio tab, trade journal, or Pine Studio.
- Per-workspace live feeds and refreshes follow that workspace window's visibility/occlusion;
  hidden tabs suspend. Stores and portfolio/paper/alert/webhook/Pine hosts are shared `Arc<Mutex>`;
  presentation state is per tab/window.
- The Trade Journal is a fourth tab kind (`TabContent::Journal`, persisted as `kind: "journal"`, one
  instance). Its trades are shared app-wide in `JournalHost`; see [Trading journal](../trading-journal.md).
- Saved-layout dirty state is a `LayoutSnapshot` fingerprint, not a flag. It covers timeframe,
  chart configs, and columns—not zoom, drawings, replay, or market data. (An Autosave layout also
  writes on a drawing-only edit, via `ChartWorkspace::drawings_dirty`, so drawings reach iCloud
  Sync; the toolbar's unsaved state still ignores them.) `store::saved_view` owns
  the library; the workspace owns the active layout. Named layouts autosave after two seconds and
  flush before switch/quit; Unnamed never autosaves.
- A tab's open layout is saved with it (`PersistedWorkspace::active_view`, an id) and its name is
  looked up in the library on restore; a deleted layout restores as Unnamed. The restored tab's dirty
  baseline is the restored state, so changes left unsaved at quit show as clean after a restart.
- `ChartWorkspace::title()` (focused-chart title, then active-layout name, then “New tab”) names the
  OS window, the accessibility label and the Move Tab to Window rows. The strip itself paints a chart
  tab from `ChartWorkspace::tab_chart()` through `model::tab_title`: logo, symbol, `▲`/`▼`, price and
  change, then `/ <layout name>`. The market is the **focused chart's** (first chart when none is
  focused; a portfolio card gives none) and the layout is `active_view_name`, so the layout name
  follows the layout, not the symbol; an unnamed layout has no `/` segment. Session persistence is
  debounced ten seconds and also runs on save/quit.
- Tab prices come from the shared `WatchlistQuoteBook` (the provider's own reference: rolling 24h or
  previous close), never from the chart's candles, and a missing price or reference is left out, not
  shown as zero. `DegenViewApp::tick_tab_quotes` asks for them with one non-lapsing demand owner
  (`TAB_QUOTE_OWNER`, `1 << 60`) covering every open chart tab in every window, only when the set
  changed; the coordinator merges demand by market, so a market a sidebar or alert follows costs
  nothing extra. Prediction markets and CoinMarketCap are not asked for (`tab_title::wants_quote`).
  The coordinator repaints only the root viewport, so the same tick repaints every tab window when
  `quote_book.revision()` moves.
- A tab's width is uniform in a strip (reorder math relies on it), at most `MAX_TAB_WIDTH` (300), and the amount shown is chosen from
  that width alone (`tab_title::Detail`), never from the live text, so a tick cannot move anything;
  the `/ layout name` (from `Detail::LAYOUT_WIDTH`, 260) is cut off where the tab ends with a fade into the tab's fill, not an ellipsis;
  the price block also keeps the widest it has been (egui temp memory). At the narrowest
  (`MIN_TAB_WIDTH`) a tab is its logo. With more tabs than fit at the narrowest the row scrolls
  (`tab_bar::max_scroll`): wheel or trackpad over the strip, the active tab eased into view when it
  or the tab count changes, and a dragged tab scrolls it near either end; the `+` stays pinned at
  the strip's end. The scroll offset (egui temp memory) is published each frame with the geometry
  (`TabBarOutput::scroll`, `publish_geometry`) so the host's reorder and drop math reads slots at
  `pointer.x - strip.left() + scroll`; the strip draws the dragged tab at `left - scroll`.
- A tab can have its own title (`PersistedWorkspace::custom_title`, set by Rename Tab… in its context
  menu): it stands in for the symbol and price, and a symbol or layout change never touches it. The
  app-wide style (symbol+price+layout, symbol only, layout name only) is `tab.title_style`
  (`host::tab_title_host`, Settings ▸ Appearance ▸ Tabs). Both are device-local: the `tab` table and
  that key never sync. A hairline (`theme::separator()`, `paint_separators`) divides two neighbouring tabs that are both at rest: not the showing tab, not the one dragged or under the pointer, not across a gap left for an incoming tab. It follows the animated tab rects. A tab has **no underline by default**, showing or not (the blue
  selection line is gone; the showing tab is the one joined to the content). A tab tagged with a Tab
  Linking colour (`PersistedWorkspace::link`, device-local; see [Tab Linking](../tab-linking.md)) has a
  line under it in that colour, showing or not: that is data, not a control or the accent.
  The tab's right-click menu ends, after a separator, with the Link Tabs section (colour swatches
  and the group's four switches, shown once a colour is chosen; `TabBarEvent::SetLink` / `SetLinkToggles`); choosing the chosen
  colour again unlinks. There is no toolbar button for it.

## Windows and tabs

- Tabs are app-owned on every OS; native automatic tabbing is off. Each deferred viewport renders
  one `TabWindow` with `ui::tab_bar` as title bar. The coordinator window stays hidden.
- macOS uses transparent full-size content with a traffic-light `leading_inset` (zero fullscreen).
  Windows/Linux are undecorated and reserve `trailing_inset` for controls; Linux draws resize edges.
  `TabBarOutput::strip` excludes both insets. Empty strip space drags; double-click maximizes. The
  strip is `tab_bar::HEIGHT` (40); recenter macOS traffic lights in it each frame. Settings/Alerts
  retain native title bars.
- Linux tab windows round their own corners (`ui::window_corners`, macOS has AppKit's, Windows is not
  done: it needs DWM's corner preference, not this). egui can only blend, so the window is
  transparent (`window_chrome::rounded_corners` asks for it on the root viewport, which sets the
  shared surface's alpha mode, and on each tab window; `App::clear_color` is clear) and the last
  thing `show_window` draws is a paint callback that erases the area outside a corner arc
  (`dst * (1 - a)`, antialiased). It is decided once at startup: a Wayland session, or X11 with a
  compositor owning `_NET_WM_CM_S<n>`; otherwise the windows stay square rather than showing black
  corners. Maximized and fullscreen windows are square. Not handled: a tiling compositor (winit
  0.30 reports no tiled state), a compositor that starts after the app.
- macOS tab windows are set non-movable each frame next to the traffic lights. AppKit otherwise
  drags the window from its title-bar band (the top ~28 pt of the strip) before egui sees the
  press, so tabs could not be reordered or torn off from there. The window still moves through
  `StartDrag` (empty strip) and `OuterPosition` (a torn-off tab).
- Window callbacks must look up active content live through `TabWindowHost`/`TabRegistry`, under
  `ui.push_id(tab_id)`. Never capture content or key tab state by viewport.
- Select/reorder/move/detach mutate `TabWindows` immediately. New and Close are `TabRequest`s handled
  by `DegenViewApp::process_commands`. OS close becomes `CloseWindow`; Windows/Linux quit after the
  last window.

## Dragging and restore

- `TabWindowHost::drive_drag` runs from every window. Reorder when the dragged center crosses a
  neighbor; outside the tear-off zone detach/follow the OS cursor. Over another strip, show an
  incoming gap and hide the floating window; release moves the tab.
- Hidden windows run no frames, so macOS/Windows dragging uses the global OS cursor/button and may
  be driven by any window. Never derive the cursor from the moving window. X11 gets the same handoff
  from `XQueryPointer` (`native_bridge::x11_pointer`, one cached connection, button 1 from the
  query's mask). Wayland has neither the cursor nor window positions: a lone tab uses OS drag, a
  tab cannot be dropped on another window, and merging needs **Move Tab to Window**.
- macOS menus emit `NativeCommand`s. Windows/Linux use `ui::shortcut` for tab creation/closing and
  all platforms support next/previous tab actions. Windows/Linux have no menu bar, so Settings opens
  with Ctrl+, or the strip's settings button (left of the window buttons); both become
  `TabRequest::OpenSettings`. macOS keeps Settings… in its menu and draws no button.
  The project's GitHub and Buy Me a Coffee links (`tab_bar::GITHUB_URL`/`COFFEE_URL`) are gray
  icons at the strip's far right on every platform, left of the window buttons, and also in
  macOS's Help menu. The user guide (`tab_bar::DOCS_URL`) is the first Help item on macOS and a
  Documentation row in Settings ▸ About on every platform.
- The first window opens at 80% of the width and 90% of the height of the main screen's usable area
  (`NSScreen.visibleFrame`, `native_bridge::main_work_area`), at least 960x620, at most 2000x1300, never bigger than the area,
  centered (`Placement::for_work_area`). Windows and Linux have no screen lookup and open at the
  fixed `Placement::DEFAULT_SIZE`. A torn-off window keeps its source's size. Window frames are not
  saved across restarts. The share and limits are a taste choice, not measured.
- Sleep and display changes: macOS moves and clamps windows when displays drop out on sleep and
  does not put them all back. `model::window_frames::FrameMemory` saves each window's frame per
  display arrangement (`native_bridge::screen_layout`, macOS only; empty elsewhere, which turns it
  off), only after the arrangement has been steady, so a frame the OS forced is never saved. When
  an arrangement with saved frames returns, or a gap in the 250 ms root tick shows the Mac slept,
  `TabWindowHost::restore_frames` (from `DegenViewApp::logic`) sends `OuterPosition`/`InnerSize` to
  windows that differ by more than 1 pt, for up to 6 s. Minimized, maximized and fullscreen windows
  are skipped, as is any tick during a tab drag. The settle times are inferred, not measured, and
  this has not been run on real multi-display sleep. Frames are not saved across restarts.
- Window grouping is not persisted. Restore sorts tabs by old window and strip position into one
  window. `PersistedTab` is untagged so old chart rows remain compatible; special tabs require
  `kind`.
- New/first-launch tabs are empty (`ChartWorkspace::empty`); only opening a watchlist market seeds a
  chart. Cache empty-page database/Keychain reads per visit, not per frame.
- Snapshot choices (`snapshot.prefs` in the `setting` table: host, trading overlays, scale, approved
  hosts) are app-wide, held by `snapshot_host::SnapshotHost` and shared by every window and Settings.
  A snapshot in flight belongs to its workspace; closing the tab drops its result (an upload
  already sent cannot be recalled). The token is in the Keychain.
