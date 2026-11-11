//! Which tabs share a window, in what order, and which one is showing.
//!
//! Pure bookkeeping for the in-app tab bar: `TabWindows` holds ids only (the tabs' content lives in
//! the app), so every operation here is a list edit that the window host turns into viewports.

pub type TabId = u64;
pub type WindowId = u64;

/// One window's tab strip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TabWindow {
    pub id: WindowId,
    pub tabs: Vec<TabId>,
    pub active: TabId,
}

/// What closing a tab left behind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Closed {
    pub window: WindowId,
    /// The tab was the window's last one, so the window is gone too.
    pub window_closed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TabWindows {
    windows: Vec<TabWindow>,
    next_window_id: WindowId,
}

impl TabWindows {
    /// Every tab in one window, the first showing: how a launch restores the session.
    pub fn from_tabs(tabs: impl IntoIterator<Item = TabId>) -> Self {
        let mut windows = Self::default();
        for tab in tabs {
            let window = windows.windows.first().map(|window| window.id);
            windows.add_tab(window, tab);
            if let Some(first) = windows.windows.first_mut() {
                first.active = first.tabs[0];
            }
        }
        windows
    }

    pub fn windows(&self) -> &[TabWindow] {
        &self.windows
    }

    pub fn window(&self, id: WindowId) -> Option<&TabWindow> {
        self.windows.iter().find(|window| window.id == id)
    }

    pub fn window_of(&self, tab: TabId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|window| window.tabs.contains(&tab))
            .map(|window| window.id)
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Tabs in window order, then strip order: the order a session is saved in.
    pub fn ordered_tabs(&self) -> Vec<TabId> {
        self.windows
            .iter()
            .flat_map(|window| window.tabs.iter().copied())
            .collect()
    }

    /// Appends `tab` to `window` and shows it. A missing or unknown window opens a new one. A tab
    /// that is already open is only selected.
    pub fn add_tab(&mut self, window: Option<WindowId>, tab: TabId) -> WindowId {
        if let Some(existing) = self.window_of(tab) {
            self.select(tab);
            return existing;
        }
        if let Some(window) = window.and_then(|id| self.window_mut(id)) {
            window.tabs.push(tab);
            window.active = tab;
            return window.id;
        }
        self.open_window(tab)
    }

    /// Removes `tab`; the window shows its right neighbour next, else its left.
    pub fn close_tab(&mut self, tab: TabId) -> Option<Closed> {
        let window_id = self.window_of(tab)?;
        let window = self.window_mut(window_id)?;
        let index = window.tabs.iter().position(|id| *id == tab)?;
        window.tabs.remove(index);
        if window.tabs.is_empty() {
            self.windows.retain(|window| window.id != window_id);
            return Some(Closed {
                window: window_id,
                window_closed: true,
            });
        }
        if window.active == tab {
            window.active = window.tabs[index.min(window.tabs.len() - 1)];
        }
        Some(Closed {
            window: window_id,
            window_closed: false,
        })
    }

    pub fn select(&mut self, tab: TabId) -> bool {
        match self.window_of(tab).and_then(|id| self.window_mut(id)) {
            Some(window) => {
                window.active = tab;
                true
            }
            None => false,
        }
    }

    /// Moves `tab` within its strip to `index` (counted before the move, clamped to the strip).
    pub fn reorder(&mut self, tab: TabId, index: usize) -> bool {
        let Some(window) = self.window_of(tab).and_then(|id| self.window_mut(id)) else {
            return false;
        };
        let Some(from) = window.tabs.iter().position(|id| *id == tab) else {
            return false;
        };
        let to = index.min(window.tabs.len() - 1);
        if from != to {
            let moved = window.tabs.remove(from);
            window.tabs.insert(to, moved);
        }
        true
    }

    /// Moves `tab` into `target` at `index` and shows it there. The window it leaves closes when
    /// it empties. Within one window this is `reorder`.
    pub fn move_tab(&mut self, tab: TabId, target: WindowId, index: usize) -> bool {
        let Some(source) = self.window_of(tab) else {
            return false;
        };
        if self.window(target).is_none() {
            return false;
        }
        if source == target {
            self.select(tab);
            return self.reorder(tab, index);
        }
        self.close_tab(tab);
        let Some(window) = self.window_mut(target) else {
            return false;
        };
        window.tabs.insert(index.min(window.tabs.len()), tab);
        window.active = tab;
        true
    }

    /// Puts `tab` in a window of its own. A tab that is already alone keeps its window.
    pub fn detach(&mut self, tab: TabId) -> Option<WindowId> {
        let source = self.window_of(tab)?;
        if self
            .window(source)
            .is_some_and(|window| window.tabs.len() == 1)
        {
            return Some(source);
        }
        self.close_tab(tab);
        Some(self.open_window(tab))
    }

    /// Shows the next (or previous) tab of `window`, wrapping around.
    pub fn cycle(&mut self, window: WindowId, forward: bool) -> Option<TabId> {
        let window = self.window_mut(window)?;
        let index = window.tabs.iter().position(|id| *id == window.active)?;
        let count = window.tabs.len();
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        window.active = window.tabs[next];
        Some(window.active)
    }

    /// Pulls every other window's tabs into `target`, in window order.
    pub fn merge_all(&mut self, target: WindowId) {
        if self.window(target).is_none() {
            return;
        }
        let others: Vec<TabId> = self
            .windows
            .iter()
            .filter(|window| window.id != target)
            .flat_map(|window| window.tabs.iter().copied())
            .collect();
        self.windows.retain(|window| window.id == target);
        if let Some(window) = self.window_mut(target) {
            window.tabs.extend(others);
        }
    }

    fn open_window(&mut self, tab: TabId) -> WindowId {
        self.next_window_id += 1;
        let id = self.next_window_id;
        self.windows.push(TabWindow {
            id,
            tabs: vec![tab],
            active: tab,
        });
        id
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut TabWindow> {
        self.windows.iter_mut().find(|window| window.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs_of(windows: &TabWindows, id: WindowId) -> Vec<TabId> {
        windows.window(id).expect("window").tabs.clone()
    }

    #[test]
    fn restore_puts_every_tab_in_one_window_showing_the_first() {
        let windows = TabWindows::from_tabs([5, 3, 9]);
        assert_eq!(windows.windows().len(), 1);
        let window = &windows.windows()[0];
        assert_eq!(window.tabs, vec![5, 3, 9]);
        assert_eq!(window.active, 5);
        assert!(TabWindows::from_tabs([]).is_empty());
    }

    #[test]
    fn a_new_tab_joins_the_window_and_shows() {
        let mut windows = TabWindows::from_tabs([1, 2]);
        let id = windows.windows()[0].id;
        assert_eq!(windows.add_tab(Some(id), 3), id);
        assert_eq!(tabs_of(&windows, id), vec![1, 2, 3]);
        assert_eq!(windows.window(id).unwrap().active, 3);
    }

    #[test]
    fn adding_without_a_window_opens_one_and_an_open_tab_is_only_selected() {
        let mut windows = TabWindows::default();
        let first = windows.add_tab(None, 1);
        let second = windows.add_tab(Some(999), 2);
        assert_ne!(first, second);
        assert_eq!(windows.add_tab(Some(second), 1), first);
        assert_eq!(windows.ordered_tabs(), vec![1, 2]);
    }

    #[test]
    fn closing_the_showing_tab_shows_its_right_neighbour_else_the_left() {
        let mut windows = TabWindows::from_tabs([1, 2, 3]);
        let id = windows.windows()[0].id;
        windows.select(2);
        windows.close_tab(2);
        assert_eq!(windows.window(id).unwrap().active, 3);
        windows.close_tab(3);
        assert_eq!(windows.window(id).unwrap().active, 1);
    }

    #[test]
    fn closing_a_background_tab_keeps_the_showing_one() {
        let mut windows = TabWindows::from_tabs([1, 2, 3]);
        let id = windows.windows()[0].id;
        windows.select(3);
        windows.close_tab(1);
        assert_eq!(windows.window(id).unwrap().active, 3);
    }

    #[test]
    fn closing_the_last_tab_closes_the_window() {
        let mut windows = TabWindows::from_tabs([1]);
        let id = windows.windows()[0].id;
        let closed = windows.close_tab(1).unwrap();
        assert_eq!(
            closed,
            Closed {
                window: id,
                window_closed: true
            }
        );
        assert!(windows.is_empty());
        assert_eq!(windows.close_tab(1), None);
    }

    #[test]
    fn reorder_clamps_to_the_strip() {
        let mut windows = TabWindows::from_tabs([1, 2, 3]);
        let id = windows.windows()[0].id;
        assert!(windows.reorder(1, 99));
        assert_eq!(tabs_of(&windows, id), vec![2, 3, 1]);
        assert!(windows.reorder(1, 0));
        assert_eq!(tabs_of(&windows, id), vec![1, 2, 3]);
        assert!(!windows.reorder(42, 0));
    }

    #[test]
    fn detach_opens_a_window_but_a_lone_tab_keeps_its_own() {
        let mut windows = TabWindows::from_tabs([1, 2]);
        let home = windows.windows()[0].id;
        let detached = windows.detach(2).unwrap();
        assert_ne!(detached, home);
        assert_eq!(tabs_of(&windows, home), vec![1]);
        assert_eq!(tabs_of(&windows, detached), vec![2]);
        assert_eq!(windows.detach(2), Some(detached));
        assert_eq!(windows.windows().len(), 2);
    }

    #[test]
    fn move_tab_joins_another_window_and_closes_the_emptied_one() {
        let mut windows = TabWindows::from_tabs([1, 2]);
        let home = windows.windows()[0].id;
        let other = windows.detach(2).unwrap();
        assert!(windows.move_tab(2, home, 0));
        assert_eq!(tabs_of(&windows, home), vec![2, 1]);
        assert_eq!(windows.window(home).unwrap().active, 2);
        assert!(windows.window(other).is_none());
    }

    #[test]
    fn move_tab_within_a_window_is_a_reorder_and_rejects_unknown_targets() {
        let mut windows = TabWindows::from_tabs([1, 2, 3]);
        let id = windows.windows()[0].id;
        assert!(windows.move_tab(3, id, 0));
        assert_eq!(tabs_of(&windows, id), vec![3, 1, 2]);
        assert!(!windows.move_tab(3, 999, 0));
        assert!(!windows.move_tab(42, id, 0));
        assert_eq!(tabs_of(&windows, id), vec![3, 1, 2]);
    }

    #[test]
    fn cycling_wraps_both_ways() {
        let mut windows = TabWindows::from_tabs([1, 2, 3]);
        let id = windows.windows()[0].id;
        assert_eq!(windows.cycle(id, false), Some(3));
        assert_eq!(windows.cycle(id, true), Some(1));
        assert_eq!(windows.cycle(999, true), None);
    }

    #[test]
    fn merge_all_pulls_every_tab_into_the_target_in_window_order() {
        let mut windows = TabWindows::from_tabs([1, 2, 3, 4]);
        let home = windows.windows()[0].id;
        windows.detach(2);
        windows.detach(4);
        windows.merge_all(home);
        assert_eq!(windows.windows().len(), 1);
        assert_eq!(windows.ordered_tabs(), vec![1, 3, 2, 4]);
    }

    #[test]
    fn ordered_tabs_follow_window_then_strip_order() {
        let mut windows = TabWindows::from_tabs([1, 2, 3]);
        windows.detach(1);
        assert_eq!(windows.ordered_tabs(), vec![2, 3, 1]);
    }
}
