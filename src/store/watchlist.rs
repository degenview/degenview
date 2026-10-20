//! Persistence and mutation for the tab's watchlists. Lists live in the `watchlist`
//! document table; per-market flags live in the `watchlist.flags` setting.

use crate::model::InstrumentId;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistColumn, WatchlistFlag, WatchlistInstrument, WatchlistSort,
};
use crate::model::watchlist_layout::Flags;
use crate::model::watchlist_text::ImportReport;
use crate::store::{Document, SharedStore};

impl Document for Watchlist {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

pub struct WatchlistStore {
    store: SharedStore,
    lists: Vec<Watchlist>,
    flags: Flags,
    load_failed: bool,
    last_error: Option<String>,
}

impl WatchlistStore {
    pub fn new(store: SharedStore) -> Self {
        let (mut lists, flags, load_failed, last_error) = {
            let store = store.lock().expect("store lock poisoned");
            let loaded = store.documents_strict::<Watchlist>("watchlist");
            let loaded_flags = store.setting_strict::<Flags>("watchlist.flags");
            let error = loaded
                .as_ref()
                .err()
                .map(ToString::to_string)
                .or_else(|| loaded_flags.as_ref().err().map(ToString::to_string));
            (
                loaded.unwrap_or_default(),
                loaded_flags.unwrap_or_default().unwrap_or_default(),
                error.is_some(),
                error,
            )
        };
        if !lists.iter().any(|list| list.is_favorites) {
            let mut favorites = Watchlist::new("Favorites");
            favorites.is_favorites = true;
            lists.push(favorites);
        }
        let mut this = Self {
            store,
            lists,
            flags,
            load_failed,
            last_error,
        };
        if !this.load_failed {
            this.persist();
        }
        this
    }

    pub fn lists(&self) -> &[Watchlist] {
        &self.lists
    }

    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    /// Takes the lists and flags from the store again, discarding what is held in memory. Used
    /// after another process or iCloud Sync wrote them: this store otherwise writes back from its
    /// own copy and would undo that change on the next edit.
    pub fn reload(&mut self) {
        let (loaded, loaded_flags) = {
            let store = self.store.lock().expect("store lock poisoned");
            (
                store.documents_strict::<Watchlist>("watchlist"),
                store.setting_strict::<Flags>("watchlist.flags"),
            )
        };
        let error = loaded
            .as_ref()
            .err()
            .map(ToString::to_string)
            .or_else(|| loaded_flags.as_ref().err().map(ToString::to_string));
        if error.is_some() {
            // Keep what is in memory and refuse writes, as at startup.
            self.load_failed = true;
            self.last_error = error;
            return;
        }
        let mut lists = loaded.unwrap_or_default();
        self.flags = loaded_flags.unwrap_or_default().unwrap_or_default();
        self.load_failed = false;
        self.last_error = None;
        let had_favorites = lists.iter().any(|list| list.is_favorites);
        if !had_favorites {
            let mut favorites = Watchlist::new("Favorites");
            favorites.is_favorites = true;
            lists.push(favorites);
        }
        self.lists = lists;
        if !had_favorites {
            self.persist();
        }
    }

    pub fn load_failed(&self) -> bool {
        self.load_failed
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    fn persist(&mut self) -> bool {
        if self.load_failed {
            return false;
        }
        let store = self.store.lock().expect("store lock poisoned");
        match store.replace_documents("watchlist", &self.lists) {
            Ok(()) => true,
            Err(error) => {
                self.last_error = Some(format!("Could not save watchlists: {error}"));
                match store.documents_strict::<Watchlist>("watchlist") {
                    Ok(mut lists) => {
                        if !lists.iter().any(|list| list.is_favorites) {
                            let mut favorites = Watchlist::new("Favorites");
                            favorites.is_favorites = true;
                            lists.push(favorites);
                        }
                        self.lists = lists;
                    }
                    Err(_) => self.load_failed = true,
                }
                false
            }
        }
    }

    fn persist_flags(&mut self) {
        if self.load_failed {
            return;
        }
        let store = self.store.lock().expect("store lock poisoned");
        if let Err(error) = store.set_setting("watchlist.flags", &self.flags) {
            self.last_error = Some(format!("Could not save watchlist flags: {error}"));
            match store.setting_strict::<Flags>("watchlist.flags") {
                Ok(flags) => self.flags = flags.unwrap_or_default(),
                Err(_) => self.load_failed = true,
            }
        }
    }

    fn list_mut(&mut self, id: EntryId) -> Option<&mut Watchlist> {
        self.lists.iter_mut().find(|list| list.id == id)
    }

    /// The Favorites list id (always present after construction).
    pub fn favorites(&self) -> EntryId {
        self.lists
            .iter()
            .find(|list| list.is_favorites)
            .map(|list| list.id)
            .unwrap_or_else(|| self.lists[0].id)
    }

    // MARK: Lists

    pub fn create_list(
        &mut self,
        name: &str,
        with: Option<WatchlistInstrument>,
    ) -> Option<EntryId> {
        let mut list = Watchlist::new(name);
        if let Some(item) = with {
            let _ = list.add(item, None);
        }
        let id = list.id;
        self.lists.push(list);
        self.persist().then_some(id)
    }

    pub fn rename_list(&mut self, id: EntryId, name: &str) {
        if let Some(list) = self.list_mut(id) {
            list.name = name.to_owned();
            self.persist();
        }
    }

    pub fn duplicate_list(&mut self, id: EntryId, name: &str) -> Option<EntryId> {
        let copy = self
            .lists
            .iter()
            .find(|list| list.id == id)
            .map(|list| list.duplicated(name))?;
        let new_id = copy.id;
        self.lists.push(copy);
        self.persist().then_some(new_id)
    }

    /// Deletes a list; Favorites never deletes.
    pub fn delete_list(&mut self, id: EntryId) {
        let Some(list) = self.lists.iter().find(|list| list.id == id) else {
            return;
        };
        if list.is_favorites {
            return;
        }
        self.lists.retain(|list| list.id != id);
        self.persist();
    }

    // MARK: Contents

    pub fn add_to_list(&mut self, list: EntryId, item: WatchlistInstrument) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.add(item, None);
            self.persist();
        }
    }

    pub fn remove_from_list(&mut self, list: EntryId, instrument: &InstrumentId) {
        if let Some(list) = self.list_mut(list)
            && list.remove_instrument(instrument)
        {
            self.persist();
        }
    }

    pub fn remove_entry(&mut self, list: EntryId, entry: EntryId) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.remove_entry(entry);
            self.persist();
        }
    }

    pub fn move_entries(&mut self, list: EntryId, ids: &[EntryId], before: Option<EntryId>) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.move_entries(ids, before);
            self.persist();
        }
    }

    pub fn move_instruments(&mut self, list: EntryId, ids: &[EntryId], section: Option<EntryId>) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.move_instruments(ids, section);
            self.persist();
        }
    }

    // MARK: Sections

    pub fn add_section(&mut self, list: EntryId, title: &str) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.add_section(title, None);
            self.persist();
        }
    }

    pub fn rename_section(&mut self, list: EntryId, section: EntryId, title: &str) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.rename_section(section, title);
            self.persist();
        }
    }

    pub fn delete_section(&mut self, list: EntryId, section: EntryId) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.delete_section(section);
            self.persist();
        }
    }

    pub fn set_section_collapsed(&mut self, list: EntryId, section: EntryId, collapsed: bool) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.set_section_collapsed(section, collapsed);
            self.persist();
        }
    }

    // MARK: Display

    pub fn set_sort(&mut self, list: EntryId, sort: WatchlistSort) {
        if let Some(list) = self.list_mut(list) {
            list.display.sort = sort;
            self.persist();
        }
    }

    pub fn set_column(&mut self, list: EntryId, column: WatchlistColumn, visible: bool) {
        if let Some(list) = self.list_mut(list) {
            list.display.set_column(column, visible);
            self.persist();
        }
    }

    pub fn set_shows_description(&mut self, list: EntryId, shows: bool) {
        if let Some(list) = self.list_mut(list) {
            list.display.shows_description = shows;
            self.persist();
        }
    }

    // MARK: Flags

    pub fn set_flag(&mut self, instrument: &InstrumentId, flag: Option<WatchlistFlag>) {
        if self.load_failed {
            return;
        }
        let key = instrument.key();
        match flag {
            Some(flag) => {
                self.flags.insert(key, flag);
            }
            None => {
                self.flags.remove(&key);
            }
        }
        self.persist_flags();
    }

    // MARK: Import / export

    pub fn import_into(&mut self, list: EntryId, text: &str) -> Option<ImportReport> {
        let parsed = crate::model::watchlist_text::parse(text);
        let target = self.list_mut(list)?;
        match crate::model::watchlist_text::import_into(target, &parsed) {
            Ok(report) => self.persist().then_some(report),
            Err(_) => None,
        }
    }

    pub fn export(&self, list: EntryId) -> Option<String> {
        self.lists
            .iter()
            .find(|target| target.id == list)
            .map(crate::model::watchlist_text::export)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::store::Store;

    fn item(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument::from_market(DataSource::Binance, symbol, symbol)
    }

    #[test]
    fn reload_takes_what_another_writer_stored_instead_of_undoing_it() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        let mut store = WatchlistStore::new(shared.clone());
        let favorites = store.favorites();
        store.add_to_list(favorites, item("BTCUSDT"));
        // Another writer (iCloud Sync) stores a new list behind this store's back.
        let mut lists: Vec<Watchlist> = shared
            .lock()
            .unwrap()
            .documents_strict("watchlist")
            .unwrap();
        lists.push(Watchlist::new("From another Mac"));
        shared
            .lock()
            .unwrap()
            .replace_documents("watchlist", &lists)
            .unwrap();
        assert_eq!(
            store.lists().len(),
            1,
            "the in-memory copy does not know yet"
        );
        store.reload();
        assert_eq!(store.lists().len(), 2);
        // The next edit keeps the other writer's list rather than writing the old copy back.
        store.add_to_list(favorites, item("ETHUSDT"));
        let stored: Vec<Watchlist> = shared
            .lock()
            .unwrap()
            .documents_strict("watchlist")
            .unwrap();
        assert!(stored.iter().any(|l| l.name == "From another Mac"));
    }

    #[test]
    fn reload_recreates_a_missing_favorites_list_and_recovers_from_a_bad_row() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        let mut store = WatchlistStore::new(shared.clone());
        shared
            .lock()
            .unwrap()
            .replace_documents::<Watchlist>("watchlist", &[])
            .unwrap();
        store.reload();
        assert!(store.lists().iter().any(|l| l.is_favorites));
        assert!(!store.load_failed());
    }

    #[test]
    fn round_trips_lists_and_flags_through_the_store() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        {
            let mut store = WatchlistStore::new(shared.clone());
            let favorites = store.favorites();
            store.add_to_list(favorites, item("BTCUSDT"));
            let id = store
                .create_list("Meme", Some(item("DOGEUSDT")))
                .expect("list");
            store.set_flag(
                &InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                Some(WatchlistFlag::Blue),
            );
            let _ = id;
        }
        let reloaded = WatchlistStore::new(shared);
        assert_eq!(reloaded.lists().len(), 2);
        let favorites = reloaded.favorites();
        let list = reloaded.lists().iter().find(|l| l.id == favorites).unwrap();
        assert!(list.contains(&InstrumentId::new(DataSource::Binance, "BTCUSDT")));
        assert!(
            reloaded
                .flags()
                .contains_key(&InstrumentId::new(DataSource::Binance, "BTCUSDT").key())
        );
    }

    #[test]
    fn unreadable_watchlist_is_reported_and_preserved() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        shared
            .lock()
            .unwrap()
            .connection
            .execute(
                "INSERT INTO watchlist (id, position, payload) VALUES ('broken', 0, 'not json')",
                [],
            )
            .expect("seed");
        let list = WatchlistStore::new(shared.clone());
        assert!(list.load_failed());
        let count: i64 = shared
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT COUNT(*) FROM watchlist WHERE id = 'broken'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn unreadable_flags_are_not_overwritten() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        shared
            .lock()
            .unwrap()
            .connection
            .execute(
                "INSERT INTO setting (key, value) VALUES ('watchlist.flags', 'bad')",
                [],
            )
            .expect("seed");
        let mut watchlists = WatchlistStore::new(shared.clone());
        assert!(watchlists.load_failed());
        watchlists.set_flag(
            &InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            Some(WatchlistFlag::Blue),
        );
        let value: String = shared
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT value FROM setting WHERE key = 'watchlist.flags'",
                [],
                |row| row.get(0),
            )
            .expect("read");
        assert_eq!(value, "bad");
    }

    #[test]
    fn failed_write_restores_the_stored_list_and_reports_error() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        let mut watchlists = WatchlistStore::new(shared.clone());
        let favorites = watchlists.favorites();
        shared
            .lock()
            .unwrap()
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_watchlist_delete BEFORE DELETE ON watchlist
                 BEGIN SELECT RAISE(ABORT, 'save failed'); END;",
            )
            .expect("trigger");
        assert!(watchlists.create_list("Unsaved", None).is_none());
        assert!(watchlists.last_error().is_some());
        assert_eq!(watchlists.lists().len(), 1);
        assert_eq!(watchlists.favorites(), favorites);
        assert_eq!(
            shared
                .lock()
                .unwrap()
                .documents::<Watchlist>("watchlist")
                .len(),
            1
        );
    }
}
