//! What synchronizes, per data category. Every document table and `setting` key is classified
//! here, and a test fails when a new table is added without a decision. Anything unclassified
//! defaults to local-only: syncing is opt-in per entity, never by accident.

/// The user-facing switch a synced entity belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Watchlists,
    Layouts,
    Templates,
    Scripts,
    Alerts,
    Preferences,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Self::Watchlists,
        Self::Layouts,
        Self::Templates,
        Self::Scripts,
        Self::Alerts,
        Self::Preferences,
    ];

    /// The stable name a switch is stored under.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Watchlists => "watchlists",
            Self::Layouts => "layouts",
            Self::Templates => "templates",
            Self::Scripts => "scripts",
            Self::Alerts => "alerts",
            Self::Preferences => "preferences",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Watchlists => "Watchlists",
            Self::Layouts => "Layouts and drawings",
            Self::Templates => "Chart and drawing templates",
            Self::Scripts => "Pine scripts",
            Self::Alerts => "Price alerts",
            Self::Preferences => "Preferences",
        }
    }
}

/// How concurrent edits to one entity are reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// Higher revision wins; ties break on (modified time, device id).
    LastWriterWins,
    /// Merge against the last synced payload, entry by entry.
    ThreeWay,
    /// Revision-aware; the losing version survives as a separate "conflict copy".
    KeepBoth,
}

/// A kind of CloudKit record. `prefix` is the first part of its record name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Layout,
    LayoutDrawing,
    Watchlist,
    ChartTemplate,
    DrawingTemplate,
    Script,
    Alert,
    Setting,
}

impl EntityKind {
    pub const ALL: [EntityKind; 8] = [
        Self::Layout,
        Self::LayoutDrawing,
        Self::Watchlist,
        Self::ChartTemplate,
        Self::DrawingTemplate,
        Self::Script,
        Self::Alert,
        Self::Setting,
    ];

    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::LayoutDrawing => "layoutdrawing",
            Self::Watchlist => "watchlist",
            Self::ChartTemplate => "chart_template",
            Self::DrawingTemplate => "drawing_template",
            Self::Script => "script",
            Self::Alert => "alert",
            Self::Setting => "setting",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.prefix() == prefix)
    }

    pub const fn category(self) -> Category {
        match self {
            Self::Layout | Self::LayoutDrawing => Category::Layouts,
            Self::Watchlist => Category::Watchlists,
            Self::ChartTemplate | Self::DrawingTemplate => Category::Templates,
            Self::Script => Category::Scripts,
            Self::Alert => Category::Alerts,
            Self::Setting => Category::Preferences,
        }
    }

    pub const fn merge(self) -> Merge {
        match self {
            Self::Watchlist => Merge::ThreeWay,
            Self::Layout | Self::Script => Merge::KeepBoth,
            Self::LayoutDrawing
            | Self::ChartTemplate
            | Self::DrawingTemplate
            | Self::Alert
            | Self::Setting => Merge::LastWriterWins,
        }
    }
}

/// The decision for one document table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TablePolicy {
    /// Rows sync as this entity kind.
    Synced(EntityKind),
    /// Never leaves this Mac, for the stated reason.
    Local(&'static str),
}

/// Classifies a document table. Unknown tables are local.
pub fn table_policy(table: &str) -> TablePolicy {
    match table {
        "saved_view" => TablePolicy::Synced(EntityKind::Layout),
        "watchlist" => TablePolicy::Synced(EntityKind::Watchlist),
        "chart_template" => TablePolicy::Synced(EntityKind::ChartTemplate),
        "drawing_template" => TablePolicy::Synced(EntityKind::DrawingTemplate),
        "price_alert" => TablePolicy::Synced(EntityKind::Alert),
        "tab" => {
            TablePolicy::Local("per-device session; tab and chart ids are sequential counters")
        }
        "drawing" => TablePolicy::Local("unused; drawings live inside layouts"),
        "portfolio" => TablePolicy::Local("financial ledger; separate design needed"),
        "paper_account" => TablePolicy::Local("transactional paper-trading state"),
        "webhook_endpoint" => TablePolicy::Local(
            "plaintext URL templates; secrets are Keychain-only and do not travel",
        ),
        "pine_alert_subscription" => {
            TablePolicy::Local("binds device-local chart and instance ids")
        }
        "mcp_client" => {
            TablePolicy::Local("security state; a verifier must not grant access elsewhere")
        }
        _ => TablePolicy::Local("unclassified"),
    }
}

/// The decision for one `setting` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingPolicy {
    /// Syncs as `setting.<key>`, merged per `Merge`.
    Portable(Merge),
    Local,
}

/// Classifies a `setting` key. Unknown keys are local.
pub fn setting_policy(key: &str) -> SettingPolicy {
    match key {
        "appTheme" | "drawing.defaults" | "drawing.prefs" | "snapshot.prefs" => {
            SettingPolicy::Portable(Merge::LastWriterWins)
        }
        "watchlist.flags" => SettingPolicy::Portable(Merge::ThreeWay),
        _ => SettingPolicy::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DOCUMENT_TABLES;

    #[test]
    fn every_document_table_is_classified() {
        for table in DOCUMENT_TABLES {
            assert_ne!(
                table_policy(table),
                TablePolicy::Local("unclassified"),
                "{table} needs a sync decision in sync::policy::table_policy"
            );
        }
    }

    #[test]
    fn synced_tables_are_the_agreed_set() {
        let synced: Vec<_> = DOCUMENT_TABLES
            .into_iter()
            .filter(|table| matches!(table_policy(table), TablePolicy::Synced(_)))
            .collect();
        assert_eq!(
            synced,
            [
                "saved_view",
                "chart_template",
                "drawing_template",
                "watchlist",
                "price_alert"
            ]
        );
    }

    #[test]
    fn secrets_and_machine_state_never_sync() {
        for table in [
            "mcp_client",
            "webhook_endpoint",
            "tab",
            "paper_account",
            "portfolio",
        ] {
            assert!(
                matches!(table_policy(table), TablePolicy::Local(_)),
                "{table}"
            );
        }
        for key in [
            "mcp.settings",
            "mcp.permissions",
            "alert.health",
            "alert.revision",
            "alert.processedCommandIDs",
            "alerts.lastSeenAt",
            "session.next_id",
            "scriptManager.layout",
            "scriptPreview.market",
            "scripts.demosSeeded",
            "addTicker.recentMarkets",
            "paper.selectedAccountID",
            "portfolio.privacyMode",
        ] {
            assert_eq!(setting_policy(key), SettingPolicy::Local, "{key}");
        }
    }

    #[test]
    fn unknown_settings_default_to_local() {
        assert_eq!(setting_policy("someFutureKey"), SettingPolicy::Local);
        assert_eq!(
            table_policy("some_future_table"),
            TablePolicy::Local("unclassified")
        );
    }

    #[test]
    fn portable_settings_are_listed() {
        assert_eq!(
            setting_policy("appTheme"),
            SettingPolicy::Portable(Merge::LastWriterWins)
        );
        assert_eq!(
            setting_policy("watchlist.flags"),
            SettingPolicy::Portable(Merge::ThreeWay)
        );
    }

    #[test]
    fn entity_prefixes_round_trip_and_are_unique() {
        for kind in EntityKind::ALL {
            assert_eq!(EntityKind::from_prefix(kind.prefix()), Some(kind));
        }
        let mut prefixes: Vec<_> = EntityKind::ALL.iter().map(|k| k.prefix()).collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        assert_eq!(prefixes.len(), EntityKind::ALL.len());
        assert_eq!(EntityKind::from_prefix("tab"), None);
    }

    #[test]
    fn script_and_layout_conflicts_keep_both_versions() {
        assert_eq!(EntityKind::Script.merge(), Merge::KeepBoth);
        assert_eq!(EntityKind::Layout.merge(), Merge::KeepBoth);
        assert_eq!(EntityKind::Watchlist.merge(), Merge::ThreeWay);
    }

    #[test]
    fn categories_have_unique_stable_keys_and_every_kind_belongs_to_one() {
        let mut keys: Vec<_> = Category::ALL.iter().map(|c| c.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Category::ALL.len());
        for kind in EntityKind::ALL {
            assert!(Category::ALL.contains(&kind.category()));
        }
    }

    #[test]
    fn icloud_gate_matches_build() {
        assert_eq!(
            crate::platform::has_icloud_sync(),
            cfg!(all(target_os = "macos", feature = "icloud"))
        );
    }
}
