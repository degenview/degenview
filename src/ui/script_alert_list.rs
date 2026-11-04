//! The Alerts window's Scripts tab: subscriptions to Pine `alert()` calls, each with its status and
//! Pause / Re-arm / Delete. A port of `PineAlertListView` (the "Fired" history half needs a stored
//! script-notification log the Rust app does not keep yet).

use super::alert_center::{self, AlertCenterAction, Tone};
use crate::model::DataSource;
use crate::model::pine_alert::{PineAlertState, PineAlertSubscription};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Id, Pos2, Rect, Sense, Vec2};

/// One script alert as the list shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptAlertRow {
    pub id: u64,
    pub script_name: String,
    /// The ticker part of the symbol key.
    pub symbol: String,
    pub source: Option<DataSource>,
    pub timeframe: String,
    pub note: String,
    pub webhook_count: usize,
    pub status: &'static str,
    pub tone: Tone,
    pub active: bool,
    pub can_rearm: bool,
    pub created_at_ms: i64,
}

/// What a script alert's badge says: whether it can fire now, and if not, why. `dataset` is the
/// `(symbol_key, timeframe)` of the chart the alert was created on, when that chart is open.
pub fn status_of(
    subscription: &PineAlertSubscription,
    dataset: Option<(&str, &str)>,
) -> (&'static str, Tone) {
    match subscription.state {
        PineAlertState::Paused => ("Paused", Tone::Neutral),
        PineAlertState::ScriptChanged => ("Script changed — re-arm", Tone::Warning),
        PineAlertState::ScriptDeleted => ("Script deleted", Tone::Bad),
        PineAlertState::Active => match dataset {
            None => ("Chart not open", Tone::Neutral),
            Some((key, timeframe)) if subscription.watches(key, timeframe) => {
                ("Active", Tone::Good)
            }
            Some(_) => ("Other symbol on chart", Tone::Warning),
        },
    }
}

/// The data source a `"<SLUG>:<ticker>"` key's prefix names.
fn source_of(prefix: &str) -> Option<DataSource> {
    [
        DataSource::Binance,
        DataSource::Coinbase,
        DataSource::CoinGecko,
        DataSource::DexScreener,
        DataSource::Alpaca,
        DataSource::Polymarket,
        DataSource::Kalshi,
        DataSource::CoinMarketCap,
    ]
    .into_iter()
    .find(|source| source.watchlist_slug().eq_ignore_ascii_case(prefix))
}

impl ScriptAlertRow {
    pub fn from_subscription(
        subscription: &PineAlertSubscription,
        dataset: Option<(&str, &str)>,
    ) -> Self {
        let (status, tone) = status_of(subscription, dataset);
        let (prefix, ticker) = subscription
            .symbol_key
            .split_once(':')
            .unwrap_or(("", &subscription.symbol_key));
        Self {
            id: subscription.id,
            script_name: subscription.script_name.clone(),
            symbol: ticker.to_owned(),
            source: source_of(prefix),
            timeframe: subscription.timeframe.clone(),
            note: subscription.note.clone(),
            webhook_count: subscription.webhook_endpoint_ids.len(),
            status,
            tone,
            active: subscription.is_active(),
            // A chart showing this market must be open to re-arm; a deleted script can't be.
            can_rearm: subscription.state != PineAlertState::ScriptDeleted
                && dataset.is_some_and(|(key, timeframe)| subscription.watches(key, timeframe)),
            created_at_ms: subscription.created_at_ms,
        }
    }

    /// "BTCUSDT · 1h · 2 webhooks", or the note in place of the market.
    pub fn detail(&self) -> String {
        let base = if self.note.is_empty() {
            format!("{} · {}", self.symbol, self.timeframe)
        } else {
            self.note.clone()
        };
        match self.webhook_count {
            0 => base,
            1 => format!("{base} · 1 webhook"),
            n => format!("{base} · {n} webhooks"),
        }
    }
}

/// Rows matching the search (script name or symbol), newest first.
pub fn rows_for<'a>(rows: &'a [ScriptAlertRow], search: &str) -> Vec<&'a ScriptAlertRow> {
    let needle = search.trim().to_lowercase();
    let mut found: Vec<&ScriptAlertRow> = rows
        .iter()
        .filter(|row| {
            needle.is_empty()
                || row.script_name.to_lowercase().contains(&needle)
                || row.symbol.to_lowercase().contains(&needle)
        })
        .collect();
    found.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
    found
}

pub fn view(
    ui: &mut egui::Ui,
    search: &str,
    all: &[ScriptAlertRow],
    actions: &mut Vec<AlertCenterAction>,
) {
    let rows = rows_for(all, search);
    let footer_height = 45.0;
    let body_height = (ui.available_height() - footer_height).max(60.0);
    let query = search.trim();
    if rows.is_empty() {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), body_height), Sense::hover());
        if all.is_empty() || query.is_empty() {
            kit::empty_state(
                ui,
                rect,
                Icon::Curlybraces,
                "No Script Alerts",
                "Apply a script with alert() calls, then create an alert from it.",
            );
        } else {
            kit::empty_state(
                ui,
                rect,
                Icon::Magnifier,
                &format!("No Results for “{query}”"),
                "Check the spelling or try a new search.",
            );
        }
    } else {
        ui.allocate_ui(Vec2::new(ui.available_width(), body_height), |ui| {
            alert_center::card_scroll(ui, body_height, |ui| {
                alert_center::day_header(ui, "Alerts", rows.len());
                for row in rows {
                    card(ui, row, actions);
                }
            });
        });
    }
    if kit::footer_bar(
        ui,
        Icon::Curlybraces,
        "Script alerts are created from alert() calls in your scripts.",
        Icon::Curlybraces,
        "Open Pine Studio",
        true,
        "Write and manage the scripts whose alert() calls appear here",
    ) {
        actions.push(AlertCenterAction::OpenPineStudio);
    }
}

fn menu(ui: &mut egui::Ui, row: &ScriptAlertRow, actions: &mut Vec<AlertCenterAction>) {
    if row.active {
        if kit::menu_item(ui, Icon::PauseFill, "Pause", true, false) {
            actions.push(AlertCenterAction::PauseScript(row.id));
            ui.close();
        }
    } else {
        let response =
            ui.scope(|ui| kit::menu_item(ui, Icon::PlayFill, "Re-arm", row.can_rearm, false));
        if !row.can_rearm {
            response
                .response
                .on_hover_text("Open its chart on the same market to re-arm it");
        }
        if response.inner {
            actions.push(AlertCenterAction::RearmScript(row.id));
            ui.close();
        }
    }
    if kit::menu_item(ui, Icon::ArrowUpRightSquare, "Webhooks…", true, false) {
        actions.push(AlertCenterAction::EditScriptWebhooks(row.id));
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Delete", true, true) {
        actions.push(AlertCenterAction::DeleteScript(row.id));
        ui.close();
    }
}

fn card(ui: &mut egui::Ui, row: &ScriptAlertRow, actions: &mut Vec<AlertCenterAction>) {
    let (rect, response) = alert_center::card(ui, Id::new(("script-alert-card", row.id)));
    let center_y = rect.center().y;
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    if !row.active {
        inner.multiply_opacity(0.7);
    }
    let ui = &mut inner;

    // A script glyph in a tinted disc, the market's source badged on the corner.
    let icon = alert_center::icon_rect(rect);
    ui.painter().circle_filled(
        icon.center(),
        icon.width() / 2.0,
        theme::with_alpha(kit::ACCENT, 0.14),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(icon.center(), Vec2::splat(15.0)),
        Icon::Curlybraces,
        kit::ACCENT,
    );
    if let Some(source) = row.source {
        kit::source_badge(ui, icon, source);
    }

    let more = Rect::from_min_size(
        Pos2::new(rect.right() - 14.0 - 26.0, center_y - 13.0),
        Vec2::splat(26.0),
    );
    alert_center::more_button(ui, more, Id::new(("script-alert-more", row.id)), |ui| {
        menu(ui, row, actions);
    });
    let badge = kit::paint_status_badge(ui, more.left() - 8.0, center_y, row.status, row.tone);

    alert_center::identity(
        ui,
        rect,
        icon.right() + 3.0 + 12.0,
        badge.left() - 12.0,
        &row.script_name,
        &row.detail(),
        |_, _, _| 0.0,
    );

    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            ui.set_width(180.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            menu(ui, row, actions);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subscription(state: PineAlertState) -> PineAlertSubscription {
        PineAlertSubscription {
            id: 4,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "Momentum".into(),
            source: String::new(),
            inputs: Default::default(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "h".into(),
            note: String::new(),
            state,
            created_at_ms: 5,
            webhook_endpoint_ids: vec![1, 2],
        }
    }

    #[test]
    fn the_badge_says_why_an_alert_cannot_fire() {
        let active = subscription(PineAlertState::Active);
        assert_eq!(status_of(&active, None), ("Chart not open", Tone::Neutral));
        assert_eq!(
            status_of(&active, Some(("binance:BTCUSDT", "1h"))),
            ("Active", Tone::Good)
        );
        assert_eq!(
            status_of(&active, Some(("binance:ETHUSDT", "1h"))),
            ("Other symbol on chart", Tone::Warning)
        );
        assert_eq!(
            status_of(&subscription(PineAlertState::Paused), None),
            ("Paused", Tone::Neutral)
        );
        assert_eq!(
            status_of(&subscription(PineAlertState::ScriptChanged), None).0,
            "Script changed — re-arm"
        );
        assert_eq!(
            status_of(&subscription(PineAlertState::ScriptDeleted), None),
            ("Script deleted", Tone::Bad)
        );
    }

    #[test]
    fn a_row_reads_market_webhooks_and_when_it_can_be_re_armed() {
        let paused = subscription(PineAlertState::Paused);
        let open = ScriptAlertRow::from_subscription(&paused, Some(("binance:BTCUSDT", "1h")));
        assert_eq!(open.detail(), "BTCUSDT · 1h · 2 webhooks");
        assert!(open.can_rearm && !open.active);
        let closed = ScriptAlertRow::from_subscription(&paused, None);
        assert!(!closed.can_rearm);
        let deleted = ScriptAlertRow::from_subscription(
            &subscription(PineAlertState::ScriptDeleted),
            Some(("binance:BTCUSDT", "1h")),
        );
        assert!(!deleted.can_rearm, "a deleted script can't be re-armed");
        let mut noted = subscription(PineAlertState::Active);
        noted.note = "Breakout".into();
        noted.webhook_endpoint_ids = vec![1];
        assert_eq!(
            ScriptAlertRow::from_subscription(&noted, None).detail(),
            "Breakout · 1 webhook"
        );
    }

    #[test]
    fn search_matches_name_or_symbol_newest_first() {
        let mut rows = vec![
            ScriptAlertRow::from_subscription(&subscription(PineAlertState::Active), None),
            ScriptAlertRow::from_subscription(&subscription(PineAlertState::Active), None),
        ];
        rows[1].script_name = "Trend".into();
        rows[1].created_at_ms = 9;
        let names = |search: &str| -> Vec<String> {
            rows_for(&rows, search)
                .iter()
                .map(|r| r.script_name.clone())
                .collect()
        };
        assert_eq!(names(""), ["Trend", "Momentum"]);
        assert_eq!(names("mom"), ["Momentum"]);
        assert_eq!(names("btc"), ["Trend", "Momentum"]);
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn the_list_draws_headlessly_empty_populated_and_searched() {
        let rows = [
            ScriptAlertRow::from_subscription(
                &subscription(PineAlertState::Active),
                Some(("binance:BTCUSDT", "1h")),
            ),
            ScriptAlertRow::from_subscription(&subscription(PineAlertState::Paused), None),
        ];
        let ctx = egui::Context::default();
        for (search, list) in [("", &rows[..0]), ("", &rows[..]), ("zzz", &rows[..])] {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                view(ui, search, list, &mut actions);
            });
            output.textures_delta.clear();
            assert!(actions.is_empty(), "nothing was clicked");
        }
    }
}
