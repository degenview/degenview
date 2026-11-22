# Changelog

What changed for people who use DegenView, release by release, newest first. `tools/release.sh` writes
each section from the fragments in `changes/` when it cuts the release, and the app shows the
sections between your version and the one on offer before it updates.

## 0.6.0-rc.34 (2026-11-23)

### New

- Drag the line above a script pane to resize it, as in TradingView; pane heights are saved with the chart.

### Fixed

- The app no longer fails to open without a message on some Windows PCs; it starts with DirectX 12 and switches to another graphics mode by itself if a start does not work.

### Fixed in release candidates

- Text no longer looks tall and stretched on screens that are not Retina.
- The update sheet no longer says its release notes are complete when it stopped reading the release list early.

## 0.6.0-rc.33 (2026-11-21)

### New

- The Update button now shows what is new and what was fixed before you install.

### Fixed

- Closing the last window on Linux or Windows no longer deletes your tabs and drawings
- Pine scripts with loops now show the right values when a loop runs a different number of times from bar to bar.
- On Wayland (GNOME, KDE), closing Settings and opening new tabs no longer wait until the window is shown again, and no empty extra DegenView window appears in Alt+Tab

### Fixed in release candidates

- US stock charts on Alpaca's free plan load the latest bars again instead of showing a data error.
