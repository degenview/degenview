//! Tab Linking's shared state: the switches of every colour group, saved to the `setting` table.
//! One of these is shared by every window (and any menu that edits a group). The choices are local to the Mac, like the tabs themselves.
//!
//! A tab's own colour is not here: it is saved with the tab (`PersistedWorkspace::link`).
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::tab_link::{LinkColor, LinkTable, LinkToggles};
use crate::store::{SharedStore, Store};

#[derive(Clone, Default)]
pub struct TabLinkHost {
    store: Option<SharedStore>,
    table: Arc<Mutex<LinkTable>>,
    /// Bumped on every change to a group's switches or a colour, so the app can repaint the tab
    /// windows (each repaints on its own and an idle one would keep showing the old state).
    revision: Arc<AtomicU64>,
}

impl TabLinkHost {
    /// The saved switches; a row this version cannot read is ignored and left as it is until the
    /// user changes a group.
    pub fn load(store: SharedStore) -> Self {
        let table = store
            .lock()
            .ok()
            .and_then(|store| store.setting_strict::<LinkTable>(LinkTable::KEY).ok())
            .flatten()
            .unwrap_or_default();
        Self {
            store: Some(store),
            table: Arc::new(Mutex::new(table)),
            revision: Arc::default(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Wakes anything watching [`revision`](Self::revision): a tab was tagged or untagged.
    pub fn touch(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    pub fn toggles(&self, color: LinkColor) -> LinkToggles {
        self.table().toggles(color)
    }

    /// The switches of every group, for the hub.
    pub fn table(&self) -> LinkTable {
        self.table.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Applies a group's switches and saves them. `held` is the store when the caller already has
    /// it locked; `None` locks the host's own.
    pub fn set_toggles(
        &self,
        held: Option<&Store>,
        color: LinkColor,
        toggles: LinkToggles,
    ) -> Result<(), String> {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if !table.set(color, toggles) {
            return Ok(());
        }
        self.touch();
        let write = |store: &Store| {
            store
                .set_setting(LinkTable::KEY, &*table)
                .map_err(|error| format!("Could not save the tab linking settings: {error}"))
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
    fn a_group_defaults_to_symbol_only() {
        let host = TabLinkHost::load(shared());
        assert_eq!(host.toggles(LinkColor::Orange), LinkToggles::default());
        assert!(host.toggles(LinkColor::Orange).symbol);
        assert!(!host.toggles(LinkColor::Orange).interval);
    }

    #[test]
    fn switches_are_saved_per_colour_and_loaded_again() {
        let store = shared();
        let host = TabLinkHost::load(store.clone());
        let all = LinkToggles {
            symbol: true,
            interval: true,
            time: true,
            date_range: true,
        };
        host.set_toggles(None, LinkColor::Blue, all).expect("save");
        assert_eq!(host.toggles(LinkColor::Blue), all);
        assert_eq!(host.toggles(LinkColor::Red), LinkToggles::default());
        let again = TabLinkHost::load(store);
        assert_eq!(again.toggles(LinkColor::Blue), all);
        assert_eq!(again.toggles(LinkColor::Red), LinkToggles::default());
    }

    #[test]
    fn a_change_made_while_the_store_is_held_is_written_through() {
        let store = shared();
        let host = TabLinkHost::load(store.clone());
        {
            let guard = store.lock().unwrap();
            host.set_toggles(Some(&guard), LinkColor::Pink, LinkToggles::NONE)
                .expect("save");
        }
        assert_eq!(
            TabLinkHost::load(store).toggles(LinkColor::Pink),
            LinkToggles::NONE
        );
    }

    #[test]
    fn every_change_bumps_the_revision_and_a_repeat_does_not() {
        let host = TabLinkHost::load(shared());
        let seen = host.revision();
        host.set_toggles(None, LinkColor::Red, LinkToggles::NONE)
            .expect("save");
        assert_eq!(host.revision(), seen + 1);
        host.set_toggles(None, LinkColor::Red, LinkToggles::NONE)
            .expect("save");
        assert_eq!(host.revision(), seen + 1, "nothing changed");
        host.touch();
        assert_eq!(host.revision(), seen + 2);
        assert_eq!(host.clone().revision(), seen + 2, "clones share it");
    }

    #[test]
    fn an_unreadable_saved_value_falls_back_to_the_defaults_and_is_kept() {
        let store = shared();
        store
            .lock()
            .unwrap()
            .set_setting(LinkTable::KEY, &serde_json::json!(["not", "a", "table"]))
            .unwrap();
        let host = TabLinkHost::load(store);
        assert_eq!(host.toggles(LinkColor::Green), LinkToggles::default());
    }
}
