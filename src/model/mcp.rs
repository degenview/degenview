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
    ChartsViewWrite,
    ChartsDelete,
    ChartsReplayWrite,
    ChartsSnapshotRead,
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
    MarketsRead,
    WatchlistsRead,
    WatchlistsWrite,
    WatchlistsDelete,
    PaperRead,
    PaperOrdersPlace,
    PaperOrdersCancel,
    PaperPositionsClose,
    PortfolioRead,
}

impl Permission {
    pub const GROUPS: [&'static str; 8] = [
        "Charts",
        "Drawings",
        "Price Alerts",
        "Pine Scripts",
        "Markets",
        "Watchlists",
        "Paper Trading",
        "Portfolio",
    ];

    pub const ALL: [Permission; 35] = [
        Permission::ChartsRead,
        Permission::ChartsCreate,
        Permission::ChartsIndicatorsWrite,
        Permission::ChartsPineInstancesWrite,
        Permission::ChartsCandlesRead,
        Permission::ChartsViewWrite,
        Permission::ChartsDelete,
        Permission::ChartsReplayWrite,
        Permission::ChartsSnapshotRead,
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
        Permission::MarketsRead,
        Permission::WatchlistsRead,
        Permission::WatchlistsWrite,
        Permission::WatchlistsDelete,
        Permission::PaperRead,
        Permission::PaperOrdersPlace,
        Permission::PaperOrdersCancel,
        Permission::PaperPositionsClose,
        Permission::PortfolioRead,
    ];

    /// The stable wire/storage identifier, e.g. `charts.read`.
    pub fn id(self) -> &'static str {
        match self {
            Self::ChartsRead => "charts.read",
            Self::ChartsCreate => "charts.create",
            Self::ChartsIndicatorsWrite => "charts.indicators.write",
            Self::ChartsPineInstancesWrite => "charts.pine_instances.write",
            Self::ChartsCandlesRead => "charts.candles.read",
            Self::ChartsViewWrite => "charts.view.write",
            Self::ChartsDelete => "charts.delete",
            Self::ChartsReplayWrite => "charts.replay.write",
            Self::ChartsSnapshotRead => "charts.snapshot.read",
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
            Self::MarketsRead => "markets.read",
            Self::WatchlistsRead => "watchlists.read",
            Self::WatchlistsWrite => "watchlists.write",
            Self::WatchlistsDelete => "watchlists.delete",
            Self::PaperRead => "paper.read",
            Self::PaperOrdersPlace => "paper.orders.place",
            Self::PaperOrdersCancel => "paper.orders.cancel",
            Self::PaperPositionsClose => "paper.positions.close",
            Self::PortfolioRead => "portfolio.read",
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
            Self::ChartsViewWrite => "Change timeframe and chart type",
            Self::ChartsDelete => "Remove charts",
            Self::ChartsReplayWrite => "Control chart replay",
            Self::ChartsSnapshotRead => "Take chart pictures",
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
            Self::MarketsRead => "Search markets and read quotes",
            Self::WatchlistsRead => "Read watchlists",
            Self::WatchlistsWrite => "Create and edit watchlists",
            Self::WatchlistsDelete => "Delete watchlists",
            Self::PaperRead => "Read paper trading",
            Self::PaperOrdersPlace => "Place and change paper orders",
            Self::PaperOrdersCancel => "Cancel paper orders",
            Self::PaperPositionsClose => "Close paper positions",
            Self::PortfolioRead => "Read portfolios",
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
            Self::ChartsViewWrite => {
                "Switch a workspace to another timeframe and a chart to another chart type."
            }
            Self::ChartsDelete => {
                "Remove a chart from a workspace, together with the drawings on it."
            }
            Self::ChartsSnapshotRead => {
                "Take a picture (PNG) of a chart as it is drawn: candles, indicators, scripts and drawings, and orders and positions only if Settings includes them in snapshots. Nothing is saved or shared."
            }
            Self::ChartsReplayWrite => {
                "Start, step, seek and stop a workspace's replay. While it runs every chart in the workspace shows the past, and the candle tools read it."
            }
            Self::DrawingsRead => "List the trend lines, rectangles and other drawings on a chart.",
            Self::DrawingsWrite => {
                "Draw trend lines, rectangles, rays, arrows, levels, Fibonacci retracements and text, and move, restyle or hide the ones that are not locked."
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
            Self::MarketsRead => {
                "Search the data providers for symbols, and read the last price, 24-hour change and volume of a market."
            }
            Self::WatchlistsRead => "List watchlists and the markets in them.",
            Self::WatchlistsWrite => "Create watchlists, rename them, and add or remove markets.",
            Self::WatchlistsDelete => {
                "Permanently remove watchlists (Favorites cannot be removed)."
            }
            Self::PaperRead => {
                "Read paper-trading accounts, positions, orders, fills and closed trades."
            }
            Self::PaperOrdersPlace => {
                "Place simulated orders (market, limit, stop, stop-limit, with take-profit and stop-loss) and change a working order's price or size. Simulated money only; nothing reaches a broker."
            }
            Self::PaperOrdersCancel => "Cancel working paper orders.",
            Self::PaperPositionsClose => {
                "Close paper positions at the market, one or all of an account's."
            }
            Self::PortfolioRead => {
                "Read portfolios, what they hold and what it is worth, and their transactions. Nothing can be changed."
            }
        }
    }

    /// The Settings heading it is listed under; `GROUPS` gives the order.
    pub fn group(self) -> &'static str {
        match self {
            Self::ChartsRead
            | Self::ChartsCreate
            | Self::ChartsIndicatorsWrite
            | Self::ChartsPineInstancesWrite
            | Self::ChartsCandlesRead
            | Self::ChartsViewWrite
            | Self::ChartsDelete
            | Self::ChartsReplayWrite
            | Self::ChartsSnapshotRead => "Charts",
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
            Self::MarketsRead => "Markets",
            Self::WatchlistsRead | Self::WatchlistsWrite | Self::WatchlistsDelete => "Watchlists",
            Self::PaperRead
            | Self::PaperOrdersPlace
            | Self::PaperOrdersCancel
            | Self::PaperPositionsClose => "Paper Trading",
            Self::PortfolioRead => "Portfolio",
        }
    }

    /// Whether the permission changes application state.
    pub fn is_mutating(self) -> bool {
        match self {
            Self::ChartsRead
            | Self::ChartsCandlesRead
            | Self::ChartsSnapshotRead
            | Self::DrawingsRead
            | Self::AlertsRead
            | Self::PineScriptsList
            | Self::PineScriptsRead
            | Self::PineDiagnosticsRead
            | Self::PineInputsRead
            | Self::PineStylesRead
            | Self::PineReportsRead
            | Self::PineAlertsRead
            | Self::MarketsRead
            | Self::PaperRead
            | Self::PortfolioRead
            | Self::WatchlistsRead => false,
            Self::ChartsCreate
            | Self::ChartsIndicatorsWrite
            | Self::ChartsPineInstancesWrite
            | Self::ChartsViewWrite
            | Self::ChartsDelete
            | Self::ChartsReplayWrite
            | Self::PaperOrdersPlace
            | Self::PaperOrdersCancel
            | Self::PaperPositionsClose
            | Self::DrawingsWrite
            | Self::DrawingsDelete
            | Self::AlertsCreate
            | Self::AlertsEdit
            | Self::AlertsPause
            | Self::PineScriptsWrite
            | Self::PineScriptsSave
            | Self::WatchlistsWrite
            | Self::WatchlistsDelete
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
    /// Empties the activity log (the log keeps one row saying that it was cleared).
    ClearActivity,
    /// Whether high-impact calls wait for the user's approval in the app.
    SetRequireConfirmation(bool),
}

/// Persisted server preferences (`setting` key `mcp.settings`). Disabled on a fresh install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    /// Deletes, saved scripts and paper orders wait for the user's approval in the app. On
    /// unless the user turns it off, including for settings saved before this existed.
    pub require_confirmation: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            require_confirmation: true,
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

/// Rows the activity log keeps; older ones are pruned as new ones arrive.
pub const AUDIT_LOG_LIMIT: usize = 1000;
/// Rows the Settings page shows.
pub const AUDIT_VISIBLE: usize = 100;

/// What an activity-log row stands for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAuditKind {
    /// One tool call.
    #[default]
    Call,
    /// A client was refused for going over its rate limit; `count` calls in one minute.
    RateLimited,
    /// The user cleared the log.
    Cleared,
    /// The server could not keep up and `count` rows were not recorded.
    Gap,
    /// The user answered (or let run out) a question about a call the agent made.
    Confirmation,
}

/// How a tool call ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAuditOutcome {
    #[default]
    Succeeded,
    /// It ran, or tried to, and ended in an error; `code` says which.
    Failed,
    /// Refused before it did anything: permission, an unknown tool, a rate limit.
    Denied,
    /// The client hung up or cancelled before an answer was sent.
    Cancelled,
    /// Waiting for the user's approval: the call was answered `CONFIRMATION_REQUIRED` and
    /// nothing ran.
    Held,
    /// A `Confirmation` row: the user approved it.
    Approved,
    /// A `Confirmation` row: the user declined it.
    Declined,
    /// A `Confirmation` row: nobody answered in time.
    Expired,
}

/// One thing a call named: `kind` is the argument (`chart_id`), `id` its value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpAuditTarget {
    pub kind: String,
    pub id: String,
}

/// One row of the activity log: who asked DegenView to do what, and how it ended.
///
/// Deliberately small. It holds ids and codes, never an argument value, a message, a token or a
/// script: the log is for answering "what did this agent do", not for replaying it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpAuditEntry {
    /// Unique per row; also the correlation id.
    pub id: u64,
    pub at_ms: i64,
    pub kind: McpAuditKind,
    pub client_id: String,
    /// The client's name when the call was made (it can be renamed or revoked later).
    pub client_name: String,
    /// The tool's name, as called.
    pub tool: String,
    pub outcome: McpAuditOutcome,
    /// The error or refusal code (`NOT_FOUND`, `PERMISSION_DENIED`...); empty on success.
    pub code: String,
    /// From the call arriving to its answer, queue wait included.
    pub duration_ms: u64,
    /// The client's own JSON-RPC id for the call, to match it with the client's trace.
    pub request_id: String,
    /// The ids the call named (a few, from a fixed list of argument names).
    pub targets: Vec<McpAuditTarget>,
    /// How many calls a `RateLimited` row stands for, or rows a `Gap` lost; 1 otherwise.
    pub count: u32,
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
