use super::*;
use crate::model::DataSource;
use crate::model::watchlist::WatchlistEntry;
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
    store.add_to_list(favorites, item("BTCUSDT")).unwrap();
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
    store.add_to_list(favorites, item("ETHUSDT")).unwrap();
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
        store.add_to_list(favorites, item("BTCUSDT")).unwrap();
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
fn rows_and_flags_saved_under_two_spellings_of_one_market_load_as_one() {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    {
        let mut favorites = Watchlist::new("Favorites");
        favorites.is_favorites = true;
        for symbol in ["BTC", "ETHUSDT", "BTCUSDT"] {
            favorites.add(item(symbol), None).ok();
        }
        // Rows from before the spellings shared a key: both sit in the document.
        favorites.push_unchecked(item("BTCUSDT"));
        let store = shared.lock().unwrap();
        store
            .replace_documents("watchlist", &[favorites])
            .expect("seed lists");
        store
            .set_setting(
                "watchlist.flags",
                &Flags::from([
                    ("BINANCE:BTC".to_owned(), WatchlistFlag::Blue),
                    ("BINANCE:ETHUSDT".to_owned(), WatchlistFlag::Red),
                ]),
            )
            .expect("seed flags");
    }
    let loaded = WatchlistStore::new(shared.clone());
    let favorites = loaded
        .lists()
        .iter()
        .find(|list| list.is_favorites)
        .unwrap();
    let symbols: Vec<&str> = favorites
        .instruments()
        .map(|item| item.instrument.symbol.as_str())
        .collect();
    assert_eq!(symbols, ["BTC", "ETHUSDT"], "the first BTC row stays, once");
    assert_eq!(
        loaded.flags().get("BINANCE:BTCUSDT"),
        Some(&WatchlistFlag::Blue),
        "the flag follows the market"
    );
    assert!(!loaded.flags().contains_key("BINANCE:BTC"));
    // And it was written back, so the next start has nothing to repair.
    let again = WatchlistStore::new(shared);
    assert_eq!(again.flags(), loaded.flags());
    assert_eq!(
        again
            .lists()
            .iter()
            .find(|list| list.is_favorites)
            .unwrap()
            .instruments()
            .count(),
        2
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
    assert_eq!(
        watchlists.create_list("Unsaved", None),
        Err(WatchlistError::SaveFailed)
    );
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

fn seeded() -> (WatchlistStore, EntryId, SharedStore) {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    let store = WatchlistStore::new(shared.clone());
    let favorites = store.favorites();
    (store, favorites, shared)
}

#[test]
fn a_refused_change_says_why_and_leaves_the_list_as_it_was() {
    let (mut store, favorites, _) = seeded();
    assert_eq!(store.add_to_list(favorites, item("BTCUSDT")), Ok(true));
    let before = store.lists().to_vec();
    assert!(matches!(
        store.add_to_list(favorites, item("btc")),
        Err(WatchlistError::Duplicate(_))
    ));
    assert_eq!(
        store.add_to_list(9, item("ETHUSDT")),
        Err(WatchlistError::NotFound)
    );
    assert_eq!(
        store.remove_entry(favorites, 12345),
        Err(WatchlistError::NotFound)
    );
    assert_eq!(
        store.add_section(favorites, "   "),
        Err(WatchlistError::EmptyName)
    );
    assert_eq!(store.lists(), before.as_slice());
}

#[test]
fn a_change_that_changes_nothing_writes_nothing() {
    let (mut store, favorites, shared) = seeded();
    store.add_to_list(favorites, item("BTCUSDT")).unwrap();
    // Any write now fails: an unchanged toggle must not even try.
    shared
        .lock()
        .unwrap()
        .connection
        .execute_batch(
            "CREATE TRIGGER no_writes BEFORE DELETE ON watchlist
             BEGIN SELECT RAISE(ABORT, 'written'); END;",
        )
        .unwrap();
    let sort = store.lists()[0].display.sort;
    assert_eq!(store.set_sort(favorites, sort), Ok(false));
    assert_eq!(
        store.set_shows_description(favorites, store.lists()[0].display.shows_description),
        Ok(false)
    );
    assert_eq!(store.rename_list(favorites, "Favorites"), Ok(false));
    let id = store.lists()[0].instruments().next().unwrap().id;
    assert_eq!(store.move_instruments(favorites, &[id], None), Ok(false));
    assert_eq!(store.last_error(), None, "nothing was written");
    // A real change does try, and reports the failed save.
    assert_eq!(
        store.rename_list(favorites, "Other"),
        Err(WatchlistError::SaveFailed)
    );
    assert_eq!(
        store.lists()[0].name,
        "Favorites",
        "the unsaved name is not kept"
    );
}

#[test]
fn names_are_cleaned_and_bounded() {
    let (mut store, favorites, _) = seeded();
    let long = "x".repeat(crate::model::watchlist::MAX_NAME_CHARS);
    assert!(store.create_list(&long, None).is_ok());
    assert_eq!(
        store.create_list(&format!("{long}x"), None),
        Err(WatchlistError::NameTooLong)
    );
    assert_eq!(
        store.create_list("a\u{7}b", None),
        Err(WatchlistError::InvalidName)
    );
    assert_eq!(
        store.create_list("  ", None),
        Err(WatchlistError::EmptyName)
    );
    let id = store.create_list("  Two\n words\t here ", None).unwrap();
    let name = store
        .lists()
        .iter()
        .find(|list| list.id == id)
        .unwrap()
        .name
        .clone();
    assert_eq!(name, "Two words here");
    assert_eq!(
        store.rename_list(favorites, ""),
        Err(WatchlistError::EmptyName)
    );
    assert_eq!(
        store.duplicate_list(favorites, "\u{0}"),
        Err(WatchlistError::InvalidName)
    );
}

#[test]
fn only_an_edit_of_the_content_moves_the_timestamp() {
    let (mut store, favorites, _) = seeded();
    let stamp = |store: &WatchlistStore| store.lists()[0].updated_at_ms;
    store
        .lists
        .iter_mut()
        .for_each(|list| list.updated_at_ms = 1);
    store
        .set_sort(favorites, crate::model::watchlist::WatchlistSort::default())
        .unwrap();
    store
        .set_column(
            favorites,
            crate::model::watchlist::WatchlistColumn::Volume,
            true,
        )
        .unwrap();
    assert_eq!(stamp(&store), 1, "display settings are not an edit");
    store.add_to_list(favorites, item("BTCUSDT")).unwrap();
    let added = stamp(&store);
    assert!(added > 1);
    let _ = store.add_to_list(favorites, item("btc"));
    assert_eq!(stamp(&store), added, "a refused add is not an edit");
    store.rename_list(favorites, "Mine").unwrap();
    assert!(stamp(&store) >= added);
}

#[test]
fn a_failed_import_changes_nothing_and_a_full_list_keeps_what_fit() {
    let (mut store, favorites, _) = seeded();
    let text: String = (0..crate::model::watchlist::MAX_ENTRIES_PER_LIST + 50)
        .map(|index| format!("BINANCE:S{index:04}USDT\n"))
        .collect();
    let report = store.import_into(favorites, &text).unwrap();
    assert_eq!(report.added, crate::model::watchlist::MAX_ENTRIES_PER_LIST);
    assert!(report.truncated);
    assert!(report.summary().contains("stopped early"));
    assert_eq!(
        store.add_to_list(favorites, item("EXTRAUSDT")),
        Err(WatchlistError::ListFull)
    );
    assert_eq!(
        store.import_into(77, "BINANCE:BTCUSDT"),
        Err(WatchlistError::NotFound)
    );
}

#[test]
fn the_list_count_is_bounded() {
    let (mut store, _, _) = seeded();
    while store.lists().len() < MAX_LISTS {
        store.create_list("L", None).unwrap();
    }
    assert_eq!(
        store.create_list("one more", None),
        Err(WatchlistError::TooManyLists)
    );
    let first = store.lists()[0].id;
    assert_eq!(
        store.duplicate_list(first, "copy"),
        Err(WatchlistError::TooManyLists)
    );
}

#[test]
fn a_second_favorites_list_and_a_repeated_row_id_are_repaired_on_load() {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    {
        let mut first = Watchlist::new("Favorites");
        first.is_favorites = true;
        let mut second = Watchlist::new("Also favorites");
        second.is_favorites = true;
        let a = item("BTCUSDT");
        let mut b = item("ETHUSDT");
        b.id = a.id;
        second.push_unchecked(a);
        second.push_unchecked(b);
        shared
            .lock()
            .unwrap()
            .replace_documents("watchlist", &[first, second])
            .unwrap();
    }
    let loaded = WatchlistStore::new(shared.clone());
    assert!(!loaded.load_failed());
    assert_eq!(
        loaded
            .lists()
            .iter()
            .filter(|list| list.is_favorites)
            .count(),
        1
    );
    assert_eq!(
        loaded.lists()[1].name,
        "Also favorites",
        "nothing was deleted"
    );
    assert!(loaded.lists().iter().all(|list| list.validate().is_empty()));
    assert_eq!(loaded.lists()[1].instruments().count(), 2);
}

#[test]
fn edit_list_is_one_change_that_reports_each_market() {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    let mut store = WatchlistStore::new(shared.clone());
    let id = store.create_list("Majors", Some(item("BTCUSDT"))).unwrap();
    let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
    let sol = InstrumentId::new(DataSource::Binance, "SOLUSDT");

    let (edit, changed) = store
        .edit_list(
            id,
            ListChanges {
                rename: Some("  Core  ".into()),
                add: vec![(item("BTCUSDT"), Place::End), (item("ETHUSDT"), Place::End)],
                remove: vec![sol.clone()],
                ..ListChanges::default()
            },
        )
        .unwrap();
    assert!(changed);
    assert!(edit.renamed);
    assert_eq!(edit.already_present, std::slice::from_ref(&btc));
    assert_eq!(edit.added.len(), 1);
    assert_eq!(edit.not_present, [sol]);
    let list = store.lists().iter().find(|l| l.id == id).unwrap();
    assert_eq!(list.name, "Core");
    assert_eq!(list.instruments().count(), 2);

    // Asking for what is already so writes nothing.
    let (edit, changed) = store
        .edit_list(
            id,
            ListChanges {
                rename: Some("Core".into()),
                add: vec![(item("BTCUSDT"), Place::End)],
                ..ListChanges::default()
            },
        )
        .unwrap();
    assert!(!changed);
    assert!(!edit.renamed);
}

#[test]
fn a_refused_edit_changes_nothing() {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    let mut store = WatchlistStore::new(shared);
    let id = store.create_list("Majors", Some(item("BTCUSDT"))).unwrap();
    let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
    // A name that is refused after the markets were valid: nothing is removed or added.
    let bad = store.edit_list(
        id,
        ListChanges {
            rename: Some("   ".into()),
            add: vec![(item("ETHUSDT"), Place::End)],
            remove: vec![btc],
            ..ListChanges::default()
        },
    );
    assert_eq!(bad, Err(WatchlistError::EmptyName));
    let list = store.lists().iter().find(|l| l.id == id).unwrap();
    assert_eq!(list.name, "Majors");
    assert_eq!(list.instruments().count(), 1);
    assert_eq!(
        store.edit_list(999, ListChanges::default()),
        Err(WatchlistError::NotFound)
    );
}

#[test]
fn sections_can_be_made_filled_renamed_and_deleted_in_one_change() {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    let mut store = WatchlistStore::new(shared);
    let id = store.create_list("Majors", Some(item("BTCUSDT"))).unwrap();
    let sol = InstrumentId::new(DataSource::Binance, "SOLUSDT");

    // A section made by this change is usable by title in the same change.
    let (edit, changed) = store
        .edit_list(
            id,
            ListChanges {
                add_sections: vec!["Alts".into(), "Stables".into()],
                add: vec![
                    (item("ETHUSDT"), Place::Section("alts".into())),
                    (item("USDCUSDT"), Place::Section("Stables".into())),
                ],
                moves: vec![
                    (
                        InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                        Place::Top,
                    ),
                    (sol, Place::Top),
                ],
                ..ListChanges::default()
            },
        )
        .unwrap();
    assert!(changed);
    assert_eq!(edit.sections_added, ["Alts", "Stables"]);
    assert_eq!(edit.added.len(), 2);
    assert_eq!(edit.moved.len(), 1);
    assert_eq!(edit.not_present.len(), 1, "SOLUSDT was never in the list");
    let list = store.lists().iter().find(|l| l.id == id).unwrap();
    let order: Vec<String> = list
        .entries
        .iter()
        .map(|entry| match entry {
            WatchlistEntry::Section(section) => format!("[{}]", section.title),
            WatchlistEntry::Instrument(item) => item.instrument.key(),
        })
        .collect();
    assert_eq!(
        order,
        [
            "BINANCE:BTCUSDT",
            "[Alts]",
            "BINANCE:ETHUSDT",
            "[Stables]",
            "BINANCE:USDCUSDT"
        ]
    );

    // Rename by title, move a market into it, delete the other section.
    let (edit, _) = store
        .edit_list(
            id,
            ListChanges {
                rename_sections: vec![("Alts".into(), "Altcoins".into())],
                moves: vec![(
                    InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                    Place::Section("Altcoins".into()),
                )],
                delete_sections: vec!["Stables".into()],
                ..ListChanges::default()
            },
        )
        .unwrap();
    assert_eq!(edit.sections_renamed, ["Altcoins"]);
    assert_eq!(edit.sections_deleted, ["Stables"]);
    let list = store.lists().iter().find(|l| l.id == id).unwrap();
    assert_eq!(list.sections().count(), 1);
    assert_eq!(list.sections().next().unwrap().title, "Altcoins");
    // Deleting a header keeps its markets: USDC joined the section above.
    assert_eq!(list.instruments().count(), 3);
}

#[test]
fn an_unknown_or_ambiguous_section_refuses_the_whole_change() {
    let shared = SharedStore::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("store"),
    ));
    let mut store = WatchlistStore::new(shared);
    let id = store.create_list("Mixed", None).unwrap();
    store
        .edit_list(
            id,
            ListChanges {
                add_sections: vec!["Same".into(), "same".into()],
                ..ListChanges::default()
            },
        )
        .unwrap();
    let ambiguous = store.edit_list(
        id,
        ListChanges {
            add: vec![(item("BTCUSDT"), Place::Section("SAME".into()))],
            ..ListChanges::default()
        },
    );
    assert!(matches!(
        ambiguous,
        Err(WatchlistError::AmbiguousSection(_))
    ));
    let unknown = store.edit_list(
        id,
        ListChanges {
            add_sections: vec!["Extra".into()],
            add: vec![(item("BTCUSDT"), Place::Section("Nope".into()))],
            ..ListChanges::default()
        },
    );
    assert_eq!(unknown, Err(WatchlistError::NotFound));
    let list = store.lists().iter().find(|l| l.id == id).unwrap();
    assert_eq!(
        list.sections().count(),
        2,
        "no 'Extra' section and no market"
    );
    assert_eq!(list.instruments().count(), 0);
    // The section id works too, even when titles collide.
    let first = list.sections().next().unwrap().id;
    store
        .edit_list(
            id,
            ListChanges {
                add: vec![(item("BTCUSDT"), Place::Section(first.to_string()))],
                ..ListChanges::default()
            },
        )
        .unwrap();
}
