//! MCP server settings, client credentials and the permission model. Pure values: no
//! HTTP, no egui, no Keychain. The server, the store and the Settings page all share these.

pub mod activity;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};

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
    /// Listens on this machine only, or on one named interface (which makes a certificate).
    SetBind(BindMode),
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
    /// Lets a client be used from other machines, or takes that away.
    SetClientRemote {
        client_id: String,
        allowed: bool,
    },
    /// Replaces the ranges a remote client may call from (empty: any).
    SetClientCidrs {
        client_id: String,
        cidrs: Vec<Cidr>,
    },
    /// Replaces the most a remote-capable client may do.
    SetRemoteCeiling(PermissionSet),
    DismissToken,
    /// Forgets every credential and setting when the saved state failed its seal check. Only
    /// the Settings page sends it, and only while that error is showing.
    ResetAccess,
    /// Empties the activity log (the log keeps one row saying that it was cleared).
    ClearActivity,
    /// Whether high-impact calls wait for the user's approval in the app.
    SetRequireConfirmation(bool),
}

/// Which network interface the server listens on.
///
/// Loopback is the default and the only mode that needs no certificate. A named interface is
/// reachable from other machines, so it is served over TLS only: see `mcp::tls`. There is no
/// "all interfaces" mode: the user names the one address to listen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindMode {
    #[default]
    Loopback,
    /// A specific non-loopback address of this machine. Built with [`BindMode::interface`].
    Interface(IpAddr),
}

/// Why an address cannot be offered as an interface to listen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindError {
    /// `0.0.0.0` / `::`: every interface at once, which this server never does.
    Unspecified,
    Multicast,
    /// `255.255.255.255`.
    Broadcast,
    /// An IPv6 link-local address needs a scope id, which a bare address cannot carry.
    LinkLocalV6,
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unspecified => "Pick one interface address, not 0.0.0.0 or ::",
            Self::Multicast => "A multicast address cannot be listened on",
            Self::Broadcast => "A broadcast address cannot be listened on",
            Self::LinkLocalV6 => "IPv6 link-local addresses are not supported",
        })
    }
}

impl BindMode {
    /// The mode for listening on `ip`. A loopback address is [`BindMode::Loopback`], so there is
    /// one way to say it.
    pub fn interface(ip: IpAddr) -> Result<Self, BindError> {
        if ip.is_loopback() {
            return Ok(Self::Loopback);
        }
        if ip.is_unspecified() {
            return Err(BindError::Unspecified);
        }
        if ip.is_multicast() {
            return Err(BindError::Multicast);
        }
        match ip {
            IpAddr::V4(v4) if v4 == Ipv4Addr::BROADCAST => Err(BindError::Broadcast),
            IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80 => Err(BindError::LinkLocalV6),
            _ => Ok(Self::Interface(ip)),
        }
    }

    pub fn is_loopback(&self) -> bool {
        matches!(self, Self::Loopback)
    }

    /// Whether the server may only be reached over TLS in this mode.
    pub fn requires_tls(&self) -> bool {
        !self.is_loopback()
    }

    /// The address the listener binds.
    pub fn address(&self) -> IpAddr {
        match self {
            Self::Loopback => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Self::Interface(ip) => *ip,
        }
    }

    pub fn scheme(&self) -> &'static str {
        if self.requires_tls() { "https" } else { "http" }
    }

    /// `host:port` as a client writes it, with an IPv6 address bracketed.
    pub fn authority(&self, port: u16) -> String {
        match self.address() {
            IpAddr::V4(ip) => format!("{ip}:{port}"),
            IpAddr::V6(ip) => format!("[{ip}]:{port}"),
        }
    }

    /// Every `Host` header value a request may carry. A server on a named interface answers to
    /// that address only: loopback is not listening, and no name is trusted without being asked.
    pub fn allowed_hosts(&self, port: u16) -> Vec<String> {
        match self {
            Self::Loopback => vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            Self::Interface(_) => vec![self.authority(port)],
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        if text == "loopback" {
            return Ok(Self::Loopback);
        }
        let ip: IpAddr = text
            .parse()
            .map_err(|_| format!("`{text}` is not an IP address"))?;
        Self::interface(ip).map_err(|error| error.to_string())
    }
}

impl Serialize for BindMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Loopback => serializer.serialize_str("loopback"),
            Self::Interface(ip) => serializer.collect_str(ip),
        }
    }
}

impl<'de> Deserialize<'de> for BindMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// An address range, `10.0.0.0/8` or `fd00::/8`. The host bits of the stored address are cleared,
/// so `10.1.2.3/8` and `10.0.0.0/8` are the same range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn new(address: IpAddr, prefix: u8) -> Result<Self, String> {
        let bits = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > bits {
            return Err(format!("/{prefix} is too long for {address}"));
        }
        Ok(Self {
            network: mask(address, prefix),
            prefix,
        })
    }

    /// `10.0.0.0/8`; a bare address is that one host.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address
            .parse()
            .map_err(|_| format!("`{text}` is not an address range"))?;
        let prefix = match prefix {
            Some(prefix) => prefix
                .parse::<u8>()
                .map_err(|_| format!("`{text}` is not an address range"))?,
            None => match address {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            },
        };
        Self::new(address, prefix)
    }

    /// Whether `address` is in the range. An IPv4 address mapped into IPv6 counts as IPv4; a
    /// range of one family never contains an address of the other.
    pub fn contains(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        match (self.network, address) {
            (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)) => {
                mask(address, self.prefix) == self.network
            }
            _ => false,
        }
    }
}

fn mask(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(ip) => {
            let bits = u32::from(ip);
            let keep = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(prefix))
            };
            IpAddr::V4(Ipv4Addr::from(bits & keep))
        }
        IpAddr::V6(ip) => {
            let bits = u128::from(ip);
            let keep = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - u32::from(prefix))
            };
            IpAddr::V6(bits_to_v6(bits & keep))
        }
    }
}

fn bits_to_v6(bits: u128) -> std::net::Ipv6Addr {
    std::net::Ipv6Addr::from(bits)
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

impl Serialize for Cidr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Cidr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// Ranges a client may be given at most.
pub const MAX_CIDRS: usize = 16;

/// Where a connection came from, and whether that is this machine.
///
/// "This machine" is not only loopback. A client on this Mac that uses the address the server
/// listens on (`https://192.168.1.20:8765`, once the server is bound to that interface and not to
/// loopback) arrives with that same address as its peer. A host elsewhere cannot do that: a
/// connection from the server's own address would have its replies delivered to the server itself,
/// so the handshake could never complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// The peer's address, with an IPv4 address mapped into IPv6 shown as IPv4.
    pub ip: IpAddr,
    /// Loopback, or the same address as the one the connection arrived on.
    pub local: bool,
}

impl Peer {
    /// The peer of a connection that arrived on `own`.
    pub fn of_connection(peer: IpAddr, own: IpAddr) -> Self {
        let ip = peer.to_canonical();
        Self {
            ip,
            local: ip.is_loopback() || ip == own.to_canonical(),
        }
    }

    /// A peer known only by its address, with nothing to compare it to: local only if loopback.
    /// For tests, and for a connection whose own address could not be read.
    pub fn from_ip(ip: IpAddr) -> Self {
        let ip = ip.to_canonical();
        Self {
            ip,
            local: ip.is_loopback(),
        }
    }
}

impl std::str::FromStr for Peer {
    type Err = std::net::AddrParseError;

    /// A peer known only by address ([`Peer::from_ip`]); handy where there is no connection.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self::from_ip)
    }
}

/// Whether a request from `peer` may use a credential with this remote scope.
///
/// A peer on this machine always may. Any other needs `remote_allowed`, and then to be inside one
/// of `cidrs` when there are any. An unknown peer (`None`) is treated as remote: if the server
/// cannot say where a request came from, it is not let in on the strength of being local.
pub fn peer_may_use(remote_allowed: bool, cidrs: &[Cidr], peer: Option<Peer>) -> bool {
    match peer {
        Some(peer) if peer.local => true,
        Some(peer) => {
            remote_allowed && (cidrs.is_empty() || cidrs.iter().any(|c| c.contains(peer.ip)))
        }
        None => false,
    }
}

/// What a client that may be used remotely can do at most, whichever address it calls from.
/// Reads only until the user says more.
pub fn default_remote_ceiling() -> PermissionSet {
    PermissionSet::read_only()
}

/// Persisted server preferences (`setting` key `mcp.settings`). Disabled on a fresh install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    /// Left out of the stored form while it is loopback, so settings saved before this existed
    /// read back unchanged. An unreadable value fails the whole load rather than becoming
    /// loopback: the server stays off and nothing is overwritten.
    #[serde(skip_serializing_if = "BindMode::is_loopback")]
    pub bind: BindMode,
    /// The most any client with remote access may do, even when it calls from this machine. A
    /// client's own grants are capped by this as well as by the global cap. Left out of the stored
    /// form while it is the default, so settings saved before this existed read back unchanged.
    #[serde(skip_serializing_if = "is_default_remote_ceiling")]
    pub remote_ceiling: PermissionSet,
    /// Deletes, saved scripts and paper orders wait for the user's approval in the app. On
    /// unless the user turns it off, including for settings saved before this existed.
    pub require_confirmation: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            bind: BindMode::Loopback,
            remote_ceiling: default_remote_ceiling(),
            require_confirmation: true,
        }
    }
}

fn is_default_remote_ceiling(ceiling: &PermissionSet) -> bool {
    *ceiling == default_remote_ceiling()
}

impl McpSettings {
    pub fn endpoint(&self) -> String {
        format!(
            "{}://{}/mcp",
            self.bind.scheme(),
            self.bind.authority(self.port)
        )
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
    /// May be used from another machine. Off for every client that existed before this did, so
    /// each stays loopback-only until the user says otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote_allowed: bool,
    /// With `remote_allowed`, the only ranges a remote request may come from. Empty means any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_cidrs: Vec<Cidr>,
}

impl McpClient {
    /// The client's access right now: its grants capped by the global permissions, and for a
    /// client with remote access by the remote ceiling too. The ceiling follows the credential,
    /// not the request: a token that may leave the machine is no stronger when used on it.
    pub fn effective(
        &self,
        global: &PermissionSet,
        remote_ceiling: &PermissionSet,
    ) -> PermissionSet {
        let capped = self.permissions.intersect(global);
        if self.remote_allowed {
            capped.intersect(remote_ceiling)
        } else {
            capped
        }
    }

    /// Whether a request from `peer` may use this credential at all.
    pub fn accepts_peer(&self, peer: Option<Peer>) -> bool {
        peer_may_use(self.remote_allowed, &self.allowed_cidrs, peer)
    }
}

/// Rows the activity log keeps; older ones are pruned as new ones arrive.
pub const AUDIT_LOG_LIMIT: usize = 1000;

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
    /// `count` requests from another machine were refused for a bad or unusable token within a
    /// minute. Written for remote addresses only; `peer` says which.
    AuthRefused,
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
    /// How many calls a `RateLimited` row stands for, requests an `AuthRefused` row stands for, or
    /// rows a `Gap` lost; 1 otherwise.
    pub count: u32,
    /// The address the request came from, when it was not this machine; empty otherwise (and on
    /// rows that are not about one request, such as an answer to a question). Rows from before
    /// remote access read back empty.
    pub peer: String,
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
            remote_allowed: false,
            allowed_cidrs: Vec::new(),
        };
        let ceiling = default_remote_ceiling();
        let effective = client.effective(&global, &ceiling);
        assert!(effective.contains(Permission::ChartsRead));
        assert!(!effective.contains(Permission::ChartsCreate));
        assert!(
            client
                .effective(&PermissionSet::none(), &ceiling)
                .is_empty()
        );
    }

    #[test]
    fn unknown_permission_ids_are_dropped_not_granted() {
        let set: PermissionSet =
            serde_json::from_str(r#"["charts.read","pine.scripts.delete"]"#).unwrap();
        assert_eq!(set, PermissionSet::of([Permission::ChartsRead]));
        assert_eq!(serde_json::to_string(&set).unwrap(), r#"["charts.read"]"#);
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn cidr(text: &str) -> Cidr {
        Cidr::parse(text).unwrap()
    }

    #[test]
    fn a_range_is_normalised_and_contains_its_own_addresses_only() {
        assert_eq!(cidr("10.1.2.3/8").to_string(), "10.0.0.0/8");
        assert_eq!(cidr("10.9.9.9/8"), cidr("10.0.0.0/8"));
        assert!(cidr("10.0.0.0/8").contains(ip("10.200.1.1")));
        assert!(!cidr("10.0.0.0/8").contains(ip("11.0.0.1")));
        assert!(cidr("192.168.1.20").contains(ip("192.168.1.20")));
        assert!(!cidr("192.168.1.20").contains(ip("192.168.1.21")));
        assert!(cidr("fd00::/8").contains(ip("fd12::1")));
        assert!(!cidr("fd00::/8").contains(ip("fe80::1")));
        assert!(cidr("0.0.0.0/0").contains(ip("203.0.113.9")));
        // A range of one family never contains the other, but a mapped IPv4 address is IPv4.
        assert!(!cidr("10.0.0.0/8").contains(ip("fd00::1")));
        assert!(!cidr("::/0").contains(ip("10.0.0.1")));
        assert!(cidr("10.0.0.0/8").contains(ip("::ffff:10.1.1.1")));
    }

    #[test]
    fn unreadable_ranges_are_refused() {
        for bad in [
            "",
            "10.0.0.0/33",
            "fd00::/129",
            "10.0.0.0/-1",
            "10.0.0.0/x",
            "lan",
            "10.0.0/8",
        ] {
            assert!(Cidr::parse(bad).is_err(), "{bad:?} must not parse");
        }
        let stored = serde_json::to_string(&cidr("10.0.0.5/24")).unwrap();
        assert_eq!(stored, r#""10.0.0.0/24""#);
        assert_eq!(
            serde_json::from_str::<Cidr>(&stored).unwrap(),
            cidr("10.0.0.0/24")
        );
        assert!(serde_json::from_str::<Cidr>(r#""nope""#).is_err());
    }

    #[test]
    fn who_may_use_a_credential_from_where() {
        let lan = [cidr("192.168.1.0/24")];
        let here = Some(Peer::from_ip(ip("127.0.0.1")));
        let also_here = Some(Peer::from_ip(ip("::1")));
        let mapped_here = Some(Peer::from_ip(ip("::ffff:127.0.0.1")));
        let near = Some(Peer::from_ip(ip("192.168.1.9")));
        let far = Some(Peer::from_ip(ip("203.0.113.9")));
        // This machine always may, remote access or not.
        for peer in [here, also_here, mapped_here] {
            assert!(peer_may_use(false, &[], peer));
            assert!(peer_may_use(true, &lan, peer));
        }
        // Another machine needs remote access, and then a matching range if there are any.
        assert!(!peer_may_use(false, &[], near));
        assert!(!peer_may_use(false, &lan, near));
        assert!(peer_may_use(true, &[], far));
        assert!(peer_may_use(true, &lan, near));
        assert!(!peer_may_use(true, &lan, far));
        // Where it came from unknown: not let in on the strength of being local.
        assert!(!peer_may_use(false, &[], None));
        assert!(!peer_may_use(true, &[], None));
    }

    #[test]
    fn this_machines_own_address_is_this_machine_and_no_other_address_is() {
        let own = ip("192.168.1.20");
        // The server listens on 192.168.1.20 and a client on the same Mac connects to it: the peer
        // is that same address. Loopback is this machine whatever the server listens on.
        assert!(Peer::of_connection(own, own).local);
        assert!(Peer::of_connection(ip("127.0.0.1"), own).local);
        assert!(Peer::of_connection(ip("::1"), ip("fd00::20")).local);
        // Another host on the same network is not, nor is a neighbouring address.
        assert!(!Peer::of_connection(ip("192.168.1.21"), own).local);
        assert!(!Peer::of_connection(ip("203.0.113.9"), own).local);
        // A mapped IPv4 address is the IPv4 address, on either side.
        assert!(Peer::of_connection(ip("::ffff:192.168.1.20"), own).local);
        assert!(Peer::of_connection(own, ip("::ffff:192.168.1.20")).local);
        assert_eq!(
            Peer::of_connection(ip("::ffff:10.0.0.7"), own).ip,
            ip("10.0.0.7")
        );
        // Known only by address, nothing to compare with: loopback only.
        assert!(Peer::from_ip(ip("127.0.0.1")).local);
        assert!(!Peer::from_ip(own).local);

        // So a credential that may not leave this machine works from its own address, and a
        // credential with ranges is not held to them there.
        let lan = [cidr("10.0.0.0/8")];
        let here = Some(Peer::of_connection(own, own));
        assert!(peer_may_use(false, &[], here));
        assert!(peer_may_use(true, &lan, here));
        let other = Some(Peer::of_connection(ip("192.168.1.21"), own));
        assert!(!peer_may_use(false, &[], other));
        assert!(!peer_may_use(true, &lan, other));
    }

    fn remote_client(remote: bool, grants: &[Permission]) -> McpClient {
        McpClient {
            id: "c".into(),
            name: "c".into(),
            verifier: "v".into(),
            permissions: PermissionSet::of(grants.iter().copied()),
            created_at_ms: 1,
            last_used_ms: None,
            remote_allowed: remote,
            allowed_cidrs: Vec::new(),
        }
    }

    #[test]
    fn the_remote_ceiling_caps_a_remote_client_whichever_address_it_calls_from() {
        let grants = [Permission::ChartsRead, Permission::ChartsCreate];
        let global = PermissionSet::everything();
        let ceiling = PermissionSet::of([Permission::ChartsRead]);
        let local = remote_client(false, &grants).effective(&global, &ceiling);
        assert!(local.contains(Permission::ChartsCreate));
        let remote = remote_client(true, &grants).effective(&global, &ceiling);
        assert!(remote.contains(Permission::ChartsRead));
        assert!(!remote.contains(Permission::ChartsCreate));
        // The global cap still applies on top.
        let none = remote_client(true, &grants).effective(&PermissionSet::none(), &ceiling);
        assert!(none.is_empty());
    }

    #[test]
    fn the_default_ceiling_is_reads_only() {
        let ceiling = default_remote_ceiling();
        assert!(ceiling.iter().all(|p| !p.is_mutating()));
        assert!(ceiling.contains(Permission::ChartsRead));
        assert!(!ceiling.contains(Permission::ChartsCreate));
    }

    #[test]
    fn remote_fields_are_stored_only_when_set_so_old_rows_read_back_unchanged() {
        let plain = serde_json::to_string(&remote_client(false, &[])).unwrap();
        assert!(
            !plain.contains("remote") && !plain.contains("allowed"),
            "{plain}"
        );
        let old: McpClient = serde_json::from_str(&plain).unwrap();
        assert!(!old.remote_allowed && old.allowed_cidrs.is_empty());

        let mut set = remote_client(true, &[]);
        set.allowed_cidrs = vec![cidr("10.0.0.0/8")];
        let stored = serde_json::to_string(&set).unwrap();
        assert!(stored.contains(r#""remoteAllowed":true"#), "{stored}");
        assert!(
            stored.contains(r#""allowedCidrs":["10.0.0.0/8"]"#),
            "{stored}"
        );
        assert_eq!(serde_json::from_str::<McpClient>(&stored).unwrap(), set);

        // The ceiling too: absent while default, round-trips when changed.
        assert!(
            !serde_json::to_string(&McpSettings::default())
                .unwrap()
                .contains("ceiling")
        );
        let changed = McpSettings {
            remote_ceiling: PermissionSet::of([Permission::ChartsRead, Permission::AlertsRead]),
            ..McpSettings::default()
        };
        let stored = serde_json::to_string(&changed).unwrap();
        assert_eq!(
            serde_json::from_str::<McpSettings>(&stored).unwrap(),
            changed
        );
    }

    #[test]
    fn loopback_addresses_are_the_loopback_mode() {
        assert_eq!(BindMode::interface(ip("127.0.0.1")), Ok(BindMode::Loopback));
        assert_eq!(BindMode::interface(ip("127.5.5.5")), Ok(BindMode::Loopback));
        assert_eq!(BindMode::interface(ip("::1")), Ok(BindMode::Loopback));
    }

    #[test]
    fn an_interface_is_one_named_address_never_all_of_them() {
        assert_eq!(
            BindMode::interface(ip("0.0.0.0")),
            Err(BindError::Unspecified)
        );
        assert_eq!(BindMode::interface(ip("::")), Err(BindError::Unspecified));
        assert_eq!(
            BindMode::interface(ip("224.0.0.1")),
            Err(BindError::Multicast)
        );
        assert_eq!(
            BindMode::interface(ip("255.255.255.255")),
            Err(BindError::Broadcast)
        );
        assert_eq!(
            BindMode::interface(ip("fe80::1")),
            Err(BindError::LinkLocalV6)
        );
        assert_eq!(
            BindMode::interface(ip("192.168.1.20")),
            Ok(BindMode::Interface(ip("192.168.1.20")))
        );
        assert_eq!(
            BindMode::interface(ip("fd00::20")),
            Ok(BindMode::Interface(ip("fd00::20")))
        );
    }

    #[test]
    fn bind_mode_decides_scheme_endpoint_and_hosts() {
        let loopback = BindMode::Loopback;
        assert!(!loopback.requires_tls());
        assert_eq!(loopback.scheme(), "http");
        assert_eq!(
            loopback.allowed_hosts(9000),
            ["127.0.0.1:9000", "localhost:9000"]
        );

        let lan = BindMode::interface(ip("192.168.1.20")).unwrap();
        assert!(lan.requires_tls());
        assert_eq!(lan.allowed_hosts(9000), ["192.168.1.20:9000"]);
        let settings = McpSettings {
            bind: lan,
            port: 9000,
            ..McpSettings::default()
        };
        assert_eq!(settings.endpoint(), "https://192.168.1.20:9000/mcp");

        let v6 = BindMode::interface(ip("fd00::20")).unwrap();
        assert_eq!(v6.allowed_hosts(9000), ["[fd00::20]:9000"]);
    }

    #[test]
    fn bind_mode_is_stored_only_when_it_is_not_loopback() {
        let stored = serde_json::to_string(&McpSettings::default()).unwrap();
        assert!(!stored.contains("bind"), "{stored}");

        let lan = McpSettings {
            bind: BindMode::interface(ip("10.0.0.7")).unwrap(),
            ..McpSettings::default()
        };
        let stored = serde_json::to_string(&lan).unwrap();
        assert!(stored.contains(r#""bind":"10.0.0.7""#), "{stored}");
        assert_eq!(serde_json::from_str::<McpSettings>(&stored).unwrap(), lan);
    }

    #[test]
    fn an_unreadable_or_all_interfaces_bind_fails_the_load() {
        for bad in ["0.0.0.0", "::", "lan", "", "fe80::1"] {
            let json = format!(r#"{{"bind":"{bad}"}}"#);
            assert!(
                serde_json::from_str::<McpSettings>(&json).is_err(),
                "{bad} must not load"
            );
        }
        let explicit: McpSettings = serde_json::from_str(r#"{"bind":"loopback"}"#).unwrap();
        assert_eq!(explicit, McpSettings::default());
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
