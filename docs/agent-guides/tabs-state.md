# Tabs, windows, and state guidance

## Ownership

- Each tab owns one `ChartWorkspace`; never treat it as app-global. `DegenViewApp` owns `TabSlot`s
  containing a workspace, portfolio tab, or Script Manager.
- Per-workspace live feeds and refreshes follow that workspace window's visibility/occlusion;
  hidden tabs suspend. Stores and portfolio/paper/alert/webhook/Pine hosts are shared `Arc<Mutex>`;
  presentation state is per tab/window.
- Saved-layout dirty state is a `LayoutSnapshot` fingerprint, not a flag. It covers timeframe,
  chart configs, and columns—not zoom, drawings, replay, or market data. (An Autosave layout also
  writes on a drawing-only edit, via `ChartWorkspace::drawings_dirty`, so drawings reach iCloud
  Sync; the toolbar's unsaved state still ignores them.) `store::saved_view` owns
  the library; the workspace owns the active layout. Named layouts autosave after two seconds and
  flush before switch/quit; Unnamed never autosaves.
- Tab title is focused-chart title, then active-layout name, then “New tab”. Session persistence is
  debounced ten seconds and also runs on save/quit.

## Windows and tabs

- Tabs are app-owned on every OS; native automatic tabbing is off. Each deferred viewport renders
  one `TabWindow` with `ui::tab_bar` as title bar. The coordinator window stays hidden.
- macOS uses transparent full-size content with a traffic-light `leading_inset` (zero fullscreen).
  Windows/Linux are undecorated and reserve `trailing_inset` for controls; Linux draws resize edges.
  `TabBarOutput::strip` excludes both insets. Empty strip space drags; double-click maximizes. The
  strip is `tab_bar::HEIGHT` (40); recenter macOS traffic lights in it each frame. Settings/Alerts
  retain native title bars.
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
  be driven by any window. Never derive the cursor from the moving window. Linux lacks this handoff;
  a lone tab uses OS drag and Wayland merging may require **Move Tab to Window**.
- macOS menus emit `NativeCommand`s. Windows/Linux use `ui::shortcut` for tab creation/closing and
  all platforms support next/previous tab actions.
- Window grouping is not persisted. Restore sorts tabs by old window and strip position into one
  window. `PersistedTab` is untagged so old chart rows remain compatible; special tabs require
  `kind`.
- New/first-launch tabs are empty (`ChartWorkspace::empty`); only opening a watchlist market seeds a
  chart. Cache empty-page database/Keychain reads per visit, not per frame.
- Snapshot choices (`snapshot.prefs` in the `setting` table: host, trading overlays, scale, approved
  hosts) are app-wide, held by `snapshot_host::SnapshotHost` and shared by every window and Settings.
  A snapshot in flight belongs to its workspace; closing the tab drops its result (an upload
  already sent cannot be recalled). The token is in the Keychain.
