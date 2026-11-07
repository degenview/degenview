//! Persistence for the MCP server: preferences and the global permission cap are `setting`
//! values, client credentials (name, permissions, token verifier) an ordered document table.
//!
//! Every read is strict. A value this version cannot decode is an error, never an empty
//! default: the caller (`mcp_host`) then refuses to start the server instead of running with
//! credentials or permissions it silently lost.

use crate::model::mcp::{McpClient, McpSettings, PermissionSet, default_global_permissions};
use crate::store::{Document, Store};

pub const SETTINGS_KEY: &str = "mcp.settings";
pub const GLOBAL_PERMISSIONS_KEY: &str = "mcp.permissions";

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
