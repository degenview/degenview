# Changelog

What changed for people who use DegenView, release by release, newest first. `tools/release.sh` writes
each section from the fragments in `changes/` when it cuts the release, and the app shows the
sections between your version and the one on offer before it updates.

## 0.6.0-rc.33 (2026-11-21)

### New

- The Update button now shows what is new and what was fixed before you install.

### Fixed

- Closing the last window on Linux or Windows no longer deletes your tabs and drawings
- Pine scripts with loops now show the right values when a loop runs a different number of times from bar to bar.
- On Wayland (GNOME, KDE), closing Settings and opening new tabs no longer wait until the window is shown again, and no empty extra DegenView window appears in Alt+Tab

### Fixed in release candidates

- US stock charts on Alpaca's free plan load the latest bars again instead of showing a data error.
