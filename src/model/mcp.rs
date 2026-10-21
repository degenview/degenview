//! MCP server settings, client credentials and the permission model. Pure values: no
//! HTTP, no egui, no Keychain. The server, the store and the Settings page all share these.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const DEFAULT_PORT: u16 = 8765;
/// Ports below this need elevated rights on most systems and are never a sensible choice.
pub const MIN_PORT: u16 = 1024;
pub const MAX_CLIENT_NAME_LEN: usize = 60;
pub const MAX_CLIENTS: usize = 16;

/// One capability a client can be granted. Reading and mutating are separate permissions;
/// there is deliberately no "full access" entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    ChartsRead,
    ChartsCreate,
    ChartsIndicatorsWrite,
    ChartsPineInstancesWrite,
    ChartsCandlesRead,
    DrawingsRead,
    DrawingsWrite,
    DrawingsDelete,
    AlertsRead,
    AlertsCreate,
    AlertsEdit,
    AlertsPause,
    AlertsDelete,
    PineScriptsList,
    PineScriptsRead,
    PineScriptsWrite,
    PineScriptsSave,
    PineDiagnosticsRead,
    PineInputsRead,
    PineStylesRead,
    PineReportsRead,
    PineAlertsRead,
}

impl Permission {
    pub const GROUPS: [&'static str; 4] = ["Charts", "Drawings", "Price Alerts", "Pine Scripts"];

    pub const ALL: [Permission; 22] = [
        Permission::ChartsRead,
        Permission::ChartsCreate,
        Permission::ChartsIndicatorsWrite,
        Permission::ChartsPineInstancesWrite,
        Permission::ChartsCandlesRead,
        Permission::DrawingsRead,
        Permission::DrawingsWrite,
        Permission::DrawingsDelete,
        Permission::AlertsRead,
        Permission::AlertsCreate,
        Permission::AlertsEdit,
        Permission::AlertsPause,
        Permission::AlertsDelete,
        Permission::PineScriptsList,
        Permission::PineScriptsRead,
        Permission::PineScriptsWrite,
        Permission::PineScriptsSave,
        Permission::PineDiagnosticsRead,
        Permission::PineInputsRead,
        Permission::PineStylesRead,
        Permission::PineReportsRead,
        Permission::PineAlertsRead,
    ];

    /// The stable wire/storage identifier, e.g. `charts.read`.
    pub fn id(self) -> &'static str {
        match self {
            Self::ChartsRead => "charts.read",
            Self::ChartsCreate => "charts.create",
            Self::ChartsIndicatorsWrite => "charts.indicators.write",
            Self::ChartsPineInstancesWrite => "charts.pine_instances.write",
            Self::ChartsCandlesRead => "charts.candles.read",
            Self::DrawingsRead => "drawings.read",
            Self::DrawingsWrite => "drawings.write",
            Self::DrawingsDelete => "drawings.delete",
            Self::AlertsRead => "alerts.read",
            Self::AlertsCreate => "alerts.create",
            Self::AlertsEdit => "alerts.edit",
            Self::AlertsPause => "alerts.pause",
            Self::AlertsDelete => "alerts.delete",
            Self::PineScriptsList => "pine.scripts.list",
            Self::PineScriptsRead => "pine.scripts.read",
            Self::PineScriptsWrite => "pine.scripts.write",
            Self::PineScriptsSave => "pine.scripts.save",
            Self::PineDiagnosticsRead => "pine.diagnostics.read",
            Self::PineInputsRead => "pine.inputs.read",
            Self::PineStylesRead => "pine.styles.read",
            Self::PineReportsRead => "pine.reports.read",
            Self::PineAlertsRead => "pine.alerts.read",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|permission| permission.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ChartsRead => "Read workspaces and charts",
            Self::ChartsCreate => "Add charts to a workspace",
            Self::ChartsIndicatorsWrite => "Add and configure indicators",
            Self::ChartsPineInstancesWrite => "Apply and configure scripts on charts",
            Self::ChartsCandlesRead => "Read chart candles",
            Self::DrawingsRead => "Read chart drawings",
            Self::DrawingsWrite => "Draw on charts and edit drawings",
            Self::DrawingsDelete => "Delete chart drawings",
            Self::AlertsRead => "Read price alerts",
            Self::AlertsCreate => "Create price alerts",
            Self::AlertsEdit => "Edit price alerts",
            Self::AlertsPause => "Pause and resume price alerts",
            Self::AlertsDelete => "Delete price alerts",
            Self::PineScriptsList => "List scripts and chart-applied scripts",
            Self::PineScriptsRead => "Read script source",
            Self::PineScriptsWrite => "Edit script drafts",
            Self::PineScriptsSave => "Save scripts over their saved source",
            Self::PineDiagnosticsRead => "Compile scripts and read diagnostics",
            Self::PineInputsRead => "Read script inputs",
            Self::PineStylesRead => "Read script style options",
            Self::PineReportsRead => "Read strategy reports",
            Self::PineAlertsRead => "Read script alerts",
        }
    }

    /// One line on what the permission lets a client do, for the Settings sheet.
    pub fn detail(self) -> &'static str {
        match self {
            Self::ChartsRead => "See workspaces, charts and the indicators on them.",
            Self::ChartsCreate => "Add charts to a workspace.",
            Self::ChartsIndicatorsWrite => {
                "Turn built-in indicators on a chart and set their period."
            }
            Self::ChartsPineInstancesWrite => {
                "Apply a saved script to a chart, change its inputs or visibility, and remove it."
            }
            Self::ChartsCandlesRead => {
                "Read the price candles a chart has loaded: open, high, low, close and volume."
            }
            Self::DrawingsRead => "List the trend lines, rectangles and other drawings on a chart.",
            Self::DrawingsWrite => {
                "Add trend lines and rectangles, and move, restyle or hide the ones that are not locked."
            }
            Self::DrawingsDelete => "Permanently remove drawings that are not locked.",
            Self::AlertsRead => "List price alerts.",
            Self::AlertsCreate => "Create price alerts.",
            Self::AlertsEdit => "Change an alert's price, direction, currency, frequency or note.",
            Self::AlertsPause => "Pause and resume price alerts.",
            Self::AlertsDelete => "Permanently remove price alerts.",
            Self::PineScriptsList => "List scripts and the scripts applied to charts.",
            Self::PineScriptsRead => "Read script source code.",
            Self::PineScriptsWrite => "Change a script's unsaved draft.",
            Self::PineScriptsSave => "Overwrite a script's saved source with its draft.",
            Self::PineDiagnosticsRead => "Compile scripts and read errors and warnings.",
            Self::PineInputsRead => "Read input definitions and applied-script settings.",
            Self::PineStylesRead => "Read plot and style options of applied scripts.",
            Self::PineReportsRead => "Read strategy backtest reports.",
            Self::PineAlertsRead => "List script alert subscriptions.",
        }
    }

    /// The Settings heading it is listed under; `GROUPS` gives the order.
    pub fn group(self) -> &'static str {
        match self {
            Self::ChartsRead
            | Self::ChartsCreate
            | Self::ChartsIndicatorsWrite
            | Self::ChartsPineInstancesWrite
            | Self::ChartsCandlesRead => "Charts",
            Self::DrawingsRead | Self::DrawingsWrite | Self::DrawingsDelete => "Drawings",
            Self::AlertsRead
            | Self::AlertsCreate
            | Self::AlertsEdit
            | Self::AlertsPause
            | Self::AlertsDelete => "Price Alerts",
            Self::PineScriptsList
            | Self::PineScriptsRead
            | Self::PineScriptsWrite
            | Self::PineScriptsSave
            | Self::PineDiagnosticsRead
            | Self::PineInputsRead
            | Self::PineStylesRead
            | Self::PineReportsRead
            | Self::PineAlertsRead => "Pine Scripts",
        }
    }

    /// Whether the permission changes application state.
    pub fn is_mutating(self) -> bool {
        match self {
            Self::ChartsRead
            | Self::ChartsCandlesRead
            | Self::DrawingsRead
            | Self::AlertsRead
            | Self::PineScriptsList
            | Self::PineScriptsRead
            | Self::PineDiagnosticsRead
            | Self::PineInputsRead
            | Self::PineStylesRead
            | Self::PineReportsRead
            | Self::PineAlertsRead => false,
            Self::ChartsCreate
            | Self::ChartsIndicatorsWrite
            | Self::ChartsPineInstancesWrite
            | Self::DrawingsWrite
            | Self::DrawingsDelete
            | Self::AlertsCreate
            | Self::AlertsEdit
            | Self::AlertsPause
            | Self::PineScriptsWrite
            | Self::PineScriptsSave
            | Self::AlertsDelete => true,
        }
    }
}

/// A set of permissions. Stored as identifier strings; an identifier this version does not
/// know (written by a newer build) is dropped on load, which can only ever *remove* access.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<String>", into = "Vec<String>")]
pub struct PermissionSet {
    granted: BTreeSet<Permission>,
}

impl PermissionSet {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn of(permissions: impl IntoIterator<Item = Permission>) -> Self {
        Self {
            granted: permissions.into_iter().collect(),
        }
    }

    pub fn contains(&self, permission: Permission) -> bool {
        self.granted.contains(&permission)
    }

    pub fn set(&mut self, permission: Permission, allowed: bool) {
        if allowed {
            self.granted.insert(permission);
        } else {
            self.granted.remove(&permission);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.granted.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.granted.iter().copied()
    }

    /// What a client may actually do: only what both the global cap and its own grant allow.
    pub fn intersect(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            granted: self.granted.intersection(&other.granted).copied().collect(),
        }
    }
}

impl From<Vec<String>> for PermissionSet {
    fn from(ids: Vec<String>) -> Self {
        Self {
            granted: ids
                .iter()
                .filter_map(|id| Permission::from_id(id))
                .collect(),
        }
    }
}

impl From<PermissionSet> for Vec<String> {
    fn from(set: PermissionSet) -> Self {
        set.granted.iter().map(|p| p.id().to_string()).collect()
    }
}

impl PermissionSet {
    /// Every permission that only reads.
    pub fn read_only() -> Self {
        Self::of(Permission::ALL.into_iter().filter(|p| !p.is_mutating()))
    }

    pub fn everything() -> Self {
        Self::of(Permission::ALL)
    }

    /// How many permissions of `group` this set holds that read, and that change DegenView.
    pub fn group_access(&self, group: &str) -> GroupAccess {
        let mut access = GroupAccess::default();
        for permission in self.iter().filter(|p| p.group() == group) {
            if permission.is_mutating() {
                access.write += 1;
            } else {
                access.read += 1;
            }
        }
        access
    }
}

/// What a set of permissions allows within one group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupAccess {
    pub read: usize,
    pub write: usize,
}

/// What the Settings page asks the MCP host to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpAction {
    SetEnabled(bool),
    SetPort(u16),
    CreateClient(String),
    RevokeClient(String),
    SetClientPermission {
        client_id: String,
        permission: Permission,
        allowed: bool,
    },
    SetGlobalPermission {
        permission: Permission,
        allowed: bool,
    },
    /// Replaces a client's whole grant at once (a preset), in one write.
    SetClientPermissions {
        client_id: String,
        permissions: PermissionSet,
    },
    /// Replaces the global cap at once.
    SetGlobalPermissions(PermissionSet),
    DismissToken,
}

/// Persisted server preferences (`setting` key `mcp.settings`). Disabled on a fresh install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
        }
    }
}

impl McpSettings {
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
}

pub fn is_valid_port(port: u16) -> bool {
    port >= MIN_PORT
}

/// The global cap: no client ever gets more than this. A first enablement grants read only.
pub fn default_global_permissions() -> PermissionSet {
    PermissionSet::of([Permission::ChartsRead])
}

/// A credential issued to one MCP client. Only the SHA-256 verifier of the bearer token is
/// stored — the token itself is shown once at creation and never kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClient {
    pub id: String,
    pub name: String,
    /// Lowercase hex SHA-256 of the bearer token.
    pub verifier: String,
    #[serde(default)]
    pub permissions: PermissionSet,
    pub created_at_ms: i64,
    #[serde(default)]
    pub last_used_ms: Option<i64>,
}

impl McpClient {
    /// The client's access right now: its grants capped by the global permissions.
    pub fn effective(&self, global: &PermissionSet) -> PermissionSet {
        self.permissions.intersect(global)
    }
}

/// Trims a client name and rejects empty or oversized ones.
pub fn clean_client_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_CLIENT_NAME_LEN {
        return None;
    }
    if name.chars().any(char::is_control) {
        return None;
    }
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_ids_round_trip() {
        for permission in Permission::ALL {
            assert_eq!(Permission::from_id(permission.id()), Some(permission));
        }
        assert_eq!(Permission::from_id("trading.full_access"), None);
        assert!(
            Permission::ALL
                .iter()
                .all(|p| Permission::GROUPS.contains(&p.group()))
        );
    }

    #[test]
    fn effective_permissions_are_the_intersection() {
        let global = PermissionSet::of([Permission::ChartsRead]);
        let client = McpClient {
            id: "1".into(),
            name: "x".into(),
            verifier: String::new(),
            permissions: PermissionSet::of([Permission::ChartsRead, Permission::ChartsCreate]),
            created_at_ms: 0,
            last_used_ms: None,
        };
        let effective = client.effective(&global);
        assert!(effective.contains(Permission::ChartsRead));
        assert!(!effective.contains(Permission::ChartsCreate));
        assert!(client.effective(&PermissionSet::none()).is_empty());
    }

    #[test]
    fn unknown_permission_ids_are_dropped_not_granted() {
        let set: PermissionSet =
            serde_json::from_str(r#"["charts.read","pine.scripts.delete"]"#).unwrap();
        assert_eq!(set, PermissionSet::of([Permission::ChartsRead]));
        assert_eq!(serde_json::to_string(&set).unwrap(), r#"["charts.read"]"#);
    }

    #[test]
    fn fresh_settings_are_disabled_on_the_default_port() {
        let settings = McpSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.endpoint(), "http://127.0.0.1:8765/mcp");
        let partial: McpSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(partial, settings);
    }

    #[test]
    fn presets_and_group_summaries_count_reads_and_writes() {
        assert!(PermissionSet::read_only().iter().all(|p| !p.is_mutating()));
        assert!(!PermissionSet::read_only().contains(Permission::AlertsDelete));
        assert!(PermissionSet::read_only().contains(Permission::PineReportsRead));
        assert_eq!(
            PermissionSet::everything().iter().count(),
            Permission::ALL.len()
        );

        let set = PermissionSet::of([
            Permission::ChartsRead,
            Permission::ChartsCreate,
            Permission::AlertsRead,
        ]);
        assert_eq!(
            set.group_access("Charts"),
            GroupAccess { read: 1, write: 1 }
        );
        assert_eq!(
            set.group_access("Price Alerts"),
            GroupAccess { read: 1, write: 0 }
        );
        assert_eq!(set.group_access("Pine Scripts"), GroupAccess::default());
        // Every permission has a description and a group the page lists.
        for permission in Permission::ALL {
            assert!(!permission.detail().is_empty());
            assert!(Permission::GROUPS.contains(&permission.group()));
        }
    }

    #[test]
    fn default_global_cap_has_no_mutating_permission() {
        assert!(
            default_global_permissions()
                .iter()
                .all(|p| !p.is_mutating())
        );
    }

    #[test]
    fn client_names_are_trimmed_and_bounded() {
        assert_eq!(clean_client_name("  Claude  ").as_deref(), Some("Claude"));
        assert_eq!(clean_client_name("   "), None);
        assert_eq!(clean_client_name("a\nb"), None);
        assert_eq!(
            clean_client_name(&"x".repeat(MAX_CLIENT_NAME_LEN + 1)),
            None
        );
    }
}
