# Tabs, windows, and state guidance

## Ownership

- Each tab owns one `ChartWorkspace`, never app-global. `DegenViewApp` owns `TabSlot`s holding a
  workspace, portfolio tab, or Pine Studio.
- Live feeds/refreshes follow the workspace window's visibility/occlusion; hidden tabs suspend.
  Stores and portfolio/paper/alert/webhook/Pine hosts are shared `Arc<Mutex>`; presentation state is
  per tab/window.
- Layout dirty state is a `LayoutSnapshot` fingerprint, not a flag: timeframe, chart configs,
  columns; not zoom, drawings, replay, market data. Autosave layouts also write on drawing-only
  edits (`ChartWorkspace::drawings_dirty`) so drawings reach iCloud Sync; the toolbar's unsaved
  state ignores them. `store::chart::saved_view` owns the library, the workspace the active layout. Named
  layouts autosave after two seconds and flush before switch/quit; Unnamed never autosaves.
- A tab saves its layout id (`PersistedWorkspace::active_view`); the name is looked up on restore,
  a deleted layout restores as Unnamed. The dirty baseline is the restored state, so changes
  unsaved at quit show clean after restart.
- `ChartWorkspace::title()` (focused-chart title, else layout name, else “New tab”) names the OS
  window, accessibility label and Move Tab to Window rows. The strip paints a chart tab from
  `ChartWorkspace::tab_chart()` via `model::tab::title`: logo, symbol, `▲`/`▼`, price and change,
  `/ <layout name>`. Market: the **focused chart's** (first chart if none; a portfolio card gives
  none). Layout: `active_view_name`, following the layout, not the symbol; unnamed has no `/`
  segment. Session persistence: debounced ten seconds, plus on save/quit.
- Tab prices come from the shared `WatchlistQuoteBook` (provider's reference: rolling 24h or
  previous close), never chart candles; a missing price or reference is omitted, not zero.
  `DegenViewApp::tick_tab_quotes` requests them, only when the set changed, with one non-lapsing
  demand owner (`TAB_QUOTE_OWNER`, `1 << 60`) for every open chart tab in every window; demand
  merges by market, so markets a sidebar or alert follows cost nothing. Prediction markets and
  CoinMarketCap are skipped (`tab_title::wants_quote`). The coordinator repaints only the root
  viewport, so that tick repaints every tab window when `quote_book.revision()` moves.
- Tab width: uniform per strip (reorder math relies on it), at most `MAX_TAB_WIDTH` (300). Detail
  is chosen from width alone (`tab_title::Detail`), never live text, so ticks move nothing.
  `/ layout name` (from `Detail::LAYOUT_WIDTH`, 260) is cut at the tab's end with a fade into its
  fill, no ellipsis; the price block keeps its widest width (egui temp memory). At `MIN_TAB_WIDTH`
  a tab is its logo.
- Overflow at the narrowest scrolls the row (`tab_bar::max_scroll`): wheel/trackpad over the strip;
  the active tab eases into view when it or the count changes; dragging near either end scrolls;
  `+` stays pinned at the end. The offset (egui temp memory) is published per frame with the
  geometry (`TabBarOutput::scroll`, `publish_geometry`); host reorder/drop math reads slots at
  `pointer.x - strip.left() + scroll`, the strip draws the dragged tab at `left - scroll`.
- Custom title (`PersistedWorkspace::custom_title`, context menu Rename Tab…) replaces symbol and
  price; symbol/layout changes never touch it. App-wide style (symbol+price+layout, symbol only,
  layout name only): `tab.title_style` (`host::tab::title`, Settings ▸ Appearance ▸ Tabs). Both
  device-local: the `tab` table and that key never sync.
- Hairline (`theme::separator()`, `paint_separators`) only between two neighbours at rest: not the
  showing, dragged or hovered tab, not across an incoming-tab gap. It follows animated tab rects.
- **No underline by default**, showing or not (no blue selection line; the showing tab joins the
  content). A Tab Linking colour (`PersistedWorkspace::link`, device-local; see
  [Tab Linking](../tab-linking.md)) underlines the tab in that colour, showing or not: data, not a
  control or the accent. The right-click menu ends, after a separator, with Link Tabs (swatches,
  plus the group's four switches once a colour is chosen; `TabBarEvent::SetLink` /
  `SetLinkToggles`); re-picking the chosen colour unlinks. No toolbar button.

## Windows and tabs

- Tabs are app-owned on every OS; native automatic tabbing off. Each deferred viewport renders one
  `TabWindow` with `ui::window::tab_bar` as title bar. The coordinator window stays hidden.
- macOS: transparent full-size content, traffic-light `leading_inset` (zero fullscreen).
  Windows/Linux: undecorated, `trailing_inset` for controls; Linux draws resize edges.
  `TabBarOutput::strip` excludes both. Empty strip drags; double-click maximizes. Strip height
  `tab_bar::HEIGHT` (40); recenter macOS traffic lights in it each frame. Settings/Alerts keep
  native title bars.
- Linux tab windows round their own corners (`ui::window::corners`; macOS uses AppKit's; Windows not
  done, needs DWM's corner preference). egui only blends, so windows are transparent
  (`window_chrome::rounded_corners` on the root viewport, setting the shared surface's alpha mode,
  and each tab window; `App::clear_color` clear) and `show_window` finishes with a paint callback
  erasing outside a corner arc (`dst * (1 - a)`, antialiased). Decided at startup: Wayland, or X11
  with a compositor owning `_NET_WM_CM_S<n>`; else square, not black corners. Maximized/fullscreen
  square. Not handled: tiling compositors (winit 0.30 reports no tiled state), a compositor started
  after the app.
- macOS tab windows are set non-movable each frame next to the traffic lights, else AppKit drags
  from the title-bar band (top ~28 pt of the strip) before egui sees the press, blocking reorder
  and tear-off. Windows still move via `StartDrag` (empty strip) and `OuterPosition` (torn-off tab).
- Window callbacks look up content live via `TabWindowHost`/`TabRegistry`, under
  `ui.push_id(tab_id)`. Never capture content or key tab state by viewport.
- Select/reorder/move/detach mutate `TabWindows` immediately. New and Close are `TabRequest`s for
  `DegenViewApp::process_commands`. OS close becomes `CloseWindow`; Windows/Linux quit after the
  last window.

## Dragging and restore

- `TabWindowHost::drive_drag` runs from every window. Reorder when the dragged center crosses a
  neighbor; past the tear-off zone, detach and follow the OS cursor. Over another strip: incoming
  gap, floating window hidden; release moves the tab.
- Hidden windows run no frames, so macOS/Windows drags use the global OS cursor/button and any window
  may drive them; never derive the cursor from the moving window. X11: same via `XQueryPointer`
  (`native_bridge::x11_pointer`, one cached connection, button 1 from the mask). Wayland has no
  cursor or window positions: a lone tab uses OS drag, no drop onto another window, merge via
  **Move Tab to Window**.
- macOS menus emit `NativeCommand`s. Windows/Linux: `ui::shortcut` for tab create/close; next/previous
  tab everywhere. No menu bar there, so Settings is Ctrl+, or the strip's settings button (left of
  the window buttons), both `TabRequest::OpenSettings`. macOS: Settings… in its menu, no button.
- GitHub/Buy Me a Coffee (`tab_bar::GITHUB_URL`/`COFFEE_URL`): gray icons at the strip's far right,
  left of the window buttons, every platform; also in macOS's Help menu. User guide
  (`tab_bar::DOCS_URL`): first Help item on macOS, Documentation row in Settings ▸ About everywhere.
- First window: 80% width, 90% height of the main screen's usable area (`NSScreen.visibleFrame`,
  `native_bridge::main_work_area`), min 960x620, max 2000x1300, never bigger than the area,
  centered (`Placement::for_work_area`). Windows/Linux lack screen lookup: fixed
  `Placement::DEFAULT_SIZE`. Torn-off windows keep the source's size. Share and limits are taste,
  not measured. Frames are not saved across restarts.
- Sleep/display changes: macOS moves/clamps windows when displays drop out and does not restore
  them all. `model::tab::window_frames::FrameMemory` saves each window's frame per display
  arrangement (`native_bridge::screen_layout`, macOS only; empty elsewhere = off), only once the
  arrangement is steady, so OS-forced frames are never saved. When a saved arrangement returns, or a
  gap in the 250 ms root tick shows the Mac slept, `TabWindowHost::restore_frames` (from
  `DegenViewApp::logic`) sends `OuterPosition`/`InnerSize` to windows off by more than 1 pt, for up
  to 6 s; skips minimized/maximized/fullscreen windows and ticks during a tab drag. Settle times
  inferred, not measured; never run on real multi-display sleep.
- Window grouping is not persisted: restore puts all tabs in one window, sorted by old window and
  strip position. `PersistedTab` is untagged so old chart rows stay compatible; special tabs need
  `kind`.
- New/first-launch tabs are empty (`ChartWorkspace::empty`); only opening a watchlist market seeds a
  chart. Cache empty-page database/Keychain reads per visit, not per frame.
- Snapshot choices (`snapshot.prefs` in the `setting` table: host, trading overlays, scale, approved
  hosts) are app-wide in `host::snapshot::SnapshotHost`, shared by all windows and Settings. An
  in-flight snapshot belongs to its workspace; closing the tab drops its result (a sent upload
  cannot be recalled). Token in the Keychain.
