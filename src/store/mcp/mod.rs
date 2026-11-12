//! Persistence for the MCP server: preferences and the global permission cap are `setting`
//! values, client credentials (name, permissions, token verifier) an ordered document table.
//!
//! Every read is strict. A value this version cannot decode is an error, never an empty
//! default: the caller (`mcp_host`) then refuses to start the server instead of running with
//! credentials or permissions it silently lost.

pub mod audit;

use crate::model::mcp::{McpClient, McpSettings, PermissionSet, default_global_permissions};
use crate::store::{Document, Store};

pub const SETTINGS_KEY: &str = "mcp.settings";
pub const GLOBAL_PERMISSIONS_KEY: &str = "mcp.permissions";
/// The keyed seal over the three values above and the client rows (see `mcp::seal`).
pub const SEAL_KEY: &str = "mcp.seal";

impl Document for McpClient {
    fn document_id(&self) -> String {
        self.id.clone()
    }
}

impl Store {
    pub fn mcp_settings(&self) -> rusqlite::Result<McpSettings> {
        Ok(self.setting_strict(SETTINGS_KEY)?.unwrap_or_default())
    }

    pub fn set_mcp_settings(&self, settings: &McpSettings) -> rusqlite::Result<()> {
        self.set_setting(SETTINGS_KEY, settings)
    }

    /// The global cap. A missing key is a first run: read-only.
    pub fn mcp_global_permissions(&self) -> rusqlite::Result<PermissionSet> {
        Ok(self
            .setting_strict(GLOBAL_PERMISSIONS_KEY)?
            .unwrap_or_else(default_global_permissions))
    }

    pub fn set_mcp_global_permissions(&self, permissions: &PermissionSet) -> rusqlite::Result<()> {
        self.set_setting(GLOBAL_PERMISSIONS_KEY, permissions)
    }

    pub fn mcp_clients(&self) -> rusqlite::Result<Vec<McpClient>> {
        self.documents_strict("mcp_client")
    }

    pub fn replace_mcp_clients(&self, clients: &[McpClient]) -> rusqlite::Result<()> {
        self.replace_documents("mcp_client", clients)
    }

    /// The stored seal, if any.
    pub fn mcp_seal(&self) -> rusqlite::Result<Option<String>> {
        self.setting_strict(SEAL_KEY)
    }

    /// Writes settings, cap, clients and (when sealing) the seal together or not at all, so the
    /// seal on disk always matches the values beside it. All three values are rewritten from the
    /// caller's state, never patched, so a row someone else changed on disk is not sealed in.
    pub fn save_mcp_state(
        &self,
        settings: &McpSettings,
        global: &PermissionSet,
        clients: &[McpClient],
        seal: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.connection.execute_batch("SAVEPOINT save_mcp_state")?;
        let result = (|| {
            self.set_mcp_settings(settings)?;
            self.set_mcp_global_permissions(global)?;
            self.replace_mcp_clients(clients)?;
            match seal {
                Some(seal) => self.set_setting(SEAL_KEY, &seal),
                None => self.remove_setting(SEAL_KEY),
            }
        })();
        if result.is_err() {
            self.connection
                .execute_batch("ROLLBACK TO save_mcp_state")?;
        }
        self.connection.execute_batch("RELEASE save_mcp_state")?;
        result
    }

    /// Forgets every MCP setting, credential and the seal, undecodable rows included. For the
    /// user's explicit "reset MCP access" when the saved state cannot be trusted. These tables
    /// are device-local (`sync::policy`), so nothing needs queuing for upload.
    pub fn clear_mcp_state(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch("SAVEPOINT clear_mcp_state")?;
        let result = (|| {
            self.connection.execute("DELETE FROM mcp_client", [])?;
            for key in [SETTINGS_KEY, GLOBAL_PERMISSIONS_KEY, SEAL_KEY] {
                self.remove_setting(key)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.connection
                .execute_batch("ROLLBACK TO clear_mcp_state")?;
        }
        self.connection.execute_batch("RELEASE clear_mcp_state")?;
        result
    }

    /// Writes a row bypassing decoding, to simulate data this version cannot read.
    #[cfg(test)]
    pub(crate) fn insert_raw_mcp_client(&self, id: &str, payload: &str) {
        self.connection
            .execute(
                "INSERT INTO mcp_client (id, position, payload) VALUES (?1, 99, ?2)",
                [id, payload],
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mcp::Permission;

    fn client(id: &str) -> McpClient {
        McpClient {
            id: id.into(),
            name: format!("client {id}"),
            verifier: "ab".repeat(32),
            permissions: PermissionSet::of([Permission::ChartsRead]),
            created_at_ms: 10,
            last_used_ms: None,
            remote_allowed: false,
            allowed_cidrs: Vec::new(),
        }
    }

    #[test]
    fn fresh_store_is_disabled_and_read_only() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.mcp_settings().unwrap(), McpSettings::default());
        assert!(!store.mcp_settings().unwrap().enabled);
        assert_eq!(
            store.mcp_global_permissions().unwrap(),
            default_global_permissions()
        );
        assert!(store.mcp_clients().unwrap().is_empty());
    }

    #[test]
    fn clients_and_settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        store
            .replace_mcp_clients(&[client("a"), client("b")])
            .unwrap();
        let ids: Vec<_> = store
            .mcp_clients()
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);

        let settings = McpSettings {
            enabled: true,
            port: 9001,
            ..McpSettings::default()
        };
        store.set_mcp_settings(&settings).unwrap();
        assert_eq!(store.mcp_settings().unwrap(), settings);
        store
            .set_mcp_global_permissions(&PermissionSet::of(Permission::ALL))
            .unwrap();
        assert!(
            store
                .mcp_global_permissions()
                .unwrap()
                .contains(Permission::ChartsCreate)
        );
    }

    #[test]
    fn state_and_seal_are_written_together_and_cleared_together() {
        let store = Store::open_in_memory().unwrap();
        let settings = McpSettings {
            enabled: true,
            port: 9001,
            ..McpSettings::default()
        };
        let global = PermissionSet::of([Permission::ChartsRead]);
        store
            .save_mcp_state(&settings, &global, &[client("a")], Some("abc"))
            .unwrap();
        assert_eq!(store.mcp_seal().unwrap().as_deref(), Some("abc"));
        assert_eq!(store.mcp_settings().unwrap(), settings);
        assert_eq!(store.mcp_clients().unwrap().len(), 1);

        store
            .save_mcp_state(&settings, &global, &[client("a")], None)
            .unwrap();
        assert_eq!(store.mcp_seal().unwrap(), None);

        store.clear_mcp_state().unwrap();
        assert_eq!(store.mcp_settings().unwrap(), McpSettings::default());
        assert!(store.mcp_clients().unwrap().is_empty());
        assert_eq!(store.mcp_seal().unwrap(), None);
    }

    #[test]
    fn a_failed_state_save_changes_nothing() {
        let store = Store::open_in_memory().unwrap();
        let global = PermissionSet::of([Permission::ChartsRead]);
        store
            .save_mcp_state(
                &McpSettings::default(),
                &global,
                &[client("a")],
                Some("old"),
            )
            .unwrap();
        store.insert_raw_mcp_client("bad", "{nope");
        let enabled = McpSettings {
            enabled: true,
            port: 9001,
            ..McpSettings::default()
        };
        assert!(
            store
                .save_mcp_state(&enabled, &global, &[client("b")], Some("new"))
                .is_err()
        );
        assert_eq!(store.mcp_settings().unwrap(), McpSettings::default());
        assert_eq!(store.mcp_seal().unwrap().as_deref(), Some("old"));

        // A reset still clears a table holding a row nothing can decode.
        store.clear_mcp_state().unwrap();
        assert!(store.mcp_clients().unwrap().is_empty());
    }

    #[test]
    fn undecodable_rows_fail_closed_and_are_not_overwritten() {
        let store = Store::open_in_memory().unwrap();
        store.replace_mcp_clients(&[client("a")]).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO mcp_client (id, position, payload) VALUES ('bad', 1, '{nope')",
                [],
            )
            .unwrap();
        assert!(store.mcp_clients().is_err());
        assert!(store.replace_mcp_clients(&[]).is_err());

        store.set_setting(SETTINGS_KEY, &"not settings").unwrap();
        assert!(store.mcp_settings().is_err());
    }
}
