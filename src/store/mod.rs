//! SQLite persistence for user-authored data, mirroring the Swift `AppDatabase`.
//!
//! Document tables hold ordered lists of JSON payloads keyed by a stable id; the
//! `setting` table holds single keyed values. Payloads stay JSON so nested models
//! can evolve through serde defaults instead of schema migrations.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub mod alert;
pub mod asset_store;
pub mod chart_template;
pub mod drawing_template;
pub mod mcp;
pub mod paper;
pub mod pine;
pub mod portfolio;
pub mod saved_view;
pub mod sync;
pub mod watchlist;
pub mod webhook;

/// A store shared with the UI thread behind a mutex.
pub type SharedStore = Arc<Mutex<Store>>;

/// Opens the app's on-disk store. A storage failure must be visible to the caller.
pub fn open_default() -> Result<SharedStore, Box<dyn std::error::Error>> {
    let dir = data_dir().ok_or("cannot locate the application data directory")?;
    std::fs::create_dir_all(&dir)?;
    Ok(Arc::new(Mutex::new(Store::open(
        &dir.join("degenview.sqlite"),
    )?)))
}

const APP_DIR: &str = "es.nico.degenview";

/// The app's data directory: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows,
/// `$XDG_DATA_HOME` (or `~/.local/share`) elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(env("HOME")?).join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(env("APPDATA").or_else(|| env("LOCALAPPDATA"))?)
    } else {
        match env("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env("HOME")?).join(".local/share"),
        }
    };
    Some(base.join(APP_DIR))
}

/// Overrides `cache_dir` (dev runs and manual verification against a scratch directory).
pub const CACHE_DIR_ENV: &str = "DEGENVIEW_CACHE_DIR";

/// Where regenerable network data lives (icons, lookups, FX rates) — separate from `data_dir`
/// because the OS may reclaim it and the user may clear it: `~/Library/Caches` on macOS,
/// `%LOCALAPPDATA%` on Windows, `$XDG_CACHE_HOME` (or `~/.cache`) elsewhere.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_from(
        &|name| std::env::var_os(name),
        cfg!(target_os = "macos"),
        cfg!(windows),
    )
}

fn cache_dir_from(
    env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    macos: bool,
    windows: bool,
) -> Option<PathBuf> {
    let env = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(dir) = env(CACHE_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    Some(if macos {
        PathBuf::from(env("HOME")?)
            .join("Library/Caches")
            .join(APP_DIR)
    } else if windows {
        PathBuf::from(env("LOCALAPPDATA").or_else(|| env("APPDATA"))?)
            .join(APP_DIR)
            .join("Cache")
    } else {
        match env("XDG_CACHE_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env("HOME")?).join(".cache"),
        }
        .join(APP_DIR)
    })
}

/// A row in a document table: a stable id plus an ordered JSON payload.
pub trait Document {
    fn document_id(&self) -> String;
}

pub(crate) const DOCUMENT_TABLES: [&str; 13] = [
    "saved_view",
    "chart_template",
    "drawing_template",
    "tab",
    "watchlist",
    "drawing",
    "portfolio",
    "paper_account",
    "price_alert",
    "webhook_endpoint",
    "pine_alert_subscription",
    "mcp_client",
    "drawing_asset",
];

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self { connection };
        store.create_schema()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let store = Self { connection };
        store.create_schema()?;
        Ok(store)
    }

    fn create_schema(&self) -> rusqlite::Result<()> {
        for table in DOCUMENT_TABLES {
            self.connection.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                    id TEXT PRIMARY KEY,
                    position INTEGER NOT NULL,
                    payload TEXT NOT NULL
                );"
            ))?;
        }
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS setting (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS window_group (
                position INTEGER PRIMARY KEY,
                tab_ids TEXT NOT NULL
            );",
        )?;

        // Portfolio ledger. Transaction order is significant to validation, so `position`
        // preserves it; snapshots are append-only history kept in rowid order.
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS portfolio_transaction (
                id TEXT PRIMARY KEY,
                portfolio_id TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                position INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS portfolio_transaction_on_portfolio
                ON portfolio_transaction (portfolio_id, timestamp);
            CREATE TABLE IF NOT EXISTS portfolio_snapshot (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                portfolio_id TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS portfolio_snapshot_on_portfolio
                ON portfolio_snapshot (portfolio_id, timestamp);",
        )?;

        // Paper trading. Collections owned by one account, kept in engine order via the rowid.
        for table in [
            "paper_order",
            "paper_fill",
            "paper_position",
            "paper_order_event",
            "paper_closed_trade",
            "paper_journal",
        ] {
            self.connection.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id TEXT NOT NULL,
                    payload TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS {table}_on_account ON {table} (account_id);"
            ))?;
        }

        // Alerts: fired events (rewritten whole on each snapshot save) and the GUI -> runtime
        // command queue (deleted once applied).
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS alert_event (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                alert_id TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS alert_event_on_alert ON alert_event (alert_id);
            CREATE TABLE IF NOT EXISTS alert_command (
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS alert_command_on_created_at ON alert_command (created_at);",
        )?;

        // Pine deliveries are claimed before notification/webhook dispatch. `dedupe_key` is null
        // for `alert.freq_all` (every execution is distinct) and stable per subscription, call
        // site, bar, and frequency for the once-per-bar modes.
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS pine_alert_event (
                id TEXT PRIMARY KEY,
                subscription_id TEXT NOT NULL,
                dedupe_key TEXT UNIQUE,
                timestamp INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS pine_alert_event_on_subscription
                ON pine_alert_event (subscription_id, timestamp);",
        )?;

        // Webhook delivery attempts. A row is claimed (`pending`) before the request goes out,
        // and the unique key means one trigger reaches one endpoint once, even if two processes
        // both see the trigger. No URL, payload or response body is stored.
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS webhook_delivery (
                id TEXT PRIMARY KEY,
                endpoint_id TEXT NOT NULL,
                source TEXT NOT NULL,
                event_id TEXT,
                timestamp INTEGER NOT NULL,
                state TEXT NOT NULL,
                status_code INTEGER,
                duration REAL,
                error TEXT,
                UNIQUE (source, event_id, endpoint_id)
            );
            CREATE INDEX IF NOT EXISTS webhook_delivery_on_event ON webhook_delivery (event_id);",
        )?;

        self.create_sync_schema()
    }

    // MARK: Documents

    pub fn documents<T: DeserializeOwned>(&self, table: &str) -> Vec<T> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT payload FROM {table} ORDER BY position"))
            .ok();
        statement
            .as_mut()
            .map(|statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| {
                        rows.filter_map(Result::ok)
                            .filter_map(|payload| serde_json::from_str(&payload).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    /// Reads a collection that will be edited and written back. A bad row must stop the edit.
    pub fn documents_strict<T: DeserializeOwned>(&self, table: &str) -> rusqlite::Result<Vec<T>> {
        self.payloads_strict(&format!("SELECT payload FROM {table} ORDER BY position"))
    }

    pub fn replace_documents<T: Serialize + DeserializeOwned + Document>(
        &self,
        table: &str,
        items: &[T],
    ) -> rusqlite::Result<()> {
        // Refuse to rewrite a collection containing rows this version cannot decode.
        // This also protects callers still using the lossy `documents` read.
        self.documents_strict::<T>(table)?;
        let encoded: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(position, item)| {
                let payload = serde_json::to_string(item)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                Ok((item.document_id(), position as i64, payload))
            })
            .collect::<rusqlite::Result<_>>()?;
        // SAVEPOINT also works when a snapshot writer already owns a transaction.
        self.connection
            .execute_batch("SAVEPOINT replace_documents")?;
        let result = (|| {
            // With sync on, the upload intent commits with the rows or not at all.
            let tracked = self.tracked_old_rows(table)?;
            self.connection
                .execute(&format!("DELETE FROM {table}"), [])?;
            let mut statement = self.connection.prepare(&format!(
                "INSERT INTO {table} (id, position, payload) VALUES (?1, ?2, ?3)"
            ))?;
            for (id, position, payload) in &encoded {
                statement.execute(rusqlite::params![id, position, payload])?;
            }
            if let Some(old) = tracked {
                self.track_documents(table, &old, &encoded)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.connection
                .execute_batch("ROLLBACK TO replace_documents")?;
        }
        self.connection.execute_batch("RELEASE replace_documents")?;
        result
    }

    // MARK: Payload rows

    /// Decodes `payload` rows from an arbitrary query, skipping any that no longer decode.
    /// For append-only tables where one bad row must not erase the rest.
    fn payloads<T: DeserializeOwned>(&self, sql: &str) -> Vec<T> {
        let Ok(mut statement) = self.connection.prepare(sql) else {
            return Vec::new();
        };
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .ok()
            .map(|rows| {
                rows.filter_map(Result::ok)
                    .filter_map(|payload| serde_json::from_str(&payload).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Like `payloads`, but a row that no longer decodes fails the read. For stores that
    /// rewrite what they load, where skipping a row would erase it on the next write.
    fn payloads_strict<T: DeserializeOwned>(&self, sql: &str) -> rusqlite::Result<Vec<T>> {
        let mut statement = self.connection.prepare(sql)?;
        let payloads: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        payloads
            .into_iter()
            .map(|payload| {
                serde_json::from_str(&payload).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .collect()
    }

    /// Runs `f` inside one SQLite transaction: a mid-write failure rolls everything back, so a
    /// multi-table replacement (portfolio/paper/alert snapshots) never leaves a partial row set.
    fn in_transaction(&self, f: impl FnOnce() -> rusqlite::Result<()>) -> rusqlite::Result<()> {
        self.connection.execute_batch("BEGIN")?;
        match f() {
            Ok(()) => {
                self.connection.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    // MARK: Settings

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.setting_strict(key).ok().flatten()
    }

    /// A missing key is distinct from a stored value this version cannot decode.
    pub fn setting_strict<T: DeserializeOwned>(&self, key: &str) -> rusqlite::Result<Option<T>> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .transpose()
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> rusqlite::Result<()> {
        let payload = serde_json::to_string(value)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        self.write_setting(key, Some(&payload))
    }

    /// Writes or clears a setting. A portable one also queues its upload in the same savepoint.
    fn write_setting(&self, key: &str, value: Option<&str>) -> rusqlite::Result<()> {
        let tracked = sync::tracks_setting(key) && self.sync_enabled();
        if !tracked {
            return self.apply_setting(key, value);
        }
        self.connection.execute_batch("SAVEPOINT write_setting")?;
        let result = (|| {
            let old: Option<String> = self
                .connection
                .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()?;
            self.apply_setting(key, value)?;
            self.track_setting(key, old.as_deref(), value)
        })();
        if result.is_err() {
            self.connection.execute_batch("ROLLBACK TO write_setting")?;
        }
        self.connection.execute_batch("RELEASE write_setting")?;
        result
    }

    fn apply_setting(&self, key: &str, value: Option<&str>) -> rusqlite::Result<()> {
        match value {
            Some(value) => self.connection.execute(
                "INSERT OR REPLACE INTO setting (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            ),
            None => self
                .connection
                .execute("DELETE FROM setting WHERE key = ?1", [key]),
        }?;
        Ok(())
    }

    #[allow(dead_code)] // settings UI (Stage 6) uses this to clear a key
    pub fn remove_setting(&self, key: &str) -> rusqlite::Result<()> {
        self.write_setting(key, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Doc {
        id: String,
        name: String,
    }

    impl Document for Doc {
        fn document_id(&self) -> String {
            self.id.clone()
        }
    }

    fn env_of<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
        }
    }

    #[test]
    fn the_cache_dir_follows_each_platforms_convention() {
        let mac = cache_dir_from(&env_of(&[("HOME", "/Users/a")]), true, false);
        assert_eq!(
            mac,
            Some(PathBuf::from("/Users/a/Library/Caches/es.nico.degenview"))
        );
        let windows = cache_dir_from(
            &env_of(&[("LOCALAPPDATA", "C:\\L"), ("APPDATA", "C:\\R")]),
            false,
            true,
        );
        assert_eq!(
            windows,
            Some(
                PathBuf::from("C:\\L")
                    .join("es.nico.degenview")
                    .join("Cache")
            )
        );
        let xdg = cache_dir_from(
            &env_of(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/home/a")]),
            false,
            false,
        );
        assert_eq!(xdg, Some(PathBuf::from("/x/es.nico.degenview")));
        let home = cache_dir_from(&env_of(&[("HOME", "/home/a")]), false, false);
        assert_eq!(
            home,
            Some(PathBuf::from("/home/a/.cache/es.nico.degenview"))
        );
        assert_eq!(cache_dir_from(&env_of(&[]), false, false), None);
    }

    #[test]
    fn the_cache_dir_can_be_overridden_and_an_empty_value_is_ignored() {
        let over = cache_dir_from(
            &env_of(&[(CACHE_DIR_ENV, "/scratch"), ("HOME", "/Users/a")]),
            true,
            false,
        );
        assert_eq!(over, Some(PathBuf::from("/scratch")));
        let empty = cache_dir_from(
            &env_of(&[(CACHE_DIR_ENV, ""), ("HOME", "/h")]),
            false,
            false,
        );
        assert_eq!(empty, Some(PathBuf::from("/h/.cache/es.nico.degenview")));
    }

    #[test]
    fn documents_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        let docs = vec![
            Doc {
                id: "a".into(),
                name: "first".into(),
            },
            Doc {
                id: "b".into(),
                name: "second".into(),
            },
        ];
        store.replace_documents("tab", &docs).expect("write");
        let loaded: Vec<Doc> = store.documents("tab");
        assert_eq!(loaded, docs);
    }

    #[test]
    fn settings_round_trip_and_remove() {
        let store = Store::open_in_memory().expect("open");
        store
            .set_setting("window_frame", &"42,42,800,600")
            .expect("write");
        assert_eq!(
            store.setting::<String>("window_frame").as_deref(),
            Some("42,42,800,600")
        );
        store.remove_setting("window_frame").expect("remove");
        assert!(store.setting::<String>("window_frame").is_none());
    }

    #[test]
    fn strict_setting_distinguishes_missing_and_unreadable() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.setting_strict::<u64>("counter").expect("read"), None);
        store
            .connection
            .execute(
                "INSERT INTO setting (key, value) VALUES ('counter', 'bad')",
                [],
            )
            .expect("seed");
        assert!(store.setting_strict::<u64>("counter").is_err());
    }

    #[test]
    fn undecodable_rows_are_skipped() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO tab (id, position, payload) VALUES ('x', 0, 'not json')",
                [],
            )
            .expect("insert bad row");
        let loaded: Vec<Doc> = store.documents("tab");
        assert!(loaded.is_empty());
    }

    #[test]
    fn replacing_documents_rolls_back_after_an_insert_error() {
        let store = Store::open_in_memory().expect("open");
        let original = Doc {
            id: "old".into(),
            name: "Original".into(),
        };
        store
            .replace_documents("tab", std::slice::from_ref(&original))
            .expect("seed");
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_bad_tab BEFORE INSERT ON tab
             WHEN NEW.id = 'bad' BEGIN SELECT RAISE(ABORT, 'bad row'); END;",
            )
            .expect("trigger");
        let replacement = [
            Doc {
                id: "new".into(),
                name: "New".into(),
            },
            Doc {
                id: "bad".into(),
                name: "Bad".into(),
            },
        ];
        assert!(store.replace_documents("tab", &replacement).is_err());
        assert_eq!(store.documents::<Doc>("tab"), vec![original]);
    }

    #[test]
    fn replacing_documents_preserves_unreadable_rows() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO tab (id, position, payload) VALUES ('old', 0, 'not json')",
                [],
            )
            .expect("seed");
        assert!(store.documents_strict::<Doc>("tab").is_err());
        assert!(store.replace_documents::<Doc>("tab", &[]).is_err());
        let payload: String = store
            .connection
            .query_row("SELECT payload FROM tab WHERE id = 'old'", [], |row| {
                row.get(0)
            })
            .expect("old row remains");
        assert_eq!(payload, "not json");
    }

    #[test]
    fn a_mid_transaction_failure_rolls_everything_back() {
        let store = Store::open_in_memory().expect("open");
        let result = store.in_transaction(|| {
            store.set_setting("half.written", &"value").expect("write");
            Err(rusqlite::Error::QueryReturnedNoRows)
        });
        assert!(result.is_err());
        assert!(
            store.setting::<String>("half.written").is_none(),
            "the write before the failure must be rolled back"
        );
    }
}
