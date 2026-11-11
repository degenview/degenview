//! The app's Settings window: a sidebar of badge-and-title rows beside the selected page —
//! Appearance, Alpaca, CoinMarketCap, Notifications (how alerts are delivered, and the
//! background agent's health), Webhooks, Storage (the cache) and About.
//!
//! A pure view: it reads settings values and returns `SettingsAction`s for the host.

// Transient: the app shell opens this window once the settings host is wired.
#![allow(dead_code)]

use crate::model::alert::{
    AlertNotificationPermission, AlertNotificationSettings, AlertRuntimeHealth, AlertServiceState,
};
use crate::model::mcp::McpAction;
use crate::platform;
use crate::service::keychain_api_keys::{AlpacaCredentialsStore, CoinMarketCapKeyStore};
use crate::ui::alert_center::{Tone, relative_time};
use crate::ui::icons::{self, Icon};
use crate::ui::settings_api_keys::{self, ApiKeysState};
use crate::ui::settings_mcp::{self, McpSettingsInput, McpSettingsState};
use crate::ui::settings_storage::{self, StorageAction, StorageState};
use crate::ui::webhook_settings::{
    self, WebhookSettingsAction, WebhookSettingsInput, WebhookSettingsState,
};
use crate::ui::{kit, settings_card, theme};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

pub const SIDEBAR_WIDTH: f32 = 224.0;
/// The Settings window: the sidebar and a 616-point page.
pub const WINDOW_SIZE: Vec2 = Vec2::new(840.0, 600.0);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SettingsSection {
    #[default]
    Appearance,
    PineStudio,
    Alpaca,
    Brokers,
    CoinMarketCap,
    Ai,
    Notifications,
    Webhooks,
    Snapshots,
    Mcp,
    Sync,
    Storage,
    About,
}

impl SettingsSection {
    pub const ALL: [Self; 13] = [
        Self::Appearance,
        Self::PineStudio,
        Self::Alpaca,
        Self::Brokers,
        Self::CoinMarketCap,
        Self::Ai,
        Self::Notifications,
        Self::Webhooks,
        Self::Snapshots,
        Self::Mcp,
        Self::Sync,
        Self::Storage,
        Self::About,
    ];

    /// The sections this build shows. iCloud Sync only exists where it can work.
    pub fn visible() -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(|section| *section != Self::Sync || crate::platform::has_icloud_sync())
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::PineStudio => "Pine Studio",
            Self::Alpaca => "Alpaca",
            Self::Brokers => "Brokers",
            Self::CoinMarketCap => "CoinMarketCap",
            Self::Ai => "AI Assistant",
            Self::Notifications => "Notifications",
            Self::Webhooks => "Webhooks",
            Self::Snapshots => "Snapshots",
            Self::Storage => "Storage",
            Self::Mcp => "MCP Server",
            Self::Sync => "iCloud Sync",
            Self::About => "About",
        }
    }

    /// The sidebar badge colour, as in the system Settings app.
    pub fn tint(self) -> Color32 {
        match self {
            Self::Appearance => Color32::from_rgb(94, 92, 230),
            Self::PineStudio => Color32::from_rgb(172, 142, 104),
            Self::Alpaca => Color32::from_rgb(255, 159, 10),
            Self::Brokers => Color32::from_rgb(255, 112, 67),
            Self::CoinMarketCap => Color32::from_rgb(10, 132, 255),
            Self::Ai => Color32::from_rgb(255, 214, 10),
            Self::Notifications => Color32::from_rgb(255, 69, 58),
            Self::Webhooks => Color32::from_rgb(48, 176, 199),
            Self::Snapshots => Color32::from_rgb(255, 55, 95),
            Self::Storage => Color32::from_rgb(52, 199, 89),
            Self::Mcp => Color32::from_rgb(191, 90, 242),
            Self::Sync => Color32::from_rgb(64, 156, 255),
            Self::About => Color32::from_rgb(142, 142, 147),
        }
    }

    /// The stable name the selection is remembered under.
    pub fn key(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::PineStudio => "pinestudio",
            Self::Alpaca => "alpaca",
            Self::Brokers => "brokers",
            Self::CoinMarketCap => "coinmarketcap",
            Self::Ai => "ai",
            Self::Notifications => "notifications",
            Self::Webhooks => "webhooks",
            Self::Snapshots => "snapshots",
            Self::Storage => "storage",
            Self::Mcp => "mcp",
            Self::Sync => "icloud",
            Self::About => "about",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    Webhook(WebhookSettingsAction),
    /// A request from the Storage page: read the cache sizes, or clear the cache.
    Storage(StorageAction),
    /// A theme tile on the Appearance page was picked.
    SetTheme(crate::ui::theme::AppTheme),
    /// A tab title style on the Appearance page was picked (or restored to the default).
    SetTabTitleStyle(crate::model::tab_title::TabTitleStyle),
    /// A switch on the Notifications page changed: the whole new settings value.
    UpdateNotifications(AlertNotificationSettings),
    RetryRegistration,
    OpenLoginItems,
    OpenNotificationSettings,
    /// The Snapshots page changed something about sharing or the images.
    Snapshots(crate::ui::settings_snapshots::SnapshotsAction),
    /// The MCP Server page asked the MCP host to change something.
    Mcp(McpAction),
    /// The AI Assistant page changed the provider or model.
    Ai(crate::ui::settings_ai::AiAction),
    /// An update was swapped in and a helper will reopen the app: quit now.
    QuitForUpdate,
    /// The About page chose which releases updates offer.
    SetUpdateChannel(crate::service::update_check::Channel),
}

pub struct SettingsInput<'a> {
    pub notifications: &'a AlertNotificationSettings,
    pub health: &'a AlertRuntimeHealth,
    pub webhooks: WebhookSettingsInput<'a>,
    pub mcp: McpSettingsInput<'a>,
    pub snapshots: crate::ui::settings_snapshots::SnapshotsInput<'a>,
    pub ai: crate::ui::settings_ai::AiSettingsInput<'a>,
    pub updates: &'a crate::host::update_host::UpdateHost,
    pub sync: &'a crate::host::sync_host::SyncHost,
    pub editor_themes: &'a crate::host::editor_theme_host::EditorThemeHost,
    pub tab_title_style: crate::model::tab_title::TabTitleStyle,
    pub now_ms: i64,
}

#[derive(Default)]
pub struct SettingsState {
    pub section: SettingsSection,
    pub webhooks: WebhookSettingsState,
    pub api_keys: ApiKeysState,
    pub brokers: crate::ui::settings_brokers::BrokersState,
    pub ai: crate::ui::settings_ai::AiSettingsState,
    pub storage: StorageState,
    pub mcp: McpSettingsState,
    pub snapshots: crate::ui::settings_snapshots::SnapshotsState,
    pub pine_studio: crate::ui::settings_pine_studio::PineStudioState,
}

// MARK: Pure labels

/// The background agent's state in words.
pub fn service_label(state: AlertServiceState) -> &'static str {
    match state {
        AlertServiceState::Enabled => "Running in background",
        AlertServiceState::RequiresApproval => "Needs approval in Login Items",
        AlertServiceState::Disabled => "Foreground only",
        AlertServiceState::NotFound => "Agent not embedded",
        AlertServiceState::Failed => "Registration failed",
        AlertServiceState::Unavailable => "Unavailable",
    }
}

pub fn service_tone(state: AlertServiceState) -> Tone {
    match state {
        AlertServiceState::Enabled => Tone::Good,
        AlertServiceState::RequiresApproval => Tone::Warning,
        AlertServiceState::Failed | AlertServiceState::NotFound => Tone::Bad,
        AlertServiceState::Disabled | AlertServiceState::Unavailable => Tone::Neutral,
    }
}

pub fn permission_label(permission: AlertNotificationPermission) -> &'static str {
    match permission {
        AlertNotificationPermission::Authorized => "Authorized",
        AlertNotificationPermission::Provisional => "Provisional",
        AlertNotificationPermission::Ephemeral => "Ephemeral",
        AlertNotificationPermission::Denied => "Denied",
        AlertNotificationPermission::NotDetermined => "Not requested",
        AlertNotificationPermission::Unknown => "Unknown",
    }
}

pub fn permission_tone(permission: AlertNotificationPermission) -> Tone {
    match permission {
        AlertNotificationPermission::Authorized
        | AlertNotificationPermission::Provisional
        | AlertNotificationPermission::Ephemeral => Tone::Good,
        AlertNotificationPermission::Denied => Tone::Bad,
        AlertNotificationPermission::NotDetermined | AlertNotificationPermission::Unknown => {
            Tone::Neutral
        }
    }
}

/// A provider's line: its last error, or whether data has arrived yet.
pub fn provider_label(last_error: Option<&str>, has_quote: bool) -> String {
    match last_error {
        Some(error) => error.to_owned(),
        None if has_quote => "Healthy".to_owned(),
        None => "Waiting for data".to_owned(),
    }
}

/// "3 unreconciled data gaps".
pub fn gaps_title(count: usize) -> String {
    format!(
        "{count} unreconciled data {}",
        if count == 1 { "gap" } else { "gaps" }
    )
}

/// One switch on the Notifications page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationSwitch {
    Delivery,
    Sound,
    InAppBanners,
    MacosNotifications,
}

impl NotificationSwitch {
    pub const ALL: [Self; 4] = [
        Self::Delivery,
        Self::Sound,
        Self::InAppBanners,
        Self::MacosNotifications,
    ];

    /// The switches this platform can honour: macOS Notifications exists only on macOS.
    pub fn available() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|switch| {
            *switch != Self::MacosNotifications || platform::has_system_notifications()
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Delivery => "Price Alert Delivery",
            Self::Sound => "Sound",
            Self::InAppBanners => "In-App Banners",
            Self::MacosNotifications => "macOS Notifications",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Delivery => {
                "Evaluate armed alerts and deliver them when their conditions are met."
            }
            Self::Sound => "Play an alert sound when a notification is delivered.",
            Self::InAppBanners => "Show alerts inside DegenView while the app is open.",
            Self::MacosNotifications => {
                "Send alerts through Notification Center, including in the background."
            }
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Delivery => "🔔",
            Self::Sound => "🔊",
            Self::InAppBanners => "▭",
            Self::MacosNotifications => "⌘",
        }
    }

    pub fn get(self, settings: &AlertNotificationSettings) -> bool {
        match self {
            Self::Delivery => settings.delivery_enabled,
            Self::Sound => settings.sound_enabled,
            Self::InAppBanners => settings.in_app_banners_enabled,
            Self::MacosNotifications => settings.macos_notifications_enabled,
        }
    }

    /// The settings with this switch set; everything else is untouched.
    pub fn set(
        self,
        settings: &AlertNotificationSettings,
        value: bool,
    ) -> AlertNotificationSettings {
        let mut updated = settings.clone();
        match self {
            Self::Delivery => updated.delivery_enabled = value,
            Self::Sound => updated.sound_enabled = value,
            Self::InAppBanners => updated.in_app_banners_enabled = value,
            Self::MacosNotifications => updated.macos_notifications_enabled = value,
        }
        updated
    }
}

// MARK: View

pub(super) const INSET: f32 = 24.0;

impl SettingsSection {
    pub(super) fn icon(self) -> Icon {
        match self {
            Self::Appearance => Icon::Paintbrush,
            Self::PineStudio => Icon::Curlybraces,
            Self::Alpaca => Icon::ChartLine,
            Self::Brokers => Icon::ArrowLeftRight,
            Self::CoinMarketCap => Icon::Gauge,
            Self::Ai => Icon::Sparkles,
            Self::Notifications => Icon::BellBadge,
            Self::Webhooks => Icon::ArrowUpRightSquare,
            Self::Snapshots => Icon::Camera,
            Self::Storage => Icon::Layers,
            Self::Mcp => Icon::LockFill,
            Self::Sync => Icon::Cycle,
            Self::About => Icon::InfoCircle,
        }
    }
}

/// A status pill laid out in the flow, sized to its label.
pub(super) fn status_badge(ui: &mut egui::Ui, text: &str, tone: Tone) {
    let probe = kit::galley(
        ui,
        text,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    let size = Vec2::new(9.0 + 7.0 + 6.0 + probe.size().x + 9.0, probe.size().y + 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    kit::paint_status_badge(ui, rect.right(), rect.center().y, text, tone);
}

/// A status line in a page's body: the icon, then the message, wrapped to the page.
pub(super) fn status_message(ui: &mut egui::Ui, text: &str, is_error: bool) {
    let color = if is_error { kit::RED } else { kit::GREEN };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::inline(
            ui,
            if is_error {
                Icon::TriangleFill
            } else {
                Icon::CheckCircleFill
            },
            12.0,
            color,
        );
        ui.add(
            egui::Label::new(kit::text(
                text,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                color,
            ))
            .wrap(),
        );
    });
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    input: &SettingsInput,
) -> Vec<SettingsAction> {
    let mut actions = Vec::new();
    let full = ui.ctx().content_rect();
    ui.ctx()
        .layer_painter(egui::LayerId::background())
        .rect_filled(full, 0.0, kit::window());
    let sidebar_rect = Rect::from_min_size(full.min, Vec2::new(SIDEBAR_WIDTH, full.height()));
    ui.painter()
        .rect_filled(sidebar_rect, 0.0, theme::background());
    ui.painter().vline(
        sidebar_rect.right() + 0.5,
        full.y_range(),
        Stroke::new(1.0, theme::separator()),
    );
    let mut side = ui.new_child(egui::UiBuilder::new().max_rect(sidebar_rect));
    sidebar(&mut side, state);

    let content = Rect::from_min_max(Pos2::new(sidebar_rect.right() + 1.0, full.top()), full.max);
    let mut page = ui.new_child(egui::UiBuilder::new().max_rect(content));
    let section = state.section;
    if section != SettingsSection::Storage {
        // Coming back to the page reads the sizes again.
        state.storage.leave();
    }
    if section != SettingsSection::PineStudio {
        // Coming back to the page starts at the themes, not in the middle of an edit.
        state.pine_studio.leave();
    }
    match section {
        SettingsSection::Appearance => {
            page_shell(
                &mut page,
                section,
                "Choose how DegenView looks across every window and chart.",
                |ui| {
                    if let Some(theme) =
                        crate::ui::settings_appearance::body(ui, crate::ui::theme::choice())
                    {
                        actions.push(SettingsAction::SetTheme(theme));
                    }
                    ui.add_space(22.0);
                    if let Some(style) =
                        crate::ui::settings_appearance::tabs(ui, input.tab_title_style)
                    {
                        actions.push(SettingsAction::SetTabTitleStyle(style));
                    }
                },
            );
        }
        SettingsSection::PineStudio => {
            crate::ui::settings_pine_studio::page(
                &mut page,
                section,
                &mut state.pine_studio,
                input.editor_themes,
            );
        }
        SettingsSection::Alpaca => {
            let store = AlpacaCredentialsStore::shared();
            page_shell(
                &mut page,
                section,
                "Connect Alpaca to chart US stocks and ETFs in DegenView.",
                |ui| settings_api_keys::alpaca_body(ui, &mut state.api_keys, store),
            );
        }
        SettingsSection::Brokers => {
            page_shell(
                &mut page,
                section,
                "Connect a broker to trade for real from your charts. Paper trading is separate.",
                |ui| {
                    crate::ui::settings_brokers::body(
                        ui,
                        &mut state.brokers,
                        crate::host::trading_hub::TradingHub::global(),
                    )
                },
            );
        }
        SettingsSection::CoinMarketCap => {
            let store = CoinMarketCapKeyStore::shared();
            page_shell(
                &mut page,
                section,
                "Market sentiment charts, with optional higher API rate limits.",
                |ui| settings_api_keys::coinmarketcap_body(ui, &mut state.api_keys, store),
            );
        }
        SettingsSection::Ai => {
            let mut ai_actions = Vec::new();
            page_shell(
                &mut page,
                section,
                "Describe a change and an AI model edits your Pine script, using your own API key.",
                |ui| crate::ui::settings_ai::body(ui, &mut state.ai, &input.ai, &mut ai_actions),
            );
            actions.extend(ai_actions.into_iter().map(SettingsAction::Ai));
        }
        SettingsSection::Notifications => {
            page_shell(
                &mut page,
                section,
                "Choose how DegenView delivers price alerts and check on background delivery.",
                |ui| notifications_page(ui, input, &mut actions),
            );
        }
        SettingsSection::Storage => {
            let mut storage_actions = Vec::new();
            page_shell(
                &mut page,
                section,
                "Logos and lookups saved on this computer so they appear instantly.",
                |ui| settings_storage::body(ui, &mut state.storage, &mut storage_actions),
            );
            actions.extend(storage_actions.into_iter().map(SettingsAction::Storage));
        }
        SettingsSection::Mcp => {
            let mut mcp_actions = Vec::new();
            page_shell(
                &mut page,
                section,
                "Let AI assistants and automation on this Mac inspect DegenView, with permissions you control.",
                |ui| settings_mcp::body(ui, &mut state.mcp, &input.mcp, &mut mcp_actions),
            );
            actions.extend(mcp_actions.into_iter().map(SettingsAction::Mcp));
        }
        SettingsSection::Snapshots => {
            let mut snapshot_actions = Vec::new();
            page_shell(
                &mut page,
                section,
                "Share a picture of a chart, and choose what it shows.",
                |ui| {
                    crate::ui::settings_snapshots::body(
                        ui,
                        &mut state.snapshots,
                        &input.snapshots,
                        &mut snapshot_actions,
                    )
                },
            );
            actions.extend(snapshot_actions.into_iter().map(SettingsAction::Snapshots));
        }
        SettingsSection::About => {
            let about = crate::ui::settings_about::show(&mut page, input.updates);
            if about.quit {
                actions.push(SettingsAction::QuitForUpdate);
            }
            if let Some(channel) = about.channel {
                actions.push(SettingsAction::SetUpdateChannel(channel));
            }
        }
        SettingsSection::Sync => {
            page_shell(
                &mut page,
                section,
                "Keep your watchlists, layouts, drawings, templates, scripts, portfolios and paper trading the same on every Mac.",
                |ui| crate::ui::settings_sync::body(ui, input.sync, input.now_ms),
            );
        }
        SettingsSection::Webhooks => {
            let mut webhook_actions = Vec::new();
            page_shell(
                &mut page,
                section,
                "Send alert messages to external HTTP endpoints, such as a trading bot or a chat channel.",
                |ui| {
                    webhook_actions =
                        webhook_settings::body(ui, &mut state.webhooks, &input.webhooks)
                },
            );
            actions.extend(webhook_actions.into_iter().map(SettingsAction::Webhook));
        }
    }
    actions
}

/// The rows of icon-and-title sections: flat, the selected one lightly filled.
fn sidebar(ui: &mut egui::Ui, state: &mut SettingsState) {
    let area = ui.max_rect();
    let mut y = area.top() + 10.0;
    for section in SettingsSection::visible() {
        let rect = Rect::from_min_size(
            Pos2::new(area.left() + 10.0, y),
            Vec2::new(area.width() - 20.0, 30.0),
        );
        y += 32.0;
        let response = ui
            .interact(
                rect,
                egui::Id::new(("settings-section", section.key())),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, section.title());
        let selected = state.section == section;
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 6.0, theme::selected_tint(false));
        } else if response.hovered() {
            painter.rect_filled(rect, 6.0, kit::primary(0.06));
        }
        // A plain icon, no tile or colour behind it: the selected row says which page is open.
        let icon = Rect::from_center_size(
            Pos2::new(rect.left() + 12.0 + 8.0, rect.center().y),
            Vec2::splat(16.0),
        );
        icons::paint(
            painter,
            icon,
            section.icon(),
            if selected {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        let title = kit::galley(
            ui,
            section.title(),
            kit::size::BODY,
            if selected { kit::MEDIUM } else { kit::REGULAR },
            theme::text_primary(),
            rect.width() - 52.0,
        );
        ui.painter().galley(
            Pos2::new(icon.right() + 12.0, rect.center().y - title.size().y / 2.0),
            title,
            Color32::WHITE,
        );
        if response.clicked() {
            state.section = section;
        }
    }
}

/// One page: the header (the picture, the title and a line about the page) and the scrolling
/// content. A page has no bar under it: no Save button, because a setting is saved as it is
/// changed, and its other actions sit in the content they act on.
pub(super) fn page_shell(
    ui: &mut egui::Ui,
    section: SettingsSection,
    subtitle: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    let body_rect = ui.max_rect();
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
    egui::ScrollArea::vertical()
        .id_salt(("settings-page", section.key()))
        .auto_shrink([false, false])
        .show(&mut body_ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(INSET as i8))
                .show(ui, |ui| {
                    ui.set_width(body_rect.width() - INSET * 2.0 - 8.0);
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
                    page_header(ui, section, subtitle);
                    ui.add_space(18.0);
                    body(ui);
                });
        });
}

/// `SheetHeader`, with the section's picture in place of an icon badge: the picture, the page
/// title and a one-line description.
pub(super) fn page_header(ui: &mut egui::Ui, section: SettingsSection, subtitle: &str) {
    let title = section.title();
    let (row, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), crate::ui::settings_art::SIZE.y),
        Sense::hover(),
    );
    let art = Rect::from_min_size(row.min, crate::ui::settings_art::SIZE);
    crate::ui::settings_art::paint(ui, art, section);
    let left = art.right() + 16.0;
    let width = row.right() - left;
    let title = kit::galley(
        ui,
        title,
        kit::size::TITLE2 + 3.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let detail = kit::galley(
        ui,
        subtitle,
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = row.center().y - (title.size().y + 2.0 + detail.size().y) / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(Pos2::new(left, top), title, theme::text_primary());
    ui.painter().galley(
        Pos2::new(left, top + title_height + 2.0),
        detail,
        theme::text_secondary(),
    );
}

/// A titled group inside a page: heading, optional description, then its content.
pub(super) fn section(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: Option<&str>,
    content: impl FnOnce(&mut egui::Ui),
) {
    ui.label(kit::text(
        title,
        kit::size::HEADLINE,
        kit::BOLD,
        theme::text_primary(),
    ));
    if let Some(subtitle) = subtitle {
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(kit::text(
                subtitle,
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
    }
    ui.add_space(10.0);
    content(ui);
    ui.add_space(18.0);
}

/// The quiet rounded card settings content sits in.
pub(super) fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 12.0;
        content(ui);
    });
}

pub(super) fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, kit::hairline()),
    );
}

impl NotificationSwitch {
    fn icon(self) -> Icon {
        match self {
            Self::Delivery => Icon::BellBadge,
            Self::Sound => Icon::SpeakerWave,
            Self::InAppBanners => Icon::RectTop,
            Self::MacosNotifications => Icon::MacWindow,
        }
    }
}

fn notifications_page(ui: &mut egui::Ui, input: &SettingsInput, actions: &mut Vec<SettingsAction>) {
    section(ui, "Delivery", None, |ui| {
        for (index, switch) in NotificationSwitch::available().enumerate() {
            if index > 0 {
                ui.add_space(10.0);
            }
            settings_card::card(
                ui,
                switch.icon(),
                switch.title(),
                Some(switch.hint()),
                |ui| {
                    let mut value = switch.get(input.notifications);
                    if settings_card::switch(ui, &mut value) {
                        actions.push(SettingsAction::UpdateNotifications(
                            switch.set(input.notifications, value),
                        ));
                    }
                },
            );
        }
    });

    // The background agent, Login Items and the permission prompt are macOS services.
    if !platform::has_system_notifications() {
        return;
    }
    section(
        ui,
        "Background agent",
        Some("Keeps alerts evaluating while DegenView is closed."),
        |ui| {
            let health = input.health;
            card(ui, |ui| {
                row(ui, "Status", |ui| {
                    status_badge(
                        ui,
                        service_label(health.service_state),
                        service_tone(health.service_state),
                    );
                });
                divider(ui);
                row(ui, "Notification permission", |ui| {
                    status_badge(
                        ui,
                        permission_label(health.notification_permission),
                        permission_tone(health.notification_permission),
                    );
                });
                if let Some(heartbeat) = health.heartbeat_ms {
                    divider(ui);
                    row(ui, "Last heartbeat", |ui| {
                        ui.label(kit::text(
                            relative_time(input.now_ms, heartbeat),
                            kit::size::BODY,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ));
                    });
                }
                for provider in &health.providers {
                    divider(ui);
                    row(ui, provider.source.display_name(), |ui| {
                        status_badge(
                            ui,
                            &provider_label(
                                provider.last_error.as_deref(),
                                provider.last_successful_quote_ms.is_some(),
                            ),
                            if provider.last_error.is_none() {
                                Tone::Good
                            } else {
                                Tone::Bad
                            },
                        );
                    });
                }
            });
            if !health.unreconciled_gaps.is_empty() {
                ui.add_space(10.0);
                kit::notice_card(
                    ui,
                    Icon::TriangleFill,
                    kit::ORANGE,
                    &gaps_title(health.unreconciled_gaps.len()),
                    None,
                    None,
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // Registration and Login Items only mean something once an agent is embedded.
                let has_agent = !matches!(
                    health.service_state,
                    AlertServiceState::NotFound | AlertServiceState::Unavailable
                );
                if has_agent {
                    if kit::push_button(ui, None, "Retry Registration", true).clicked() {
                        actions.push(SettingsAction::RetryRegistration);
                    }
                    if kit::push_button(ui, None, "Login Items Settings", true).clicked() {
                        actions.push(SettingsAction::OpenLoginItems);
                    }
                }
                if kit::push_button(ui, None, "Notification Settings", true).clicked() {
                    actions.push(SettingsAction::OpenNotificationSettings);
                }
            });
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(kit::text(
                    "If the background item is disabled or unavailable, alerts keep evaluating while DegenView is open.",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
        },
    );
}

pub(super) fn row(ui: &mut egui::Ui, title: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        ui.label(kit::text(
            title,
            kit::size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_primary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AI page as the user saw it fail: a provider's paragraph as the Test Connection result.
    /// The paragraph shows as a card, and the page has no bar under it, in both themes.
    #[test]
    fn the_ai_page_shows_a_long_error_as_a_card_and_has_no_bar_under_it() {
        use crate::service::keychain_ai_keys::AiKeyStore;
        use crate::service::llm::prefs::AiPrefs;
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::settings_ai::{self, AiSettingsInput};
        use crate::ui::settings_api_keys::StatusMessage;
        use crate::ui::theme::AppTheme;
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        let failure = crate::service::llm::sample::server_error().report();
        let _ = AiKeyStore::shared();
        for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let mut screen = Screen::new(WINDOW_SIZE - Vec2::new(SIDEBAR_WIDTH, 0.0));
            theme::apply(&screen.ctx, app_theme);
            let prefs = AiPrefs::default();
            let mut state = settings_ai::AiSettingsState::default();
            state.status = Some(StatusMessage::failure(failure.clone()));
            let mut actions = Vec::new();
            let mut last = None;
            for _ in 0..3 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let mut page = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
                    page_shell(&mut page, SettingsSection::Ai, "Describe a change and an AI model edits your Pine script, using your own API key.", |ui| {
                        settings_ai::body(ui, &mut state, &AiSettingsInput { prefs: &prefs }, &mut actions)
                    });
                }));
            }
            assert!(
                state.status.is_some(),
                "{name}: nothing dismissed the error"
            );
            if gallery_dir().is_some() {
                screen.save(&format!("settings-ai-{name}"), last.expect("a frame"));
            }
        }
    }

    /// Every page opens with a picture of the same size, so the controls start at the same height,
    /// even where the narrowest page makes the subtitle wrap.
    #[test]
    fn every_page_header_takes_the_pictures_height() {
        use crate::ui::render_tests::Screen;
        for width in [320.0_f32, 600.0] {
            for section in SettingsSection::ALL {
                let mut screen = Screen::new(Vec2::new(width, 200.0));
                let mut height = 0.0;
                screen.run(Vec::new(), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let top = ui.cursor().top();
                    page_header(
                        ui,
                        section,
                        "A long description of what this page is for, which wraps on a narrow page.",
                    );
                    height = ui.cursor().top() - top;
                });
                assert!(
                    (height - crate::ui::settings_art::SIZE.y).abs() < 0.5,
                    "{section:?} at {width}: {height}"
                );
            }
        }
    }

    #[test]
    fn sections_remember_themselves_by_key() {
        for section in SettingsSection::ALL {
            assert_eq!(SettingsSection::from_key(section.key()), Some(section));
        }
        assert_eq!(SettingsSection::from_key("display"), None);
        // Swift opens Settings on Appearance, first in the sidebar.
        assert_eq!(SettingsSection::default(), SettingsSection::Appearance);
        assert_eq!(SettingsSection::ALL[0], SettingsSection::Appearance);
        assert_eq!(SettingsSection::Webhooks.title(), "Webhooks");
        assert_eq!(SettingsSection::Storage.title(), "Storage");
        assert_eq!(
            SettingsSection::from_key("storage"),
            Some(SettingsSection::Storage)
        );
        // Storage sits before About, which stays last.
        assert_eq!(
            SettingsSection::ALL[SettingsSection::ALL.len() - 1],
            SettingsSection::About
        );
        let tints: std::collections::HashSet<_> = SettingsSection::ALL
            .iter()
            .map(|section| section.tint())
            .collect();
        assert_eq!(
            tints.len(),
            SettingsSection::ALL.len(),
            "each badge is distinct"
        );
        assert_ne!(
            SettingsSection::Notifications.tint(),
            SettingsSection::Webhooks.tint()
        );
    }

    #[test]
    fn the_agent_state_reads_in_words_and_tones() {
        use AlertServiceState::*;
        for (state, label, tone) in [
            (Enabled, "Running in background", Tone::Good),
            (
                RequiresApproval,
                "Needs approval in Login Items",
                Tone::Warning,
            ),
            (Disabled, "Foreground only", Tone::Neutral),
            (NotFound, "Agent not embedded", Tone::Bad),
            (Failed, "Registration failed", Tone::Bad),
            (Unavailable, "Unavailable", Tone::Neutral),
        ] {
            assert_eq!(service_label(state), label);
            assert_eq!(service_tone(state), tone);
        }
    }

    #[test]
    fn notification_permission_reads_in_words_and_tones() {
        use AlertNotificationPermission::*;
        for (permission, label, tone) in [
            (Authorized, "Authorized", Tone::Good),
            (Provisional, "Provisional", Tone::Good),
            (Ephemeral, "Ephemeral", Tone::Good),
            (Denied, "Denied", Tone::Bad),
            (NotDetermined, "Not requested", Tone::Neutral),
            (Unknown, "Unknown", Tone::Neutral),
        ] {
            assert_eq!(permission_label(permission), label);
            assert_eq!(permission_tone(permission), tone);
        }
    }

    #[test]
    fn provider_lines_say_why_or_that_all_is_well() {
        assert_eq!(provider_label(Some("Rate limited"), true), "Rate limited");
        assert_eq!(provider_label(None, true), "Healthy");
        assert_eq!(provider_label(None, false), "Waiting for data");
        assert_eq!(gaps_title(1), "1 unreconciled data gap");
        assert_eq!(gaps_title(3), "3 unreconciled data gaps");
    }

    #[test]
    fn each_switch_changes_only_its_own_setting() {
        let base = AlertNotificationSettings::default();
        for switch in NotificationSwitch::ALL {
            assert!(switch.get(&base), "{} starts on", switch.title());
            let off = switch.set(&base, false);
            assert!(!switch.get(&off));
            let others_on = NotificationSwitch::ALL
                .iter()
                .filter(|other| **other != switch)
                .all(|other| other.get(&off));
            assert!(
                others_on,
                "turning {} off touched another switch",
                switch.title()
            );
            assert_eq!(switch.set(&off, true), base);
        }
        assert!(NotificationSwitch::Delivery.hint().contains("armed alerts"));
        assert_eq!(
            NotificationSwitch::MacosNotifications.title(),
            "macOS Notifications"
        );
    }
}
