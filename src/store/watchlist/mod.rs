//! Persistence and mutation for the tab's watchlists. Lists live in the `watchlist`
//! document table; per-market flags live in the `watchlist.flags` setting.

use crate::lock::lock;
use crate::model::InstrumentId;
use crate::model::watchlist::layout::Flags;
use crate::model::watchlist::text::ImportReport;
use crate::model::watchlist::{
    EntryId, MAX_LISTS, Watchlist, WatchlistColumn, WatchlistError, WatchlistFlag,
    WatchlistInstrument, WatchlistSort,
};
use crate::store::{Document, SharedStore};

impl Document for Watchlist {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

/// Where in a list a market goes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Place {
    /// At the end of the list, which is the end of its last section.
    #[default]
    End,
    /// Before the first section, with the markets that belong to none.
    Top,
    /// At the end of a section, named by its id or its title.
    Section(String),
}

/// Everything one [`WatchlistStore::edit_list`] may change, applied in this order: the list's
/// name, new sections, renamed sections, removals, moves, additions, deleted sections. Sections
/// are named by id or title (see [`Watchlist::resolve_section`]), so a section made by this
/// very change can be used by the moves and additions after it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ListChanges {
    pub rename: Option<String>,
    pub add_sections: Vec<String>,
    /// `(section, new title)`.
    pub rename_sections: Vec<(String, String)>,
    pub remove: Vec<InstrumentId>,
    pub moves: Vec<(InstrumentId, Place)>,
    pub add: Vec<(WatchlistInstrument, Place)>,
    /// Removes the header only: its markets join the section above, or the top.
    pub delete_sections: Vec<String>,
}

/// What [`WatchlistStore::edit_list`] did, market by market.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ListEdit {
    pub added: Vec<InstrumentId>,
    pub already_present: Vec<InstrumentId>,
    pub removed: Vec<InstrumentId>,
    /// Markets asked to be removed or moved that are not in the list.
    pub not_present: Vec<InstrumentId>,
    pub renamed: bool,
    pub moved: Vec<InstrumentId>,
    pub sections_added: Vec<String>,
    pub sections_renamed: Vec<String>,
    pub sections_deleted: Vec<String>,
}

pub struct WatchlistStore {
    store: SharedStore,
    lists: Vec<Watchlist>,
    flags: Flags,
    load_failed: bool,
    last_error: Option<String>,
}

/// Flags saved under a spelling `InstrumentId::key` no longer uses (`BINANCE:BTC`) move to the
/// market's key; a flag already on that key wins. Returns whether any moved.
fn respell_flags(flags: Flags) -> (Flags, bool) {
    let mut respelled = Flags::new();
    let mut moved = false;
    let mut later = Vec::new();
    for (key, flag) in flags {
        match InstrumentId::rekey(&key) {
            Some(current) if current != key => {
                moved = true;
                later.push((current, flag));
            }
            _ => {
                respelled.insert(key, flag);
            }
        }
    }
    for (key, flag) in later {
        respelled.entry(key).or_insert(flag);
    }
    (respelled, moved)
}

impl WatchlistStore {
    pub fn new(store: SharedStore) -> Self {
        let (mut lists, flags, load_failed, last_error) = {
            let store = lock(&store);
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
        let (flags, flags_respelled) = respell_flags(flags);
        let mut this = Self {
            store,
            lists,
            flags,
            load_failed,
            last_error,
        };
        if !this.load_failed {
            this.repair();
            this.persist();
            if flags_respelled {
                this.persist_flags();
            }
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
            let store = lock(&self.store);
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
        let (flags, flags_respelled) =
            respell_flags(loaded_flags.unwrap_or_default().unwrap_or_default());
        self.flags = flags;
        self.load_failed = false;
        self.last_error = None;
        let had_favorites = lists.iter().any(|list| list.is_favorites);
        if !had_favorites {
            let mut favorites = Watchlist::new("Favorites");
            favorites.is_favorites = true;
            lists.push(favorites);
        }
        self.lists = lists;
        let repaired = self.repair();
        if !had_favorites || repaired {
            self.persist();
        }
        if flags_respelled {
            self.persist_flags();
        }
    }

    /// Puts stored lists right where they can be without losing anything the user chose: a
    /// second Favorites list becomes an ordinary one, rows saved when `BTC` and `BTCUSDT`
    /// were two markets are one now (the first stays), and a repeated row id gets a new one.
    /// Returns whether anything changed.
    fn repair(&mut self) -> bool {
        let mut any = false;
        let mut seen_favorites = false;
        for list in &mut self.lists {
            if list.is_favorites {
                if seen_favorites {
                    list.is_favorites = false;
                    any = true;
                }
                seen_favorites = true;
            }
            any |= list.repair();
        }
        any
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
        let store = lock(&self.store);
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
        let store = lock(&self.store);
        if let Err(error) = store.set_setting("watchlist.flags", &self.flags) {
            self.last_error = Some(format!("Could not save watchlist flags: {error}"));
            match store.setting_strict::<Flags>("watchlist.flags") {
                Ok(flags) => self.flags = flags.unwrap_or_default(),
                Err(_) => self.load_failed = true,
            }
        }
    }

    /// The Favorites list id (always present after construction).
    pub fn favorites(&self) -> EntryId {
        self.lists
            .iter()
            .find(|list| list.is_favorites)
            .map(|list| list.id)
            .unwrap_or_else(|| self.lists[0].id)
    }

    /// Runs `change` on a copy of one list; when it differs from the stored list, keeps it
    /// (stamping `updated_at_ms` for `edit`s of content) and saves. A change that fails
    /// leaves the lists as they were, and one that changes nothing writes nothing. Returns
    /// whether the list changed.
    fn modify(
        &mut self,
        id: EntryId,
        edit: bool,
        change: impl FnOnce(&mut Watchlist) -> Result<(), WatchlistError>,
    ) -> Result<bool, WatchlistError> {
        if self.load_failed {
            return Err(WatchlistError::PersistenceUnavailable);
        }
        let index = self
            .lists
            .iter()
            .position(|list| list.id == id)
            .ok_or(WatchlistError::NotFound)?;
        let mut candidate = self.lists[index].clone();
        change(&mut candidate)?;
        if candidate == self.lists[index] {
            return Ok(false);
        }
        if edit {
            candidate.touch();
        }
        self.lists[index] = candidate;
        if self.persist() {
            Ok(true)
        } else {
            Err(WatchlistError::SaveFailed)
        }
    }

    // MARK: Lists

    pub fn create_list(
        &mut self,
        name: &str,
        with: Option<WatchlistInstrument>,
    ) -> Result<EntryId, WatchlistError> {
        if self.load_failed {
            return Err(WatchlistError::PersistenceUnavailable);
        }
        if self.lists.len() >= MAX_LISTS {
            return Err(WatchlistError::TooManyLists);
        }
        let mut list = Watchlist::new(Watchlist::validated(name)?);
        if let Some(item) = with {
            // A market already refused (a card, say) still gets its list.
            let _ = list.add(item, None);
        }
        let id = list.id;
        self.lists.push(list);
        if self.persist() {
            Ok(id)
        } else {
            Err(WatchlistError::SaveFailed)
        }
    }

    pub fn rename_list(&mut self, id: EntryId, name: &str) -> Result<bool, WatchlistError> {
        let name = Watchlist::validated(name)?;
        self.modify(id, true, |list| {
            list.name = name;
            Ok(())
        })
    }

    pub fn duplicate_list(&mut self, id: EntryId, name: &str) -> Result<EntryId, WatchlistError> {
        if self.load_failed {
            return Err(WatchlistError::PersistenceUnavailable);
        }
        if self.lists.len() >= MAX_LISTS {
            return Err(WatchlistError::TooManyLists);
        }
        let name = Watchlist::validated(name)?;
        let copy = self
            .lists
            .iter()
            .find(|list| list.id == id)
            .map(|list| list.duplicated(&name))
            .ok_or(WatchlistError::NotFound)?;
        let new_id = copy.id;
        self.lists.push(copy);
        if self.persist() {
            Ok(new_id)
        } else {
            Err(WatchlistError::SaveFailed)
        }
    }

    /// Deletes a list; Favorites never deletes (`Ok(false)`).
    pub fn delete_list(&mut self, id: EntryId) -> Result<bool, WatchlistError> {
        if self.load_failed {
            return Err(WatchlistError::PersistenceUnavailable);
        }
        let list = self
            .lists
            .iter()
            .find(|list| list.id == id)
            .ok_or(WatchlistError::NotFound)?;
        if list.is_favorites {
            return Ok(false);
        }
        self.lists.retain(|list| list.id != id);
        if self.persist() {
            Ok(true)
        } else {
            Err(WatchlistError::SaveFailed)
        }
    }

    // MARK: Contents

    pub fn add_to_list(
        &mut self,
        list: EntryId,
        item: WatchlistInstrument,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.add(item, None))
    }

    pub fn remove_from_list(
        &mut self,
        list: EntryId,
        instrument: &InstrumentId,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| {
            list.remove_instrument(instrument)
                .then_some(())
                .ok_or(WatchlistError::NotFound)
        })
    }

    /// Applies `changes` to one list in one copy and one save: a refusal (a bad name, an unknown
    /// section, a full list) leaves the list as it was. A market already in the list, or one to
    /// remove or move that is not, is reported and skipped rather than refused. Returns what was
    /// done and whether the list changed.
    pub fn edit_list(
        &mut self,
        id: EntryId,
        changes: ListChanges,
    ) -> Result<(ListEdit, bool), WatchlistError> {
        let rename = changes
            .rename
            .as_deref()
            .map(Watchlist::validated)
            .transpose()?;
        let mut edit = ListEdit::default();
        let changed = self.modify(id, true, |list| {
            if let Some(name) = rename {
                edit.renamed = list.name != name;
                list.name = name;
            }
            for title in &changes.add_sections {
                let section = list.add_section(title, None)?;
                edit.sections_added.push(section.title);
            }
            for (reference, title) in &changes.rename_sections {
                let section = list.resolve_section(reference)?;
                list.rename_section(section, title)?;
                edit.sections_renamed.push(Watchlist::validated(title)?);
            }
            for instrument in &changes.remove {
                if list.remove_instrument(instrument) {
                    edit.removed.push(instrument.clone());
                } else {
                    edit.not_present.push(instrument.clone());
                }
            }
            for (instrument, place) in &changes.moves {
                let Some(entry) = list.entry_for(instrument).map(|item| item.id) else {
                    edit.not_present.push(instrument.clone());
                    continue;
                };
                let section = match place {
                    Place::Section(reference) => Some(list.resolve_section(reference)?),
                    Place::Top | Place::End => None,
                };
                list.move_instrument(entry, section)?;
                edit.moved.push(instrument.clone());
            }
            for (item, place) in &changes.add {
                if list.contains(&item.instrument) {
                    edit.already_present.push(item.instrument.clone());
                    continue;
                }
                let instrument = item.instrument.clone();
                let entry = item.id;
                match place {
                    Place::End => list.add(item.clone(), None)?,
                    Place::Top => {
                        list.add(item.clone(), None)?;
                        list.move_instrument(entry, None)?;
                    }
                    Place::Section(reference) => {
                        let section = list.resolve_section(reference)?;
                        list.add(item.clone(), Some(section))?;
                    }
                }
                edit.added.push(instrument);
            }
            for reference in &changes.delete_sections {
                let section = list.resolve_section(reference)?;
                list.delete_section(section)?;
                edit.sections_deleted.push(reference.trim().to_owned());
            }
            Ok(())
        })?;
        Ok((edit, changed))
    }

    pub fn remove_entry(&mut self, list: EntryId, entry: EntryId) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.remove_entry(entry))
    }

    pub fn move_entries(
        &mut self,
        list: EntryId,
        ids: &[EntryId],
        before: Option<EntryId>,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.move_entries(ids, before))
    }

    pub fn move_instruments(
        &mut self,
        list: EntryId,
        ids: &[EntryId],
        section: Option<EntryId>,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.move_instruments(ids, section))
    }

    // MARK: Sections

    pub fn add_section(&mut self, list: EntryId, title: &str) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.add_section(title, None).map(|_| ()))
    }

    pub fn rename_section(
        &mut self,
        list: EntryId,
        section: EntryId,
        title: &str,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.rename_section(section, title))
    }

    pub fn delete_section(
        &mut self,
        list: EntryId,
        section: EntryId,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, true, |list| list.delete_section(section))
    }

    /// How a section is shown, not what the list holds, so it does not count as an edit.
    pub fn set_section_collapsed(
        &mut self,
        list: EntryId,
        section: EntryId,
        collapsed: bool,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, false, |list| {
            list.set_section_collapsed(section, collapsed)
        })
    }

    // MARK: Display

    pub fn set_sort(&mut self, list: EntryId, sort: WatchlistSort) -> Result<bool, WatchlistError> {
        self.modify(list, false, |list| {
            list.display.sort = sort;
            Ok(())
        })
    }

    pub fn set_column(
        &mut self,
        list: EntryId,
        column: WatchlistColumn,
        visible: bool,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, false, |list| {
            list.display.set_column(column, visible);
            Ok(())
        })
    }

    pub fn set_shows_description(
        &mut self,
        list: EntryId,
        shows: bool,
    ) -> Result<bool, WatchlistError> {
        self.modify(list, false, |list| {
            list.display.shows_description = shows;
            Ok(())
        })
    }

    // MARK: Flags

    pub fn set_flag(&mut self, instrument: &InstrumentId, flag: Option<WatchlistFlag>) {
        if self.load_failed {
            return;
        }
        let key = instrument.key();
        if self.flags.get(&key) == flag.as_ref() {
            return;
        }
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

    /// Imports `text` into a list. The list is replaced only when the whole import worked.
    pub fn import_into(
        &mut self,
        list: EntryId,
        text: &str,
    ) -> Result<ImportReport, WatchlistError> {
        let parsed = crate::model::watchlist::text::parse(text);
        let mut report = None;
        self.modify(list, true, |target| {
            report = Some(crate::model::watchlist::text::import_into(target, &parsed)?);
            Ok(())
        })?;
        Ok(report.unwrap_or_default())
    }

    pub fn export(&self, list: EntryId) -> Option<String> {
        self.lists
            .iter()
            .find(|target| target.id == list)
            .map(crate::model::watchlist::text::export)
    }
}

#[cfg(test)]
mod tests;
