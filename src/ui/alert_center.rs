//! The Alerts window: every price alert by state, what has fired, and the script alerts.
//! A pure view: it reads rules, history and runtime health and returns
//! `AlertCenterAction`s; the host owns the alert store, the editor sheet and the
//! "seen" bookkeeping (`UnseenAlerts`).

// Transient: the Alerts window hosts this once the alert store is wired.
#![allow(dead_code)]

use crate::model::alert::AlertNotificationPermission;
use crate::model::alert::{
    AlertAsset, AlertCondition, AlertCurrency, AlertDeliveryState, AlertEventOrigin,
    AlertFrequency, AlertId, AlertRuntimeHealth, AlertServiceState, AlertState, AlertTriggerEvent,
    PriceAlert,
};
use crate::model::alert_feed::day_sections;
use crate::ui::icons::{self, Icon};
use crate::ui::script_alert_list::{self, ScriptAlertRow};
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AlertTab {
    #[default]
    Active,
    Triggered,
    Paused,
    All,
    History,
    Scripts,
}

impl AlertTab {
    pub const ALL: [Self; 6] = [
        Self::Active,
        Self::Triggered,
        Self::Paused,
        Self::All,
        Self::History,
        Self::Scripts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Triggered => "Triggered",
            Self::Paused => "Paused",
            Self::All => "All",
            Self::History => "History",
            Self::Scripts => "Scripts",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Active => "🔔",
            Self::Triggered => "✔",
            Self::Paused => "⏸",
            Self::All => "☰",
            Self::History => "🕘",
            Self::Scripts => "{}",
        }
    }

    fn matches(self, alert: &PriceAlert) -> bool {
        match self {
            Self::Active => alert.state == AlertState::Active,
            Self::Triggered => alert.state == AlertState::Triggered,
            Self::Paused => alert.state == AlertState::Paused,
            Self::All => true,
            Self::History | Self::Scripts => false,
        }
    }
}

/// The market price an alert row compares against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuoteSummary {
    pub price: f64,
    pub currency: AlertCurrency,
    pub fresh: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AlertCenterAction {
    Edit(AlertId),
    /// Pause an active alert, resume a paused one, re-enable a triggered one.
    Toggle(AlertId),
    /// Watch an alert on this Mac, or stop. A choice about this Mac only; it never syncs.
    SetEvaluateHere(AlertId, bool),
    Duplicate(AlertId),
    Delete(AlertId),
    ClearHistory,
    OpenNotificationSettings,
    /// Scripts tab: switch a script alert off, back on, or remove it.
    PauseScript(u64),
    RearmScript(u64),
    DeleteScript(u64),
    /// Choose which webhook endpoints a script alert also posts to.
    EditScriptWebhooks(u64),
    OpenScriptManager,
}

pub struct AlertCenterInput<'a> {
    pub load_error: Option<&'a str>,
    pub alerts: &'a [PriceAlert],
    pub history: &'a [AlertTriggerEvent],
    pub health: &'a AlertRuntimeHealth,
    /// By asset key (`AlertAsset::key`).
    pub quote: &'a dyn Fn(&str) -> Option<QuoteSummary>,
    /// `(delivered, total)` webhook results for a trigger, if it posted to any.
    pub webhook_summary: &'a dyn Fn(u64) -> Option<(usize, usize)>,
    pub script_alerts: &'a [ScriptAlertRow],
    /// Alerts defined here but not watched on this Mac (they came from another Mac).
    pub muted: &'a std::collections::HashSet<AlertId>,
    /// iCloud Sync is on, so the per-Mac watch choice is offered.
    pub sync_enabled: bool,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

#[derive(Default)]
pub struct AlertCenterState {
    pub tab: AlertTab,
    pub search: String,
    pending_delete: Option<AlertId>,
    confirming_clear: bool,
}

impl AlertCenterState {
    /// Switches to the Scripts tab and clears any search that would hide it.
    pub fn show_scripts(&mut self) {
        self.search.clear();
        self.tab = AlertTab::Scripts;
    }
}

// MARK: Pure derivations

/// The search matches the symbol, the name and the provider's own id, ignoring case.
pub fn matches_search(asset: &AlertAsset, search: &str) -> bool {
    let needle = search.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    [
        asset.symbol.as_str(),
        asset.name.as_str(),
        asset.instrument.symbol.as_str(),
        asset.source().display_name(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(&needle))
}

/// Rules of a tab matching the search, newest-edited first.
pub fn rules_for<'a>(alerts: &'a [PriceAlert], tab: AlertTab, search: &str) -> Vec<&'a PriceAlert> {
    let mut rules: Vec<&PriceAlert> = alerts
        .iter()
        .filter(|alert| tab.matches(alert) && matches_search(&alert.asset, search))
        .collect();
    rules.sort_by_key(|alert| std::cmp::Reverse(alert.updated_at_ms));
    rules
}

pub fn history_for<'a>(
    history: &'a [AlertTriggerEvent],
    search: &str,
) -> Vec<&'a AlertTriggerEvent> {
    history
        .iter()
        .filter(|event| matches_search(&event.asset, search))
        .collect()
}

/// The count shown on a tab; `None` hides the badge.
pub fn tab_count(tab: AlertTab, input: &AlertCenterInput) -> usize {
    match tab {
        AlertTab::History => input.history.len(),
        AlertTab::Scripts => input.script_alerts.len(),
        _ => input
            .alerts
            .iter()
            .filter(|alert| tab.matches(alert))
            .count(),
    }
}

/// "2 active · 1 triggered", or "No price alerts yet".
pub fn summary(alerts: &[PriceAlert]) -> String {
    let count = |state| alerts.iter().filter(|alert| alert.state == state).count();
    let parts: Vec<String> = [
        (AlertState::Active, "active"),
        (AlertState::Triggered, "triggered"),
        (AlertState::Paused, "paused"),
    ]
    .into_iter()
    .filter_map(|(state, word)| {
        let n = count(state);
        (n > 0).then(|| format!("{n} {word}"))
    })
    .collect();
    if parts.is_empty() {
        "No price alerts yet".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// The newest trigger of each rule, shown on a triggered card.
pub fn last_fired_by_rule(history: &[AlertTriggerEvent]) -> HashMap<AlertId, &AlertTriggerEvent> {
    let mut latest: HashMap<AlertId, &AlertTriggerEvent> = HashMap::new();
    for event in history {
        match latest.get(&event.alert_id) {
            Some(existing) if existing.timestamp_ms >= event.timestamp_ms => {}
            _ => {
                latest.insert(event.alert_id, event);
            }
        }
    }
    latest
}

pub use crate::ui::kit::Tone;

/// `(text, tone, help)` for a rule's status badge.
pub fn status(alert: &PriceAlert, quote_is_fresh: bool) -> (&'static str, Tone, &'static str) {
    match alert.state {
        AlertState::Active if !alert.armed => (
            "Re-arming",
            Tone::Warning,
            "Waiting for the price to move back across the level before it can fire again.",
        ),
        AlertState::Active if !quote_is_fresh => (
            "Waiting for data",
            Tone::Warning,
            "Waiting for current market data.",
        ),
        AlertState::Active => ("Active", Tone::Good, "Watching the market."),
        AlertState::Paused => (
            "Paused",
            Tone::Neutral,
            "Paused. It will not fire until resumed.",
        ),
        AlertState::Triggered => (
            "Triggered",
            Tone::Warning,
            "Fired. Re-enable it to watch again.",
        ),
        AlertState::Unsupported => (
            "Unsupported",
            Tone::Bad,
            "This rule type is not supported by this version.",
        ),
    }
}

/// [`status`], but an alert this Mac is not watching says so instead of claiming to be active.
pub fn status_here(
    alert: &PriceAlert,
    quote_is_fresh: bool,
    muted: bool,
) -> (&'static str, Tone, &'static str) {
    if muted && alert.state == AlertState::Active {
        return (
            "Not watching here",
            Tone::Neutral,
            "This alert came from another Mac and is switched off on this one, so it notifies from one Mac only. Choose Watch on This Mac to turn it on here too.",
        );
    }
    status(alert, quote_is_fresh)
}

pub fn toggle_title(state: AlertState) -> &'static str {
    match state {
        AlertState::Active => "Pause",
        AlertState::Triggered => "Re-enable",
        _ => "Resume",
    }
}

/// The market price, when it is fresh and already in the alert's currency.
pub fn live_price(alert: &PriceAlert, quote: Option<QuoteSummary>) -> Option<f64> {
    quote
        .filter(|quote| quote.fresh && quote.currency == alert.currency)
        .map(|quote| quote.price)
}

/// "0.8% away" for a live rule, "Fired 3 hr. ago" once it has.
pub fn secondary_text(
    alert: &PriceAlert,
    last_fired: Option<&AlertTriggerEvent>,
    live: Option<f64>,
    now_ms: i64,
) -> Option<String> {
    if alert.state == AlertState::Triggered
        && let Some(event) = last_fired
    {
        return Some(format!(
            "Fired {}",
            relative_time(now_ms, event.timestamp_ms)
        ));
    }
    if alert.state != AlertState::Active {
        return None;
    }
    let (price, target) = (live?, alert.condition.target()?);
    if price <= 0.0 {
        return None;
    }
    let away = (target - price).abs() / price * 100.0;
    Some(if away < 10.0 {
        format!("{away:.1}% away")
    } else {
        format!("{away:.0}% away")
    })
}

/// Abbreviated relative time: "now", "5 min. ago", "3 hr. ago", "2 days ago".
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms) / 1_000;
    if secs < 0 {
        return "in the future".to_owned();
    }
    let (value, unit) = match secs {
        0..=4 => return "now".to_owned(),
        5..=59 => (secs, "sec."),
        60..=3_599 => (secs / 60, "min."),
        3_600..=86_399 => (secs / 3_600, "hr."),
        86_400..=604_799 => (
            secs / 86_400,
            if secs / 86_400 == 1 { "day" } else { "days" },
        ),
        604_800..=2_591_999 => (secs / 604_800, "wk."),
        2_592_000..=31_535_999 => (secs / 2_592_000, "mo."),
        _ => (secs / 31_536_000, "yr."),
    };
    format!("{value} {unit} ago")
}

/// "14:03" in the viewer's time zone.
pub fn clock_time(ms: i64, utc_offset_secs: i64) -> String {
    let seconds = (ms.div_euclid(1_000) + utc_offset_secs).rem_euclid(86_400);
    format!("{:02}:{:02}", seconds / 3_600, seconds % 3_600 / 60)
}

/// What an alert waits for as `(glyph, label, bullish)`: "Crosses above", "Falls 5%".
pub fn condition_chip(condition: &AlertCondition) -> (&'static str, String, Option<bool>) {
    let percent = |value: f64| {
        let text = format!("{value:.2}");
        format!("{}%", text.trim_end_matches('0').trim_end_matches('.'))
    };
    match condition {
        AlertCondition::CrossesAbove { .. } => ("↗", "Crosses above".to_owned(), Some(true)),
        AlertCondition::CrossesBelow { .. } => ("↘", "Crosses below".to_owned(), Some(false)),
        AlertCondition::RisesBy { percent: p, .. } => {
            ("↑", format!("Rises {}", percent(*p)), Some(true))
        }
        AlertCondition::FallsBy { percent: p, .. } => {
            ("↓", format!("Falls {}", percent(*p)), Some(false))
        }
        AlertCondition::Unsupported { .. } => ("?", "Unsupported".to_owned(), None),
    }
}

/// A trigger record carries no condition, so a deleted rule's direction is read off the prices.
pub fn crossed_chip(observed: f64, target: f64) -> (&'static str, String, Option<bool>) {
    if observed >= target {
        ("↗", "Crossed above".to_owned(), Some(true))
    } else {
        ("↘", "Crossed below".to_owned(), Some(false))
    }
}

/// The sentence under the history list.
pub fn history_note(count: usize) -> String {
    if count == 0 {
        "Every time an alert fires it is logged here.".to_owned()
    } else {
        format!(
            "{count} {} logged. Clearing history keeps your alerts.",
            if count == 1 { "trigger" } else { "triggers" }
        )
    }
}

pub fn runtime_badge(health: &AlertRuntimeHealth) -> (&'static str, Tone, &'static str) {
    if health.service_state == AlertServiceState::Enabled
        && health.owner == crate::model::alert::AlertRuntimeOwner::Agent
    {
        (
            "Background agent active",
            Tone::Good,
            "Alerts keep being checked when DegenView is closed.",
        )
    } else {
        (
            "Foreground evaluation only",
            Tone::Warning,
            "Alerts are only checked while DegenView is open.",
        )
    }
}

pub fn empty_message(
    tab: AlertTab,
    searching: bool,
    any_in_tab: bool,
) -> (&'static str, &'static str) {
    if searching && any_in_tab {
        return ("No Results", "Check the spelling or try a new search.");
    }
    match tab {
        AlertTab::Active => (
            "No Active Alerts",
            "Click the bell on a chart to alert on a price.",
        ),
        AlertTab::Triggered => (
            "Nothing Triggered",
            "An alert that fires once waits here until you re-enable it.",
        ),
        AlertTab::Paused => (
            "Nothing Paused",
            "Paused alerts wait here until you resume them.",
        ),
        AlertTab::All => (
            "No Alerts",
            "Click the bell on a chart to alert on a price.",
        ),
        AlertTab::History => (
            "No Trigger History",
            "Every time an alert fires it is logged here.",
        ),
        AlertTab::Scripts => (
            "No Script Alerts",
            "Alerts raised by your Pine scripts appear here.",
        ),
    }
}

// MARK: View

pub(super) const GUTTER: f32 = 20.0;
const CARD_HEIGHT: f32 = 58.0;
const ICON: f32 = 34.0;
const SEARCH_ID: &str = "alert-center-search";

impl AlertTab {
    fn icon(self) -> Icon {
        match self {
            Self::Active => Icon::BellBadge,
            Self::Triggered => Icon::CheckCircle,
            Self::Paused => Icon::PauseCircle,
            Self::All => Icon::List,
            Self::History => Icon::ClockArrow,
            Self::Scripts => Icon::Curlybraces,
        }
    }

    fn empty_icon(self) -> Icon {
        match self {
            Self::Active => Icon::BellSlash,
            Self::Triggered => Icon::CheckCircle,
            Self::Paused => Icon::PauseCircle,
            Self::All => Icon::Bell,
            Self::History => Icon::ClockArrow,
            Self::Scripts => Icon::Curlybraces,
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
) -> Vec<AlertCenterAction> {
    let mut actions = Vec::new();
    let full = ui.max_rect();
    ui.ctx()
        .layer_painter(egui::LayerId::background())
        .rect_filled(ui.ctx().content_rect(), 0.0, kit::window());
    ui.spacing_mut().item_spacing = Vec2::ZERO;

    if let Some(error) = input.load_error {
        ui.add_space(24.0);
        padded(ui, |ui| {
            ui.colored_label(theme::ORANGE, format!("Alerts unavailable: {error}"));
        });
        return actions;
    }

    header(ui, state, input, &mut actions);
    ui.painter().hline(
        full.x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, theme::separator()),
    );
    ui.add_space(1.0);

    match state.tab {
        AlertTab::History => history_view(ui, state, input),
        AlertTab::Scripts => {
            script_alert_list::view(ui, &state.search, input.script_alerts, &mut actions);
        }
        tab => rules_view(ui, state, input, tab, &mut actions),
    }

    delete_dialog(ui.ctx(), state, input, &mut actions);
    clear_dialog(ui.ctx(), state, &mut actions);
    actions
}

/// Lays `content` out inside the window's side gutters.
pub(super) fn padded<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(GUTTER as i8, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui)
        })
        .inner
}

fn header(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    actions: &mut Vec<AlertCenterAction>,
) {
    ui.add_space(18.0);
    padded(ui, |ui| {
        let (row, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
        let painter = ui.painter();
        let badge = Rect::from_min_size(row.min, Vec2::splat(44.0));
        painter.rect_filled(badge, 12.0, theme::with_alpha(kit::ACCENT, 0.12));
        icons::paint(
            painter,
            Rect::from_center_size(badge.center(), Vec2::splat(20.0)),
            Icon::BellBadge,
            kit::ACCENT,
        );

        // Right to left: the search field, then the runtime badge.
        let search = Rect::from_min_size(
            Pos2::new(row.right() - 260.0, row.center().y - 13.0),
            Vec2::new(260.0, 26.0),
        );
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(search));
        kit::search_field(
            &mut field,
            Id::new(SEARCH_ID),
            &mut state.search,
            "Search asset or name",
            260.0,
        );
        let (badge_text, tone, help) = runtime_badge(input.health);
        let runtime =
            kit::paint_status_badge(ui, search.left() - 12.0, row.center().y, badge_text, tone);
        let _ = ui
            .interact(runtime, Id::new("alert-runtime-badge"), Sense::hover())
            .on_hover_text(help);

        let left = badge.right() + 14.0;
        let width = runtime.left() - 12.0 - left;
        let title = kit::galley(
            ui,
            "Alerts",
            kit::size::TITLE2 + 1.0,
            kit::SEMIBOLD,
            theme::text_primary(),
            width,
        );
        let detail = kit::galley(
            ui,
            &summary(input.alerts),
            kit::size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
            width,
        );
        let top = row.center().y - (title.size().y + 2.0 + detail.size().y) / 2.0;
        let painter = ui.painter();
        painter.galley(Pos2::new(left, top), title.clone(), theme::text_primary());
        painter.galley(
            Pos2::new(left, top + title.size().y + 2.0),
            detail,
            theme::text_secondary(),
        );

        ui.add_space(14.0);
        let items: Vec<kit::TabItem<AlertTab>> = AlertTab::ALL
            .into_iter()
            .map(|tab| kit::TabItem {
                value: tab,
                title: tab.title(),
                icon: tab.icon(),
                count: tab_count(tab, input),
            })
            .collect();
        kit::tab_bar(ui, Id::new("alert-center-tabs"), &items, &mut state.tab);
    });
    ui.add_space(14.0);

    if input.health.notification_permission == AlertNotificationPermission::Denied {
        padded(ui, |ui| {
            if kit::notice_card(
                ui,
                Icon::BellSlash,
                kit::ORANGE,
                "Notifications are turned off",
                Some(
                    "Alerts still fire and are logged here, but macOS will not show them. \
                     Allow DegenView in System Settings.",
                ),
                Some("Open Settings"),
            ) {
                actions.push(AlertCenterAction::OpenNotificationSettings);
            }
        });
        ui.add_space(14.0);
    }
}

/// The empty or no-results page of a list, centred in what is left of the window.
fn empty_page(ui: &mut egui::Ui, tab: AlertTab, search: &str, any_in_tab: bool, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let query = search.trim();
    if !query.is_empty() && any_in_tab {
        kit::empty_state(
            ui,
            rect,
            Icon::Magnifier,
            &format!("No Results for “{query}”"),
            "Check the spelling or try a new search.",
        );
    } else {
        let (title, detail) = empty_message(tab, false, any_in_tab);
        kit::empty_state(ui, rect, tab.empty_icon(), title, detail);
    }
}

/// The scrolling stack every tab lays its cards in: even gutters and spaced cards.
pub(super) fn card_scroll(ui: &mut egui::Ui, height: f32, content: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(height)
        .show(ui, |ui| {
            padded(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.add_space(8.0);
                content(ui);
                ui.add_space(16.0);
            });
        });
}

/// A day's heading in a history list ("Today", "Monday, 29 September") with how many fired.
pub(super) fn day_header(ui: &mut egui::Ui, title: &str, count: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let y = rect.bottom() - 11.0;
    let title = kit::galley(
        ui,
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_secondary(),
        rect.width() - 50.0,
    );
    let right = rect.left() + title.size().x;
    ui.painter().galley(
        Pos2::new(rect.left(), y - title.size().y / 2.0),
        title,
        theme::text_secondary(),
    );
    kit::paint_count(
        ui,
        right + 6.0,
        y,
        count,
        theme::text_secondary(),
        theme::quaternary(1.0),
    );
}

/// The card chrome shared by every alerts row; returns the card and its response.
pub(super) fn card(ui: &mut egui::Ui, id: Id) -> (Rect, egui::Response) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), CARD_HEIGHT), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    ui.painter()
        .rect_filled(rect, 10.0, kit::primary(if hovered { 0.07 } else { 0.035 }));
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, kit::hairline()),
        egui::StrokeKind::Inside,
    );
    (rect, response)
}

/// The icon slot at the left of a card, leaving room for the source badge.
pub(super) fn icon_rect(card: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(card.left() + 14.0, card.center().y - ICON / 2.0 - 1.5),
        Vec2::splat(ICON),
    )
}

/// The headline and caption of a card, from `left` up to `right`; `extras` paints after the
/// headline (frequency and webhook marks, the delayed capsule) and returns how wide it was.
pub(super) fn identity(
    ui: &egui::Ui,
    card: Rect,
    left: f32,
    right: f32,
    headline: &str,
    caption: &str,
    extras: impl FnOnce(&egui::Ui, Pos2, f32) -> f32,
) {
    let width = (right - left).max(40.0);
    let title = kit::galley(
        ui,
        headline,
        kit::size::HEADLINE,
        kit::BOLD,
        theme::text_primary(),
        width - 60.0,
    );
    let detail = kit::galley(
        ui,
        caption,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = card.center().y - (title.size().y + 2.0 + detail.size().y) / 2.0;
    let title_size = title.size();
    ui.painter()
        .galley(Pos2::new(left, top), title, theme::text_primary());
    extras(
        ui,
        Pos2::new(left + title_size.x + 6.0, top + title_size.y / 2.0),
        width - title_size.x - 6.0,
    );
    ui.painter().galley(
        Pos2::new(left, top + title_size.y + 2.0),
        detail,
        theme::text_secondary(),
    );
}

/// A small secondary glyph after a card's headline, with a tooltip.
fn mark(ui: &egui::Ui, at: Pos2, icon: Icon, id: Id, tip: &str) -> f32 {
    let rect = Rect::from_min_size(Pos2::new(at.x, at.y - 5.5), Vec2::splat(11.0));
    icons::paint(ui.painter(), rect, icon, theme::text_secondary());
    let _ = ui
        .interact(rect.expand(3.0), id, Sense::hover())
        .on_hover_text(tip);
    11.0 + 6.0
}

/// A two-line, right-aligned column: a semibold value over a caption.
pub(super) fn value_column(
    ui: &egui::Ui,
    right: f32,
    center_y: f32,
    value: &str,
    caption: Option<&str>,
    weight: f32,
) {
    let value = kit::digits(
        ui,
        value,
        kit::size::BODY,
        weight,
        theme::text_primary(),
        132.0,
    );
    let caption = caption.map(|caption| {
        kit::galley(
            ui,
            caption,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
            132.0,
        )
    });
    let total = value.size().y + caption.as_ref().map_or(0.0, |c| 2.0 + c.size().y);
    let top = center_y - total / 2.0;
    let value_height = value.size().y;
    ui.painter().galley(
        Pos2::new(right - value.size().x, top),
        value,
        theme::text_primary(),
    );
    if let Some(caption) = caption {
        ui.painter().galley(
            Pos2::new(right - caption.size().x, top + value_height + 2.0),
            caption,
            theme::text_secondary(),
        );
    }
}

/// The always-visible "More actions" button at the right of a card; its menu is `items`.
pub(super) fn more_button(
    ui: &mut egui::Ui,
    rect: Rect,
    id: Id,
    items: impl FnOnce(&mut egui::Ui),
) {
    let response = kit::hover_button(ui, rect, id, Icon::Ellipsis, "More actions", true, true);
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .show(|ui| {
            ui.set_width(180.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            items(ui);
        });
}

fn condition_chip_parts(chip: &(&'static str, String, Option<bool>)) -> (Icon, Color32) {
    let icon = match chip.0 {
        "↗" => Icon::ArrowUpRight,
        "↘" => Icon::ArrowDownRight,
        "↑" => Icon::ArrowUp,
        "↓" => Icon::ArrowDown,
        _ => Icon::CircleDashed,
    };
    let tint = match chip.2 {
        Some(true) => kit::GREEN,
        Some(false) => kit::RED,
        None => theme::text_secondary(),
    };
    (icon, tint)
}

fn rules_view(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    tab: AlertTab,
    actions: &mut Vec<AlertCenterAction>,
) {
    let rules = rules_for(input.alerts, tab, &state.search);
    let height = ui.available_height();
    if rules.is_empty() {
        let any_in_tab = input.alerts.iter().any(|alert| tab.matches(alert));
        empty_page(ui, tab, &state.search, any_in_tab, height);
        return;
    }
    let fired = last_fired_by_rule(input.history);
    card_scroll(ui, height, |ui| {
        for alert in rules {
            rule_row(
                ui,
                state,
                input,
                alert,
                fired.get(&alert.id).copied(),
                actions,
            );
        }
    });
}

fn rule_menu(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    alert: &PriceAlert,
    watch: Option<bool>,
    actions: &mut Vec<AlertCenterAction>,
) {
    let toggle_icon = if alert.state == AlertState::Active {
        Icon::PauseFill
    } else {
        Icon::PlayFill
    };
    if kit::menu_item(ui, Icon::Pencil, "Edit…", true, false) {
        actions.push(AlertCenterAction::Edit(alert.id));
        ui.close();
    }
    if kit::menu_item(
        ui,
        toggle_icon,
        toggle_title(alert.state),
        alert.state != AlertState::Unsupported,
        false,
    ) {
        actions.push(AlertCenterAction::Toggle(alert.id));
        ui.close();
    }
    // `watch` is `Some(currently watched here)` while iCloud Sync is on.
    if let Some(watched) = watch {
        let (icon, title) = if watched {
            (Icon::EyeSlash, "Stop Watching on This Mac")
        } else {
            (Icon::Eye, "Watch on This Mac")
        };
        if kit::menu_item(
            ui,
            icon,
            title,
            alert.state != AlertState::Unsupported,
            false,
        ) {
            actions.push(AlertCenterAction::SetEvaluateHere(alert.id, !watched));
            ui.close();
        }
    }
    if kit::menu_item(ui, Icon::CopyDoc, "Duplicate", true, false) {
        actions.push(AlertCenterAction::Duplicate(alert.id));
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Delete…", true, true) {
        state.pending_delete = Some(alert.id);
        ui.close();
    }
}

fn rule_row(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    alert: &PriceAlert,
    last_fired: Option<&AlertTriggerEvent>,
    actions: &mut Vec<AlertCenterAction>,
) {
    let (rect, response) = card(ui, Id::new(("alert-card", alert.id)));
    let hovered = ui.rect_contains_pointer(rect);
    let quote = (input.quote)(&alert.asset.key());
    let live = live_price(alert, quote);
    let (status_text, tone, status_help) = status_here(
        alert,
        quote.is_some_and(|q| q.fresh),
        input.muted.contains(&alert.id),
    );
    let center_y = rect.center().y;

    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    if alert.state == AlertState::Paused {
        row.multiply_opacity(0.7);
    }
    let ui = &mut row;

    let icon = icon_rect(rect);
    kit::ticker_icon(ui, icon, &alert.asset.symbol, alert.asset.source());
    kit::source_badge(ui, icon, alert.asset.source());

    // Right to left: actions, status, target, condition.
    let mut right = rect.right() - 14.0;
    let more = Rect::from_min_size(Pos2::new(right - 26.0, center_y - 13.0), Vec2::splat(26.0));
    let watch = input.sync_enabled.then(|| !input.muted.contains(&alert.id));
    more_button(ui, more, Id::new(("alert-more", alert.id)), |ui| {
        rule_menu(ui, state, alert, watch, actions);
    });
    let toggle = more.translate(Vec2::new(-28.0, 0.0));
    let toggle_icon = if alert.state == AlertState::Active {
        Icon::PauseFill
    } else {
        Icon::PlayFill
    };
    if kit::hover_button(
        ui,
        toggle,
        Id::new(("alert-toggle", alert.id)),
        toggle_icon,
        toggle_title(alert.state),
        hovered,
        alert.state != AlertState::Unsupported,
    )
    .clicked()
    {
        actions.push(AlertCenterAction::Toggle(alert.id));
    }
    let edit = toggle.translate(Vec2::new(-28.0, 0.0));
    if kit::hover_button(
        ui,
        edit,
        Id::new(("alert-edit", alert.id)),
        Icon::Pencil,
        "Edit",
        hovered,
        true,
    )
    .clicked()
    {
        actions.push(AlertCenterAction::Edit(alert.id));
    }
    right = edit.left() - 12.0;

    let badge = kit::paint_status_badge(ui, right, center_y, status_text, tone);
    let _ = ui
        .interact(badge, Id::new(("alert-status", alert.id)), Sense::hover())
        .on_hover_text(status_help);
    right -= 136.0 + 12.0;

    let target = alert.condition.target().map_or_else(
        || "—".to_owned(),
        |target| alert.currency.format_price(target),
    );
    let secondary = secondary_text(alert, last_fired, live, input.now_ms);
    value_column(
        ui,
        right,
        center_y,
        &target,
        secondary.as_deref(),
        kit::SEMIBOLD,
    );
    if let Some(price) = live {
        let column = Rect::from_min_max(
            Pos2::new(right - 132.0, rect.top()),
            Pos2::new(right, rect.bottom()),
        );
        let _ = ui
            .interact(column, Id::new(("alert-target", alert.id)), Sense::hover())
            .on_hover_text(format!("Now {}", alert.currency.format_price(price)));
    }
    right -= 132.0 + 12.0;

    let chip = condition_chip(&alert.condition);
    let (chip_icon, tint) = condition_chip_parts(&chip);
    let chip_rect = kit::paint_chip(ui, right, center_y, chip_icon, &chip.1, tint);

    let caption = if alert.note.is_empty() {
        format!(
            "{} · {}",
            alert.asset.name,
            alert.asset.source().display_name()
        )
    } else {
        alert.note.clone()
    };
    let id = alert.id;
    identity(
        ui,
        rect,
        icon.right() + 3.0 + 12.0,
        chip_rect.left() - 8.0,
        &alert.asset.symbol,
        &caption,
        |ui, at, _| {
            let (frequency, tip) = if alert.frequency == AlertFrequency::Once {
                (Icon::OneCircle, "Triggers once")
            } else {
                (Icon::Repeat, "Triggers every time")
            };
            let mut width = mark(ui, at, frequency, Id::new(("alert-frequency", id)), tip);
            if !alert.webhook_endpoint_ids.is_empty() {
                let count = alert.webhook_endpoint_ids.len();
                width += mark(
                    ui,
                    at + Vec2::new(width, 0.0),
                    Icon::ArrowUpRightSquare,
                    Id::new(("alert-webhooks", id)),
                    &format!(
                        "Posts to {count} webhook{}",
                        if count == 1 { "" } else { "s" }
                    ),
                );
            }
            width
        },
    );

    if response.double_clicked() {
        actions.push(AlertCenterAction::Edit(alert.id));
    }
    let spoken = format!(
        "{} alert: {} {}, {}",
        alert.asset.symbol, chip.1, target, status_text,
    );
    let response = accessibility::label(response, spoken);
    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            ui.set_width(180.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let watch = input.sync_enabled.then(|| !input.muted.contains(&alert.id));
            rule_menu(ui, state, alert, watch, actions);
        });
}

fn history_view(ui: &mut egui::Ui, state: &mut AlertCenterState, input: &AlertCenterInput) {
    let events = history_for(input.history, &state.search);
    let footer_height = 45.0;
    let body_height = (ui.available_height() - footer_height).max(60.0);
    if events.is_empty() {
        empty_page(
            ui,
            AlertTab::History,
            &state.search,
            !input.history.is_empty(),
            body_height,
        );
    } else {
        let rules: HashMap<AlertId, &PriceAlert> =
            input.alerts.iter().map(|alert| (alert.id, alert)).collect();
        let sections = day_sections(
            events,
            |event| event.timestamp_ms,
            input.now_ms,
            input.utc_offset_secs,
        );
        ui.allocate_ui(Vec2::new(ui.available_width(), body_height), |ui| {
            card_scroll(ui, body_height, |ui| {
                for section in sections {
                    day_header(ui, &section.title, section.items.len());
                    for event in section.items {
                        history_row(ui, input, event, rules.get(&event.alert_id).copied());
                    }
                }
            });
        });
    }
    if kit::footer_bar(
        ui,
        Icon::ClockArrow,
        &history_note(input.history.len()),
        Icon::Trash,
        "Clear History",
        !input.history.is_empty(),
        "Remove every logged trigger, not just the ones matching the search",
    ) {
        state.confirming_clear = true;
    }
}

/// "Webhooks 2/3 delivered" and its tone: green when all landed, red when none did.
pub fn webhook_chip(delivered: usize, total: usize) -> (String, Tone) {
    let tone = if delivered == total {
        Tone::Good
    } else if delivered == 0 {
        Tone::Bad
    } else {
        Tone::Warning
    };
    (format!("Webhooks {delivered}/{total} delivered"), tone)
}

fn history_row(
    ui: &mut egui::Ui,
    input: &AlertCenterInput,
    event: &AlertTriggerEvent,
    rule: Option<&PriceAlert>,
) {
    let (rect, _) = card(ui, Id::new(("alert-event", event.id)));
    let center_y = rect.center().y;
    let icon = icon_rect(rect);
    kit::ticker_icon(ui, icon, &event.asset.symbol, event.asset.source());
    kit::source_badge(ui, icon, event.asset.source());

    // Right to left: time, price, condition, webhook and delivery marks.
    let mut right = rect.right() - 14.0;
    value_column(
        ui,
        right,
        center_y,
        &clock_time(event.timestamp_ms, input.utc_offset_secs),
        Some(&relative_time(input.now_ms, event.timestamp_ms)),
        kit::REGULAR,
    );
    right -= 92.0 + 12.0;
    value_column(
        ui,
        right,
        center_y,
        &event.currency.format_price(event.observed_value),
        Some(&format!(
            "Target {}",
            event.currency.format_price(event.target)
        )),
        kit::SEMIBOLD,
    );
    right -= 132.0 + 12.0;

    let chip = match rule {
        Some(rule) => condition_chip(&rule.condition),
        None => crossed_chip(event.observed_value, event.target),
    };
    let (chip_icon, tint) = condition_chip_parts(&chip);
    let mut left = kit::paint_chip(ui, right, center_y, chip_icon, &chip.1, tint).left();

    if let Some((delivered, total)) = (input.webhook_summary)(event.id) {
        let (label, tone) = webhook_chip(delivered, total);
        left = kit::paint_chip(
            ui,
            left - 8.0,
            center_y,
            Icon::ArrowUpRightSquare,
            &label,
            tone.color(),
        )
        .left();
    }
    if event.delivery.state == AlertDeliveryState::Failed {
        let warning = Rect::from_min_size(
            Pos2::new(left - 8.0 - 14.0, center_y - 7.0),
            Vec2::splat(14.0),
        );
        icons::paint(ui.painter(), warning, Icon::TriangleFill, kit::RED);
        let _ = ui
            .interact(
                warning,
                Id::new(("alert-event-failed", event.id)),
                Sense::hover(),
            )
            .on_hover_text(
                event
                    .delivery
                    .error
                    .as_deref()
                    .unwrap_or("The notification could not be delivered."),
            );
        left = warning.left();
    }

    let caption = format!(
        "{} · {}",
        event.asset.name,
        event.asset.source().display_name()
    );
    let event_id = event.id;
    identity(
        ui,
        rect,
        icon.right() + 3.0 + 12.0,
        left - 8.0,
        &event.asset.symbol,
        &caption,
        |ui, at, _| {
            if event.origin != AlertEventOrigin::CatchUp {
                return 0.0;
            }
            let width = kit::chip_size(ui, "Delayed").x;
            let placed = kit::paint_chip(
                ui,
                at.x + width,
                at.y,
                Icon::ClockBadge,
                "Delayed",
                kit::ORANGE,
            );
            let _ = ui
                .interact(
                    placed,
                    Id::new(("alert-event-delayed", event_id)),
                    Sense::hover(),
                )
                .on_hover_text("Seen late: the price crossed while DegenView was not watching.");
            width + 6.0
        },
    );
}

fn delete_dialog(
    ctx: &egui::Context,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    actions: &mut Vec<AlertCenterAction>,
) {
    let Some(id) = state.pending_delete else {
        return;
    };
    let Some(alert) = input.alerts.iter().find(|alert| alert.id == id) else {
        state.pending_delete = None;
        return;
    };
    let message = format!(
        "The {} alert is removed. Its trigger history is kept.",
        alert.asset.symbol
    );
    match kit::confirm(
        ctx,
        Id::new("alert-delete"),
        &kit::Confirm {
            icon: Icon::Trash,
            title: "Delete this alert?",
            message: &message,
            destructive: "Delete",
        },
    ) {
        Some(kit::ConfirmOutcome::Confirmed) => {
            actions.push(AlertCenterAction::Delete(id));
            state.pending_delete = None;
        }
        Some(kit::ConfirmOutcome::Cancelled) => state.pending_delete = None,
        None => {}
    }
}

fn clear_dialog(
    ctx: &egui::Context,
    state: &mut AlertCenterState,
    actions: &mut Vec<AlertCenterAction>,
) {
    if !state.confirming_clear {
        return;
    }
    match kit::confirm(
        ctx,
        Id::new("alert-clear"),
        &kit::Confirm {
            icon: Icon::Trash,
            title: "Clear all trigger history?",
            message: "This can't be undone. Your alerts are kept.",
            destructive: "Clear History",
        },
    ) {
        Some(kit::ConfirmOutcome::Confirmed) => {
            actions.push(AlertCenterAction::ClearHistory);
            state.confirming_clear = false;
        }
        Some(kit::ConfirmOutcome::Cancelled) => state.confirming_clear = false,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{AlertDeliveryRecord, AlertNotificationPermission};
    use crate::model::{DataSource, InstrumentId};

    fn asset(symbol: &str, name: &str) -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, format!("{symbol}USDT")),
            symbol,
            name,
        )
    }

    fn alert(symbol: &str, name: &str, state: AlertState, updated: i64) -> PriceAlert {
        let mut alert = PriceAlert::new(
            asset(symbol, name),
            AlertCondition::CrossesAbove { target: 100.0 },
            0,
        );
        alert.state = state;
        alert.updated_at_ms = updated;
        alert
    }

    fn event(alert: &PriceAlert, at: i64) -> AlertTriggerEvent {
        AlertTriggerEvent {
            id: at as u64,
            alert_id: alert.id,
            asset: alert.asset.clone(),
            observed_value: 101.0,
            target: 100.0,
            currency: AlertCurrency::Usd,
            timestamp_ms: at,
            quote_fingerprint: "f".into(),
            origin: AlertEventOrigin::Live,
            delivery: AlertDeliveryRecord::default(),
            candle: None,
        }
    }

    static NO_MUTED: std::sync::LazyLock<std::collections::HashSet<AlertId>> =
        std::sync::LazyLock::new(Default::default);

    fn input<'a>(
        alerts: &'a [PriceAlert],
        history: &'a [AlertTriggerEvent],
        health: &'a AlertRuntimeHealth,
    ) -> AlertCenterInput<'a> {
        AlertCenterInput {
            load_error: None,
            alerts,
            history,
            health,
            quote: &|_| None,
            webhook_summary: &|_| None,
            script_alerts: &[],
            muted: &NO_MUTED,
            sync_enabled: false,
            now_ms: 10_000_000,
            utc_offset_secs: 0,
        }
    }

    #[test]
    fn a_muted_alert_says_it_is_not_watching_here_and_other_states_are_unchanged() {
        let active = alert("BTC", "Bitcoin", AlertState::Active, 0);
        assert_eq!(status_here(&active, true, true).0, "Not watching here");
        assert_eq!(status_here(&active, true, false), status(&active, true));
        let paused = alert("BTC", "Bitcoin", AlertState::Paused, 0);
        assert_eq!(status_here(&paused, true, true), status(&paused, true));
        assert!(status_here(&active, true, true).2.contains("another Mac"));
    }

    #[test]
    fn tabs_filter_by_state_and_sort_newest_edit_first() {
        let alerts = vec![
            alert("BTC", "Bitcoin", AlertState::Active, 10),
            alert("ETH", "Ethereum", AlertState::Active, 30),
            alert("SOL", "Solana", AlertState::Paused, 20),
            alert("XRP", "Ripple", AlertState::Triggered, 5),
        ];
        let symbols = |tab| {
            rules_for(&alerts, tab, "")
                .iter()
                .map(|alert| alert.asset.symbol.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(symbols(AlertTab::Active), ["ETH", "BTC"]);
        assert_eq!(symbols(AlertTab::Paused), ["SOL"]);
        assert_eq!(symbols(AlertTab::Triggered), ["XRP"]);
        assert_eq!(symbols(AlertTab::All), ["ETH", "SOL", "BTC", "XRP"]);
        assert!(symbols(AlertTab::History).is_empty());
        assert!(symbols(AlertTab::Scripts).is_empty());
    }

    #[test]
    fn search_matches_symbol_name_provider_id_and_source() {
        let a = asset("BTC", "Bitcoin");
        for hit in ["btc", "BIT", " coin ", "btcusdt", "binance", ""] {
            assert!(matches_search(&a, hit), "{hit:?}");
        }
        for miss in ["eth", "coinbase", "xyz"] {
            assert!(!matches_search(&a, miss), "{miss:?}");
        }
    }

    #[test]
    fn counts_and_summary_follow_the_states() {
        let alerts = vec![
            alert("BTC", "Bitcoin", AlertState::Active, 0),
            alert("ETH", "Ethereum", AlertState::Active, 0),
            alert("SOL", "Solana", AlertState::Triggered, 0),
        ];
        let history = vec![event(&alerts[0], 1), event(&alerts[2], 2)];
        let health = AlertRuntimeHealth::default();
        let input = input(&alerts, &history, &health);
        assert_eq!(tab_count(AlertTab::Active, &input), 2);
        assert_eq!(tab_count(AlertTab::Triggered, &input), 1);
        assert_eq!(tab_count(AlertTab::Paused, &input), 0);
        assert_eq!(tab_count(AlertTab::All, &input), 3);
        assert_eq!(tab_count(AlertTab::History, &input), 2);
        assert_eq!(tab_count(AlertTab::Scripts, &input), 0);
        assert_eq!(summary(&alerts), "2 active · 1 triggered");
        assert_eq!(summary(&[]), "No price alerts yet");
    }

    #[test]
    fn the_newest_trigger_of_each_rule_wins() {
        let a = alert("BTC", "Bitcoin", AlertState::Triggered, 0);
        let b = alert("ETH", "Ethereum", AlertState::Triggered, 0);
        let history = vec![event(&a, 5), event(&a, 9), event(&b, 7), event(&a, 3)];
        let latest = last_fired_by_rule(&history);
        assert_eq!(latest[&a.id].timestamp_ms, 9);
        assert_eq!(latest[&b.id].timestamp_ms, 7);
    }

    #[test]
    fn status_badges_explain_why_a_rule_is_not_yet_firing() {
        let mut a = alert("BTC", "Bitcoin", AlertState::Active, 0);
        assert_eq!(status(&a, true).0, "Active");
        assert_eq!(
            status(&a, false),
            (
                "Waiting for data",
                Tone::Warning,
                "Waiting for current market data."
            )
        );
        a.armed = false;
        assert_eq!(status(&a, true).0, "Re-arming");
        a.state = AlertState::Paused;
        assert_eq!(status(&a, true).1, Tone::Neutral);
        a.state = AlertState::Triggered;
        assert_eq!(status(&a, true).0, "Triggered");
        a.state = AlertState::Unsupported;
        assert_eq!(status(&a, true).1, Tone::Bad);
        assert_eq!(toggle_title(AlertState::Active), "Pause");
        assert_eq!(toggle_title(AlertState::Triggered), "Re-enable");
        assert_eq!(toggle_title(AlertState::Paused), "Resume");
    }

    #[test]
    fn the_live_price_is_used_only_when_fresh_and_in_the_alerts_currency() {
        let a = alert("BTC", "Bitcoin", AlertState::Active, 0);
        let quote = QuoteSummary {
            price: 95.0,
            currency: AlertCurrency::Usd,
            fresh: true,
        };
        assert_eq!(live_price(&a, Some(quote)), Some(95.0));
        assert_eq!(
            live_price(
                &a,
                Some(QuoteSummary {
                    fresh: false,
                    ..quote
                })
            ),
            None
        );
        assert_eq!(
            live_price(
                &a,
                Some(QuoteSummary {
                    currency: AlertCurrency::Eur,
                    ..quote
                })
            ),
            None
        );
        assert_eq!(live_price(&a, None), None);
    }

    #[test]
    fn the_secondary_line_shows_distance_or_when_it_fired() {
        let active = alert("BTC", "Bitcoin", AlertState::Active, 0);
        assert_eq!(
            secondary_text(&active, None, Some(95.0), 0).as_deref(),
            Some("5.3% away")
        );
        assert_eq!(
            secondary_text(&active, None, Some(50.0), 0).as_deref(),
            Some("100% away")
        );
        assert_eq!(secondary_text(&active, None, None, 0), None);
        assert_eq!(secondary_text(&active, None, Some(0.0), 0), None);

        let mut triggered = alert("BTC", "Bitcoin", AlertState::Triggered, 0);
        let fired = event(&triggered, 1_000_000);
        assert_eq!(
            secondary_text(&triggered, Some(&fired), None, 1_000_000 + 3 * 3_600_000).as_deref(),
            Some("Fired 3 hr. ago")
        );
        triggered.state = AlertState::Paused;
        assert_eq!(
            secondary_text(&triggered, Some(&fired), Some(95.0), 0),
            None
        );
    }

    #[test]
    fn relative_times_are_abbreviated() {
        let now = 100_000_000_000;
        for (ago_secs, expected) in [
            (0, "now"),
            (4, "now"),
            (30, "30 sec. ago"),
            (120, "2 min. ago"),
            (3 * 3_600, "3 hr. ago"),
            (86_400, "1 day ago"),
            (3 * 86_400, "3 days ago"),
            (14 * 86_400, "2 wk. ago"),
            (90 * 86_400, "3 mo. ago"),
            (800 * 86_400, "2 yr. ago"),
        ] {
            assert_eq!(
                relative_time(now, now - ago_secs * 1_000),
                expected,
                "{ago_secs}"
            );
        }
        assert_eq!(relative_time(now, now + 5_000), "in the future");
    }

    #[test]
    fn clock_times_follow_the_viewers_zone() {
        // 2024-01-15 15:00 UTC.
        assert_eq!(clock_time(1_705_330_800_000, 0), "15:00");
        assert_eq!(clock_time(1_705_330_800_000, 2 * 3_600), "17:00");
        assert_eq!(clock_time(1_705_330_800_000, -8 * 3_600), "07:00");
        assert_eq!(clock_time(1_705_330_800_000 + 90_000, 0), "15:01");
    }

    #[test]
    fn condition_chips_name_the_direction() {
        assert_eq!(
            condition_chip(&AlertCondition::CrossesAbove { target: 1.0 }).1,
            "Crosses above"
        );
        assert_eq!(
            condition_chip(&AlertCondition::CrossesBelow { target: 1.0 }).2,
            Some(false)
        );
        assert_eq!(
            condition_chip(&AlertCondition::rises_by(5.0, 100.0)).1,
            "Rises 5%"
        );
        assert_eq!(
            condition_chip(&AlertCondition::falls_by(2.5, 100.0)).1,
            "Falls 2.5%"
        );
        assert_eq!(
            condition_chip(&AlertCondition::Unsupported { tag: "x".into() }).2,
            None
        );
        assert_eq!(crossed_chip(101.0, 100.0).1, "Crossed above");
        assert_eq!(crossed_chip(100.0, 100.0).1, "Crossed above");
        assert_eq!(crossed_chip(99.0, 100.0).1, "Crossed below");
    }

    #[test]
    fn notes_badges_and_empty_states_read_right() {
        assert_eq!(
            history_note(0),
            "Every time an alert fires it is logged here."
        );
        assert_eq!(
            history_note(1),
            "1 trigger logged. Clearing history keeps your alerts."
        );
        assert_eq!(
            history_note(7),
            "7 triggers logged. Clearing history keeps your alerts."
        );

        let mut health = AlertRuntimeHealth::default();
        assert_eq!(runtime_badge(&health).0, "Foreground evaluation only");
        health.service_state = AlertServiceState::Enabled;
        health.owner = crate::model::alert::AlertRuntimeOwner::Agent;
        assert_eq!(runtime_badge(&health).1, Tone::Good);

        assert_eq!(empty_message(AlertTab::Active, true, true).0, "No Results");
        assert_eq!(
            empty_message(AlertTab::Active, true, false).0,
            "No Active Alerts"
        );
        assert_eq!(
            empty_message(AlertTab::History, false, false).0,
            "No Trigger History"
        );
        assert_eq!(
            empty_message(AlertTab::Paused, false, false).0,
            "Nothing Paused"
        );
        let denied = AlertRuntimeHealth {
            notification_permission: AlertNotificationPermission::Denied,
            ..Default::default()
        };
        assert_eq!(
            denied.notification_permission,
            AlertNotificationPermission::Denied
        );
    }

    #[test]
    fn history_search_filters_by_asset_and_keeps_order() {
        let a = alert("BTC", "Bitcoin", AlertState::Active, 0);
        let b = alert("ETH", "Ethereum", AlertState::Active, 0);
        let history = vec![event(&a, 1), event(&b, 2), event(&a, 3)];
        let found = history_for(&history, "eth");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].asset.symbol, "ETH");
        assert_eq!(history_for(&history, "").len(), 3);
    }

    #[test]
    fn showing_scripts_clears_a_search_that_would_hide_them() {
        let mut state = AlertCenterState {
            search: "btc".into(),
            ..AlertCenterState::default()
        };
        state.show_scripts();
        assert!(state.search.is_empty());
        assert_eq!(state.tab, AlertTab::Scripts);
    }
}
