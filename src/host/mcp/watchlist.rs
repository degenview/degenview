use crate::host::mcp::confirmation;
use crate::mcp::bridge::{McpError, McpReply};
use crate::mcp::confirm::Summary;
use crate::mcp::market::{
    self, WatchlistChangeResult, WatchlistCommand, WatchlistPage, WatchlistsResult,
};
use crate::model::InstrumentId;
use crate::model::watchlist::{Watchlist, WatchlistError, WatchlistInstrument};
use crate::store::watchlist::{ListChanges, ListEdit, Place, WatchlistStore};
use eframe::egui;
use std::sync::{Mutex, MutexGuard};

fn lock(watchlists: &Mutex<WatchlistStore>) -> Result<MutexGuard<'_, WatchlistStore>, McpError> {
    watchlists
        .lock()
        .map_err(|_| McpError::AppUnavailable("Watchlists are unavailable right now".into()))
}

/// Runs a watchlist command on the app thread, against the store the sidebar uses.
pub fn run(
    command: WatchlistCommand,
    watchlists: &Mutex<WatchlistStore>,
    ctx: &egui::Context,
) -> Result<McpReply, McpError> {
    let mut store = lock(watchlists)?;
    let changed = match command {
        WatchlistCommand::List => return list_watchlists(&store).map(McpReply::Watchlists),
        WatchlistCommand::Get {
            watchlist_id,
            offset,
            limit,
        } => {
            return get_watchlist(&store, watchlist_id, offset, limit)
                .map(|page| McpReply::Watchlist(Box::new(page)));
        }
        WatchlistCommand::Create { name, markets } => create_watchlist(&mut store, &name, markets)?,
        WatchlistCommand::Update {
            watchlist_id,
            changes,
        } => update_watchlist(&mut store, watchlist_id, *changes)?,
        WatchlistCommand::Delete { watchlist_id } => delete_watchlist(&mut store, watchlist_id)?,
    };
    ctx.request_repaint();
    Ok(McpReply::WatchlistChanged(Box::new(changed)))
}

/// What a watchlist command the user is asked about would do (see [`confirmation`]).
pub fn describe(
    command: &WatchlistCommand,
    watchlists: &Mutex<WatchlistStore>,
) -> Result<Summary, McpError> {
    match command {
        WatchlistCommand::Delete { watchlist_id } => {
            confirmation::delete_watchlist(&*lock(watchlists)?, *watchlist_id)
        }
        _ => confirmation::plainly(command),
    }
}

fn watchlist_error(error: WatchlistError) -> McpError {
    match error {
        WatchlistError::NotFound => McpError::NotFound(error.to_string()),
        WatchlistError::Duplicate(_) => McpError::Conflict(error.to_string()),
        WatchlistError::EmptyName
        | WatchlistError::NameTooLong
        | WatchlistError::InvalidName
        | WatchlistError::AmbiguousSection(_) => McpError::InvalidArgument(error.to_string()),
        WatchlistError::ListFull | WatchlistError::TooManyLists => {
            McpError::ResourceLimit(error.to_string())
        }
        WatchlistError::PersistenceUnavailable | WatchlistError::SaveFailed => {
            McpError::AppUnavailable(error.to_string())
        }
    }
}

fn readable_watchlists(store: &WatchlistStore) -> Result<&[Watchlist], McpError> {
    if store.load_failed() {
        return Err(McpError::AppUnavailable(
            "Watchlists could not be read, so they are unavailable".into(),
        ));
    }
    Ok(store.lists())
}

pub(super) fn watchlist_by_id(store: &WatchlistStore, id: u64) -> Result<&Watchlist, McpError> {
    readable_watchlists(store)?
        .iter()
        .find(|list| list.id == id)
        .ok_or_else(|| {
            McpError::NotFound(
                "No such watchlist: use a watchlist_id from degenview_list_watchlists".into(),
            )
        })
}

pub fn list_watchlists(store: &WatchlistStore) -> Result<WatchlistsResult, McpError> {
    Ok(WatchlistsResult {
        watchlists: readable_watchlists(store)?
            .iter()
            .map(market::watchlist_summary)
            .collect(),
    })
}

pub fn get_watchlist(
    store: &WatchlistStore,
    id: u64,
    offset: usize,
    limit: usize,
) -> Result<WatchlistPage, McpError> {
    let list = watchlist_by_id(store, id)?;
    Ok(WatchlistPage {
        detail: market::watchlist_detail(list, offset, limit, None),
        markets: market::page_markets(list, offset, limit),
    })
}

fn change_result(
    status: &str,
    store: &WatchlistStore,
    id: u64,
    edit: ListEdit,
) -> WatchlistChangeResult {
    let key = |markets: Vec<InstrumentId>| markets.iter().map(InstrumentId::key).collect();
    let list = store.lists().iter().find(|list| list.id == id);
    WatchlistChangeResult {
        status: status.to_owned(),
        renamed_to: edit
            .renamed
            .then(|| list.map(|list| list.name.clone()))
            .flatten(),
        watchlist: list.map(market::watchlist_summary),
        added: key(edit.added),
        already_present: key(edit.already_present),
        removed: key(edit.removed),
        not_present: key(edit.not_present),
        moved: key(edit.moved),
        sections_added: edit.sections_added,
        sections_renamed: edit.sections_renamed,
        sections_deleted: edit.sections_deleted,
    }
}

pub fn create_watchlist(
    store: &mut WatchlistStore,
    name: &str,
    markets: Vec<WatchlistInstrument>,
) -> Result<WatchlistChangeResult, McpError> {
    readable_watchlists(store)?;
    let id = store.create_list(name, None).map_err(watchlist_error)?;
    let edit = if markets.is_empty() {
        ListEdit::default()
    } else {
        match store.edit_list(
            id,
            ListChanges {
                add: markets.into_iter().map(|item| (item, Place::End)).collect(),
                ..ListChanges::default()
            },
        ) {
            Ok((edit, _)) => edit,
            Err(error) => {
                // The list was meant to be created with its markets or not at all.
                let _ = store.delete_list(id);
                return Err(watchlist_error(error));
            }
        }
    };
    Ok(change_result("created", store, id, edit))
}

pub fn update_watchlist(
    store: &mut WatchlistStore,
    id: u64,
    changes: ListChanges,
) -> Result<WatchlistChangeResult, McpError> {
    watchlist_by_id(store, id)?;
    let (edit, changed) = store.edit_list(id, changes).map_err(watchlist_error)?;
    Ok(change_result(
        if changed { "updated" } else { "unchanged" },
        store,
        id,
        edit,
    ))
}

pub fn delete_watchlist(
    store: &mut WatchlistStore,
    id: u64,
) -> Result<WatchlistChangeResult, McpError> {
    let summary = market::watchlist_summary(watchlist_by_id(store, id)?);
    if summary.is_favorites {
        return Err(McpError::Conflict(
            "Favorites cannot be deleted; remove its markets or rename it instead".into(),
        ));
    }
    if !store.delete_list(id).map_err(watchlist_error)? {
        return Err(McpError::Conflict(
            "That watchlist cannot be deleted".into(),
        ));
    }
    Ok(WatchlistChangeResult {
        status: "deleted".into(),
        watchlist: None,
        added: Vec::new(),
        already_present: Vec::new(),
        removed: Vec::new(),
        not_present: Vec::new(),
        renamed_to: None,
        moved: Vec::new(),
        sections_added: Vec::new(),
        sections_renamed: Vec::new(),
        sections_deleted: Vec::new(),
    })
}
