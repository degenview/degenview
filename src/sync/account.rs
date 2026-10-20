//! Enabling sync, binding a database to one iCloud account, and what happens when the account
//! changes. The rule that matters: data never moves between accounts without the user choosing
//! how. A database is bound to exactly one account; any other account is a pause, not a sync.

use super::record::sha256_hex;
use crate::store::Store;
use crate::store::sync::SyncState;
use std::path::{Path, PathBuf};

pub const CONTAINER: &str = "iCloud.es.nico.degenview";

/// What a database keeps of an iCloud account: a hash, never the identifier itself.
pub fn hash_user(container: &str, user_record_name: &str) -> String {
    sha256_hex(format!("v1|{container}|{user_record_name}").as_bytes())
}

/// How the user wants to resolve a different account being signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountChoice {
    /// Stop syncing; keep everything here as it is.
    KeepLocalOnly,
    /// Use the new account's data. This Mac's own copy is stashed first and stays recoverable.
    UseThisAccountsData,
    /// Combine this Mac's data with the new account's, nothing discarded on either side.
    MergeIntoThisAccount,
}

/// Turns sync on for this database. Creates the device identity on first use. Nothing is
/// uploaded yet: the first-time merge decides what goes where.
pub fn enable(
    store: &Store,
    device_name: &str,
    hardware_hash: &str,
    environment: &str,
) -> rusqlite::Result<SyncState> {
    let mut state = store.ensure_sync_state(device_name, hardware_hash)?;
    state.enabled = true;
    state.container_id = Some(CONTAINER.to_string());
    state.environment = Some(environment.to_string());
    store.save_sync_state(&state)?;
    Ok(state)
}

/// Stops all sync activity. Local data, the iCloud binding and anything waiting to upload are
/// kept, and nothing is deleted from iCloud; turning sync back on resumes where it left off.
pub fn disable(store: &Store) -> rusqlite::Result<()> {
    if let Some(mut state) = store.sync_state()? {
        state.enabled = false;
        store.save_sync_state(&state)?;
    }
    Ok(())
}

/// Binds this database to the account the user just agreed to use. The first-time merge still
/// has to run before anything uploads.
pub fn bind(store: &Store, user_hash: &str, now_ms: i64) -> rusqlite::Result<()> {
    let Some(mut state) = store.sync_state()? else {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    };
    state.account_hash = Some(user_hash.to_string());
    state.account_bound_ms = Some(now_ms);
    state.initial_sync_done = false;
    state.engine_state = None;
    state.engine_state_account_hash = None;
    store.save_sync_state(&state)
}

/// What resolving an account change did, for the interface to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// Where this Mac's data was stashed, when the choice called for it.
    pub backup: Option<PathBuf>,
    /// The first-time merge must run against the new account before uploads resume.
    pub needs_join: bool,
}

/// Applies the user's choice after a different Apple Account was signed in. Until this runs,
/// nothing is uploaded. Every path keeps this Mac's data recoverable; none of them deletes
/// anything from either account.
pub fn resolve_account_change(
    store: &Store,
    choice: AccountChoice,
    new_user_hash: &str,
    backups_dir: &Path,
    now_ms: i64,
) -> rusqlite::Result<Resolution> {
    match choice {
        AccountChoice::KeepLocalOnly => {
            if let Some(mut state) = store.sync_state()? {
                state.enabled = false;
                state.account_hash = None;
                state.account_bound_ms = None;
                state.initial_sync_done = false;
                state.engine_state = None;
                state.engine_state_account_hash = None;
                store.save_sync_state(&state)?;
            }
            store.reset_sync_data()?;
            Ok(Resolution {
                backup: None,
                needs_join: false,
            })
        }
        AccountChoice::UseThisAccountsData => {
            let backup = stash(store, backups_dir, now_ms)?;
            store.reset_sync_data()?;
            store.clear_replaceable_collections()?;
            bind(store, new_user_hash, now_ms)?;
            Ok(Resolution {
                backup: Some(backup),
                needs_join: true,
            })
        }
        AccountChoice::MergeIntoThisAccount => {
            store.reset_sync_data()?;
            bind(store, new_user_hash, now_ms)?;
            Ok(Resolution {
                backup: None,
                needs_join: true,
            })
        }
    }
}

/// After iCloud confirmed that this Mac's data there was deleted: stop syncing and forget the
/// binding and everything learned about the cloud. Local data is untouched, and turning sync on
/// again starts a fresh first-time merge.
pub fn after_cloud_delete(store: &Store) -> rusqlite::Result<()> {
    if let Some(mut state) = store.sync_state()? {
        state.enabled = false;
        state.account_hash = None;
        state.account_bound_ms = None;
        state.dataset_id = None;
        state.initial_sync_done = false;
        state.engine_state = None;
        state.engine_state_account_hash = None;
        store.save_sync_state(&state)?;
    }
    store.reset_sync_data()
}

/// Another Mac deleted DegenView's data in iCloud. Either upload this Mac's data again as a fresh
/// start (`reupload`), or stop syncing. Nothing local is removed either way.
pub fn resolve_remote_deletion(store: &Store, reupload: bool) -> rusqlite::Result<()> {
    store.reset_sync_data()?;
    if let Some(mut state) = store.sync_state()? {
        state.initial_sync_done = false;
        state.engine_state = None;
        state.engine_state_account_hash = None;
        state.dataset_id = None;
        state.enabled = reupload;
        if !reupload {
            state.account_hash = None;
            state.account_bound_ms = None;
        }
        store.save_sync_state(&state)?;
    }
    Ok(())
}

/// A copy of the whole database, named by time, that opens as an ordinary DegenView database.
pub fn stash(store: &Store, backups_dir: &Path, now_ms: i64) -> rusqlite::Result<PathBuf> {
    std::fs::create_dir_all(backups_dir)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let mut path = backups_dir.join(format!("degenview-before-icloud-{now_ms}.sqlite"));
    let mut n = 2;
    while path.exists() {
        path = backups_dir.join(format!("degenview-before-icloud-{now_ms}-{n}.sqlite"));
        n += 1;
    }
    store.backup_to(&path)?;
    Ok(path)
}

/// A database copied to another Mac (Migration Assistant, a restored backup) carries a device
/// identity that belongs to the original. If the hardware differs, mint a new identity and drop
/// the engine checkpoint, which is per device. Returns whether the identity changed.
pub fn check_device_identity(store: &Store, hardware_hash: &str) -> rusqlite::Result<bool> {
    let Some(state) = store.sync_state()? else {
        return Ok(false);
    };
    if state.hardware_hash == hardware_hash {
        return Ok(false);
    }
    store.reset_sync_device(hardware_hash)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "degenview-sync-account-{tag}-{}-{}",
            std::process::id(),
            crate::store::sync::new_device_id()
        ))
    }

    #[test]
    fn the_account_hash_is_stable_and_not_the_identifier() {
        let a = hash_user(CONTAINER, "_abc123");
        assert_eq!(a, hash_user(CONTAINER, "_abc123"));
        assert_ne!(a, hash_user(CONTAINER, "_abc124"));
        assert_ne!(a, hash_user("iCloud.other", "_abc123"));
        assert!(!a.contains("abc123"));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn enabling_creates_an_identity_and_uploads_nothing() {
        let store = Store::open_in_memory().unwrap();
        let state = enable(&store, "Studio", "hw", "Development").unwrap();
        assert!(state.enabled);
        assert_eq!(state.container_id.as_deref(), Some(CONTAINER));
        assert_eq!(state.environment.as_deref(), Some("Development"));
        assert!(state.account_hash.is_none());
        assert!(!state.initial_sync_done);
        assert!(store.outbox().unwrap().is_empty());
    }

    #[test]
    fn enabling_again_keeps_the_device_identity() {
        let store = Store::open_in_memory().unwrap();
        let first = enable(&store, "Studio", "hw", "Production").unwrap();
        disable(&store).unwrap();
        assert!(!store.sync_enabled());
        let second = enable(&store, "Renamed", "hw", "Production").unwrap();
        assert_eq!(first.device_id, second.device_id);
    }

    #[test]
    fn disabling_keeps_the_binding_and_the_pending_changes() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        bind(&store, "acct", 5).unwrap();
        store.set_setting("appTheme", &"dark").unwrap();
        assert_eq!(store.outbox().unwrap().len(), 1);
        disable(&store).unwrap();
        let state = store.sync_state().unwrap().unwrap();
        assert_eq!(state.account_hash.as_deref(), Some("acct"));
        assert_eq!(store.outbox().unwrap().len(), 1, "pending changes are kept");
        assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
        // While off, edits are not tracked; reconcile picks them up when sync returns.
        store.set_setting("appTheme", &"light").unwrap();
        assert_eq!(store.outbox().unwrap().len(), 1);
    }

    #[test]
    fn binding_resets_the_first_time_merge_and_the_checkpoint() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        let mut state = store.sync_state().unwrap().unwrap();
        state.initial_sync_done = true;
        state.engine_state = Some(vec![1]);
        store.save_sync_state(&state).unwrap();
        bind(&store, "new-account", 9).unwrap();
        let state = store.sync_state().unwrap().unwrap();
        assert_eq!(state.account_hash.as_deref(), Some("new-account"));
        assert_eq!(state.account_bound_ms, Some(9));
        assert!(!state.initial_sync_done);
        assert!(state.engine_state.is_none());
    }

    #[test]
    fn binding_without_enabling_is_an_error() {
        let store = Store::open_in_memory().unwrap();
        assert!(bind(&store, "x", 1).is_err());
    }

    #[test]
    fn a_cloned_database_gets_a_new_identity_only_when_the_hardware_differs() {
        let store = Store::open_in_memory().unwrap();
        let original = enable(&store, "Studio", "hw-a", "Production").unwrap();
        assert!(!check_device_identity(&store, "hw-a").unwrap());
        assert!(check_device_identity(&store, "hw-b").unwrap());
        let cloned = store.sync_state().unwrap().unwrap();
        assert_ne!(cloned.device_id, original.device_id);
        assert_eq!(cloned.hardware_hash, "hw-b");
        assert!(!check_device_identity(&store, "hw-b").unwrap());
        let empty = Store::open_in_memory().unwrap();
        assert!(!check_device_identity(&empty, "hw").unwrap());
    }

    fn with_data(store: &Store) {
        store.set_setting("appTheme", &"dark").unwrap();
        store
            .set_setting("mcp.settings", &serde_json::json!({"port": 1}))
            .unwrap();
    }

    #[test]
    fn keeping_local_only_stops_syncing_and_touches_no_data() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        bind(&store, "old", 1).unwrap();
        with_data(&store);
        let root = dir("keep");
        let resolution =
            resolve_account_change(&store, AccountChoice::KeepLocalOnly, "new", &root, 7).unwrap();
        assert_eq!(
            resolution,
            Resolution {
                backup: None,
                needs_join: false
            }
        );
        let state = store.sync_state().unwrap().unwrap();
        assert!(!state.enabled);
        assert!(state.account_hash.is_none());
        assert!(store.outbox().unwrap().is_empty());
        assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
        assert!(
            !root.exists(),
            "no backup is needed when nothing is replaced"
        );
    }

    #[test]
    fn merging_rebinds_and_forgets_the_old_accounts_links_without_touching_data() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        bind(&store, "old", 1).unwrap();
        with_data(&store);
        assert!(!store.outbox().unwrap().is_empty());
        let root = dir("merge");
        let resolution =
            resolve_account_change(&store, AccountChoice::MergeIntoThisAccount, "new", &root, 7)
                .unwrap();
        assert!(resolution.needs_join);
        assert!(resolution.backup.is_none());
        let state = store.sync_state().unwrap().unwrap();
        assert_eq!(state.account_hash.as_deref(), Some("new"));
        assert!(state.enabled);
        assert!(
            store.outbox().unwrap().is_empty(),
            "the old account's queue is dropped"
        );
        assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
    }

    #[test]
    fn using_the_new_accounts_data_stashes_a_recoverable_copy_first() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        bind(&store, "old", 1).unwrap();
        store
            .replace_documents(
                "chart_template",
                &[crate::store::sync::RawDoc(
                    serde_json::json!({"id": 1500, "name": "Mine"}),
                )],
            )
            .unwrap();
        with_data(&store);
        let root = dir("use");
        let resolution =
            resolve_account_change(&store, AccountChoice::UseThisAccountsData, "new", &root, 7)
                .unwrap();
        let backup = resolution.backup.expect("a backup was made");
        assert!(resolution.needs_join);
        // The live database no longer has the templates...
        assert!(store.raw_rows("chart_template").unwrap().is_empty());
        // ...but the backup opens as a normal database and still does.
        let restored = Store::open(&backup).unwrap();
        assert_eq!(restored.raw_rows("chart_template").unwrap().len(), 1);
        assert_eq!(
            restored.setting::<String>("appTheme").as_deref(),
            Some("dark")
        );
        // Local-only data was not part of the replacement.
        assert!(store.setting::<serde_json::Value>("mcp.settings").is_some());
        // Nothing was queued to delete anything from either account.
        assert!(store.outbox().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn after_a_cloud_delete_sync_is_off_unbound_and_local_data_is_untouched() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        bind(&store, "acct", 1).unwrap();
        let mut state = store.sync_state().unwrap().unwrap();
        state.initial_sync_done = true;
        state.engine_state = Some(vec![1]);
        store.save_sync_state(&state).unwrap();
        with_data(&store);
        after_cloud_delete(&store).unwrap();
        let state = store.sync_state().unwrap().unwrap();
        assert!(!state.enabled);
        assert!(state.account_hash.is_none());
        assert!(!state.initial_sync_done);
        assert!(state.engine_state.is_none());
        assert!(store.outbox().unwrap().is_empty());
        assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
    }

    #[test]
    fn after_another_mac_deletes_the_cloud_data_this_mac_can_start_over_or_stop() {
        let store = Store::open_in_memory().unwrap();
        enable(&store, "Studio", "hw", "Production").unwrap();
        bind(&store, "acct", 1).unwrap();
        with_data(&store);
        resolve_remote_deletion(&store, true).unwrap();
        let again = store.sync_state().unwrap().unwrap();
        assert!(
            again.enabled,
            "still syncing, bound, and about to merge afresh"
        );
        assert_eq!(again.account_hash.as_deref(), Some("acct"));
        assert!(!again.initial_sync_done);
        assert!(store.outbox().unwrap().is_empty());
        resolve_remote_deletion(&store, false).unwrap();
        let off = store.sync_state().unwrap().unwrap();
        assert!(!off.enabled);
        assert!(off.account_hash.is_none());
        assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
    }

    #[test]
    fn two_stashes_in_the_same_millisecond_do_not_overwrite_each_other() {
        let store = Store::open_in_memory().unwrap();
        let root = dir("twice");
        let first = stash(&store, &root, 5).unwrap();
        let second = stash(&store, &root, 5).unwrap();
        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
