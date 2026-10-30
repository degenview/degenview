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
    Portfolios,
    PaperTrading,
    Preferences,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Self::Watchlists,
        Self::Layouts,
        Self::Templates,
        Self::Scripts,
        Self::Alerts,
        Self::Portfolios,
        Self::PaperTrading,
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
            Self::Portfolios => "portfolios",
            Self::PaperTrading => "paper_trading",
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
            Self::Portfolios => "Portfolios",
            Self::PaperTrading => "Paper trading",
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
    /// An image a drawing shows, kept once by the hash of its bytes.
    DrawingAsset,
    Watchlist,
    ChartTemplate,
    DrawingTemplate,
    Script,
    Alert,
    Portfolio,
    PortfolioTransaction,
    /// One paper account with everything that belongs to it, as a single record.
    PaperAccount,
    Setting,
}

impl EntityKind {
    pub const ALL: [EntityKind; 12] = [
        Self::Layout,
        Self::LayoutDrawing,
        Self::DrawingAsset,
        Self::Watchlist,
        Self::ChartTemplate,
        Self::DrawingTemplate,
        Self::Script,
        Self::Alert,
        Self::Portfolio,
        Self::PortfolioTransaction,
        Self::PaperAccount,
        Self::Setting,
    ];

    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::LayoutDrawing => "layoutdrawing",
            Self::DrawingAsset => "drawingasset",
            Self::Watchlist => "watchlist",
            Self::ChartTemplate => "chart_template",
            Self::DrawingTemplate => "drawing_template",
            Self::Script => "script",
            Self::Alert => "alert",
            Self::Portfolio => "portfolio",
            Self::PortfolioTransaction => "portfoliotx",
            Self::PaperAccount => "paperaccount",
            Self::Setting => "setting",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.prefix() == prefix)
    }

    pub const fn category(self) -> Category {
        match self {
            Self::Layout | Self::LayoutDrawing | Self::DrawingAsset => Category::Layouts,
            Self::Watchlist => Category::Watchlists,
            Self::ChartTemplate | Self::DrawingTemplate => Category::Templates,
            Self::Script => Category::Scripts,
            Self::Alert => Category::Alerts,
            Self::Portfolio | Self::PortfolioTransaction => Category::Portfolios,
            Self::PaperAccount => Category::PaperTrading,
            Self::Setting => Category::Preferences,
        }
    }

    pub const fn merge(self) -> Merge {
        match self {
            Self::Watchlist => Merge::ThreeWay,
            Self::Layout | Self::Script | Self::PaperAccount => Merge::KeepBoth,
            Self::LayoutDrawing
            | Self::DrawingAsset
            | Self::ChartTemplate
            | Self::DrawingTemplate
            | Self::Alert
            | Self::Portfolio
            | Self::PortfolioTransaction
            | Self::Setting => Merge::LastWriterWins,
        }
    }
}

impl EntityKind {
    /// Whether the apply layer settles two Macs' concurrent edits itself (a watchlist entry by
    /// entry, a setting key by key, a layout, script or paper account into a winner and a
    /// conflict copy). A refused upload of such an entity is merged with the cloud's version and
    /// the result uploaded; saving it over the cloud's version would drop the other Mac's change.
    pub const fn combines_edits(self) -> bool {
        matches!(
            self,
            Self::Watchlist | Self::Setting | Self::Layout | Self::Script | Self::PaperAccount
        )
    }
}

/// The decision for one document table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TablePolicy {
    /// Rows sync as this entity kind.
    Synced(EntityKind),
    /// Rows are not entities of their own: a custom write path builds one record per owner out
    /// of several tables (a paper account and everything it owns). The generic row hook ignores
    /// the table.
    Bundled(EntityKind),
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
        "editor_theme" => {
            TablePolicy::Local("custom editor themes; syncing them is a later change")
        }
        "price_alert" => TablePolicy::Synced(EntityKind::Alert),
        "tab" => {
            TablePolicy::Local("per-device session; tab and chart ids are sequential counters")
        }
        "drawing" => TablePolicy::Local("unused; drawings live inside layouts"),
        "drawing_asset" => TablePolicy::Synced(EntityKind::DrawingAsset),
        "portfolio" => TablePolicy::Synced(EntityKind::Portfolio),
        "paper_account" => TablePolicy::Bundled(EntityKind::PaperAccount),
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

/// The switch an entity belongs to. A setting follows its key: flags are marks on the markets in
/// the user's watchlists, so they travel with Watchlists rather than Preferences.
pub fn category_of(kind: EntityKind, sync_id: &str) -> Category {
    match kind {
        EntityKind::Setting => setting_category(sync_id).unwrap_or(Category::Preferences),
        _ => kind.category(),
    }
}

/// How an entity's concurrent edits merge. A setting follows its key's policy.
pub fn merge_of(kind: EntityKind, sync_id: &str) -> Merge {
    match (kind, setting_policy(sync_id)) {
        (EntityKind::Setting, SettingPolicy::Portable(merge)) => merge,
        _ => kind.merge(),
    }
}

fn setting_category(key: &str) -> Option<Category> {
    match key {
        "watchlist.flags" => Some(Category::Watchlists),
        _ => None,
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
                "portfolio",
                "price_alert",
                "drawing_asset"
            ]
        );
    }

    #[test]
    fn a_paper_account_syncs_as_one_bundle_not_row_by_row() {
        assert_eq!(
            table_policy("paper_account"),
            TablePolicy::Bundled(EntityKind::PaperAccount)
        );
        assert_eq!(EntityKind::PaperAccount.merge(), Merge::KeepBoth);
        assert_eq!(EntityKind::PaperAccount.category(), Category::PaperTrading);
        assert_eq!(
            EntityKind::PortfolioTransaction.category(),
            Category::Portfolios
        );
        assert_eq!(EntityKind::Portfolio.merge(), Merge::LastWriterWins);
    }

    #[test]
    fn secrets_and_machine_state_never_sync() {
        for table in ["mcp_client", "webhook_endpoint", "tab"] {
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
            "pineLogs.prefs",
            "scriptDebug.state",
            // Names this user's own account and server.
            "ai.prefs",
            // Their prompts about their own scripts, and the model's replies.
            "ai.history",
            "scriptPreview.market",
            "scripts.demosSeeded",
            "addTicker.recentMarkets",
            "paper.selectedAccountID",
            "portfolio.privacyMode",
            // A channel is chosen per Mac: syncing it would move every other Mac onto it.
            "update.channel",
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
    fn flags_sync_with_watchlists_and_merge_per_key() {
        let flags = EntityKind::Setting;
        assert_eq!(category_of(flags, "watchlist.flags"), Category::Watchlists);
        assert_eq!(merge_of(flags, "watchlist.flags"), Merge::ThreeWay);
        assert_eq!(category_of(flags, "appTheme"), Category::Preferences);
        assert_eq!(merge_of(flags, "appTheme"), Merge::LastWriterWins);
        assert_eq!(
            category_of(EntityKind::Watchlist, "1"),
            Category::Watchlists
        );
        assert_eq!(merge_of(EntityKind::Watchlist, "1"), Merge::ThreeWay);
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
