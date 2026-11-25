//! Cached host snapshots for the Settings paint path.
//!
//! Every mouse move inside Settings used to re-clone alerts, MCP clients and related host state.
//! Charts never did that, so Settings felt sticky while charts stayed fine. This cache is refreshed
//! at most a few times a second, and only loads the heavy pieces the open section needs.

use crate::mcp::state::McpStatus;
use crate::model::alert::pine::PineAlertSubscription;
use crate::model::alert::{AlertNotificationSettings, AlertRuntimeHealth, PriceAlert};
use crate::model::mcp::{McpAuditEntry, McpClient, McpSettings, PermissionSet};
use crate::ui::settings::window::SettingsSection;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a paint snapshot stays good while the pointer keeps moving.
pub(super) const FRESH: Duration = Duration::from_millis(300);

/// What one Settings paint needs from the hosts, already cloned.
#[derive(Clone)]
pub struct SettingsPaintData {
    pub at: Instant,
    pub section: SettingsSection,
    pub notifications: AlertNotificationSettings,
    pub health: AlertRuntimeHealth,
    /// Filled when the Webhooks page (or an endpoint delete) needs reference counts.
    pub alerts: Arc<[PriceAlert]>,
    pub subscriptions: Arc<[PineAlertSubscription]>,
    pub mcp: McpPaintData,
}

#[derive(Clone)]
pub struct McpPaintData {
    pub status: McpStatus,
    pub settings: McpSettings,
    pub clients: Arc<[McpClient]>,
    pub global: PermissionSet,
    pub fresh_token: Option<(String, String)>,
    pub load_error: Option<String>,
    pub notice: Option<String>,
    pub can_reset: bool,
    pub activity: Arc<[McpAuditEntry]>,
    pub activity_unreadable: usize,
    pub tls_fingerprint: Option<String>,
}

impl Default for McpPaintData {
    fn default() -> Self {
        Self {
            status: McpStatus::Disabled,
            settings: McpSettings::default(),
            clients: Arc::from([]),
            global: PermissionSet::none(),
            fresh_token: None,
            load_error: None,
            notice: None,
            can_reset: false,
            activity: Arc::from([]),
            activity_unreadable: 0,
            tls_fingerprint: None,
        }
    }
}

impl SettingsPaintData {
    /// Whether this snapshot can serve another paint of `section`.
    pub fn serves(&self, section: SettingsSection) -> bool {
        self.section == section && self.at.elapsed() < FRESH
    }

    pub fn needs_webhook_refs(section: SettingsSection) -> bool {
        matches!(section, SettingsSection::Webhooks)
    }

    pub fn needs_mcp(section: SettingsSection) -> bool {
        matches!(section, SettingsSection::Mcp)
    }

    pub fn needs_notifications(section: SettingsSection) -> bool {
        matches!(
            section,
            SettingsSection::Notifications | SettingsSection::Webhooks | SettingsSection::Mcp
        )
    }
}
