//! How chart tabs name themselves (Settings ▸ Appearance ▸ Tabs): one value shared by every
//! window and Settings, written to the `setting` table on every change. The choice is local to
//! the Mac, like the tabs themselves.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::tab::title::TabTitleStyle;
use crate::store::{SharedStore, Store};

#[derive(Clone, Default)]
pub struct TabTitleHost {
    store: Option<SharedStore>,
    style: Arc<Mutex<TabTitleStyle>>,
    /// Bumped on every change, so the app can repaint the tab windows: egui repaints each
    /// window on its own, and a window with nothing animating (a portfolio, say) would keep
    /// showing the old titles until something else woke it.
    revision: Arc<AtomicU64>,
}

impl TabTitleHost {
    /// The saved choice; a value this version cannot read is ignored and left as it is until
    /// the user changes it.
    pub fn load(store: SharedStore) -> Self {
        let style = store
            .lock()
            .ok()
            .and_then(|store| {
                store
                    .setting_strict::<TabTitleStyle>(TabTitleStyle::KEY)
                    .ok()
            })
            .flatten()
            .unwrap_or_default();
        Self {
            store: Some(store),
            style: Arc::new(Mutex::new(style)),
            revision: Arc::default(),
        }
    }

    /// Changes whenever the style does.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn style(&self) -> TabTitleStyle {
        *self.style.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Applies the choice and saves it. `held` is the store when the caller already has it
    /// locked (the Settings window draws under that lock); `None` locks the host's own.
    pub fn set(&self, held: Option<&Store>, style: TabTitleStyle) -> Result<(), String> {
        let mut current = self.style.lock().unwrap_or_else(|e| e.into_inner());
        if *current == style {
            return Ok(());
        }
        *current = style;
        self.revision.fetch_add(1, Ordering::AcqRel);
        let write = |store: &Store| {
            store
                .set_setting(TabTitleStyle::KEY, &style)
                .map_err(|error| format!("Could not save the tab title setting: {error}"))
        };
        if let Some(store) = held {
            return write(store);
        }
        let Some(shared) = &self.store else {
            return Ok(());
        };
        let store = shared
            .lock()
            .map_err(|_| "The settings database is unavailable.".to_owned())?;
        write(&store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("store")))
    }

    #[test]
    fn defaults_to_the_full_title() {
        assert_eq!(TabTitleHost::load(shared()).style(), TabTitleStyle::Full);
    }

    #[test]
    fn a_choice_is_saved_and_loaded_again() {
        let store = shared();
        let host = TabTitleHost::load(store.clone());
        host.set(None, TabTitleStyle::LayoutOnly).expect("save");
        assert_eq!(host.style(), TabTitleStyle::LayoutOnly);
        assert_eq!(TabTitleHost::load(store).style(), TabTitleStyle::LayoutOnly);
    }

    #[test]
    fn a_choice_made_while_the_store_is_held_is_written_through() {
        let store = shared();
        let host = TabTitleHost::load(store.clone());
        {
            let guard = store.lock().unwrap();
            host.set(Some(&guard), TabTitleStyle::SymbolOnly)
                .expect("save");
        }
        assert_eq!(TabTitleHost::load(store).style(), TabTitleStyle::SymbolOnly);
    }

    #[test]
    fn every_change_bumps_the_revision_and_a_repeat_does_not() {
        let host = TabTitleHost::load(shared());
        let seen = host.revision();
        host.set(None, TabTitleStyle::SymbolOnly).expect("save");
        assert_eq!(host.revision(), seen + 1);
        host.set(None, TabTitleStyle::SymbolOnly).expect("save");
        assert_eq!(host.revision(), seen + 1, "nothing changed");
        // Clones share it: a change made in Settings is seen by the app's copy.
        let other = host.clone();
        other.set(None, TabTitleStyle::LayoutOnly).expect("save");
        assert_eq!(host.revision(), seen + 2);
    }

    #[test]
    fn restoring_the_default_is_a_choice_too() {
        let store = shared();
        let host = TabTitleHost::load(store.clone());
        host.set(None, TabTitleStyle::SymbolOnly).expect("save");
        host.set(None, TabTitleStyle::default()).expect("save");
        assert_eq!(TabTitleHost::load(store).style(), TabTitleStyle::Full);
    }
}
