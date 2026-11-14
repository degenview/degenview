use super::session::PersistedSession;
use super::window::set_window_title;
use super::*;
use crate::model::{ChartColumn, DataSource};
use crate::workspace::PersistedChart;
use crate::workspace::PersistedWorkspace;

fn title_commands(ctx: &egui::Context, title: &str) -> usize {
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        set_window_title(ui.ctx(), title.to_owned());
    });
    output.textures_delta.clear();
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(0, |viewport| {
            viewport
                .commands
                .iter()
                .filter(|command| matches!(command, egui::ViewportCommand::Title(_)))
                .count()
        })
}

#[test]
fn a_window_is_titled_only_when_its_title_changes() {
    let ctx = egui::Context::default();
    assert_eq!(title_commands(&ctx, "BTCUSDT"), 1);
    // A viewport command asks egui for another frame, so repeating it never lets the app idle.
    assert_eq!(title_commands(&ctx, "BTCUSDT"), 0);
    assert_eq!(title_commands(&ctx, "BTCUSDT"), 0);
    assert_eq!(title_commands(&ctx, "ETHUSDT"), 1);
    assert_eq!(title_commands(&ctx, "ETHUSDT"), 0);
}

#[test]
fn persisted_session_round_trips() {
    let session = PersistedSession {
        next_id: 3,
        tabs: vec![PersistedTab::Chart(PersistedWorkspace {
            id: 2,
            timeframe: Timeframe::Hour,
            watchlist_visible: true,
            charts: vec![PersistedChart {
                id: 1,
                symbol: "ETHUSDT".into(),
                source: DataSource::Binance,
                log_scale: false,
                scale_mode: None,
                y_zoom: 1.0,
                indicators: crate::model::IndicatorConfig::default(),
                appearance: crate::model::ChartAppearance::default(),
                drawings: Vec::new(),
                pine_instances: Vec::new(),
                candle_count: 0,
                viewport: Default::default(),
                title: None,
                pm_series: Vec::new(),
                image_url: None,
                card: None,
                overlays: Default::default(),
            }],
            columns: vec![ChartColumn::new(10, vec![1])],
            focused: Some(1),
            sync: Default::default(),
            active_view: Some(9),
            custom_title: None,
            link: None,
        })],
    };
    let encoded = serde_json::to_string(&session).expect("serialize session");
    let decoded: PersistedSession = serde_json::from_str(&encoded).expect("deserialize session");
    assert_eq!(decoded, session);
}

#[test]
fn old_chart_tab_shape_and_portfolio_tab_both_decode() {
    let old_chart = serde_json::json!({
        "id": 2,
        "timeframe": "Hour",
        "watchlist_visible": true,
        "charts": [],
        "columns": [],
        "focused": null
    });
    let portfolio = serde_json::json!({ "id": 9, "kind": "portfolio" });

    let chart: PersistedTab = serde_json::from_value(old_chart).expect("old chart tab");
    let special: PersistedTab = serde_json::from_value(portfolio).expect("portfolio tab");
    assert!(matches!(chart, PersistedTab::Chart(_)));
    assert_eq!(
        special,
        PersistedTab::Special(PersistedSpecialTab {
            id: 9,
            kind: PersistedSpecialTabKind::Portfolio,
        })
    );
}
