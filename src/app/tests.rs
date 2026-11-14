use super::alert::commit_price_alert_checked;
use super::session::PersistedSession;
use super::window::set_window_title;
use super::*;
use crate::model::alert::AlertServiceState;
use crate::model::{ChartColumn, DataSource};
use crate::workspace::PersistedChart;
use crate::workspace::PersistedWorkspace;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

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

#[derive(Default)]
struct CountingNotifier(std::sync::atomic::AtomicUsize);

impl Notifier for CountingNotifier {
    fn deliver(
        &self,
        _: &crate::model::alert::notification::NotificationContent,
    ) -> Result<(), String> {
        Ok(())
    }

    fn request_authorization(
        &self,
    ) -> Result<crate::model::alert::AlertNotificationPermission, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(crate::model::alert::AlertNotificationPermission::Unknown)
    }
}

#[test]
fn an_alert_whose_save_never_reached_the_queue_is_an_error_with_no_side_effects() {
    let store = crate::store::Store::open_in_memory().unwrap();
    store.break_alert_queue();
    let counting = Arc::new(CountingNotifier::default());
    let notifier: Arc<dyn Notifier> = counting.clone();
    let service = Arc::new(Mutex::new(BackgroundAlertService::default()));
    let control = Arc::new(Mutex::new(AlertRuntimeControl::new(
        AlertServiceState::Disabled,
        0,
    )));
    let alert = Box::new(crate::model::alert::PriceAlert::new(
        crate::model::alert::AlertAsset::new(
            crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        ),
        crate::model::alert::AlertCondition::CrossesAbove { target: 1.0 },
        1,
    ));
    let result =
        commit_price_alert_checked(&store, &notifier, &service, &control, alert, None, None, 1);
    assert!(result.is_err());
    // Give a wrongly spawned authorization thread the chance to show itself.
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        counting.0.load(Ordering::SeqCst),
        0,
        "no authorization is requested for an alert that does not exist"
    );
    assert_eq!(
        control.lock().unwrap().service_state,
        AlertServiceState::Disabled,
        "no service setup either"
    );
}

#[test]
fn service_status_answer_replaces_the_state_once_it_arrives() {
    let mut control = AlertRuntimeControl::new(AlertServiceState::Disabled, 0);
    let (sender, receiver) = mpsc::channel();
    control.status_probe = Some(receiver);

    control.poll_service_status();
    assert_eq!(control.service_state, AlertServiceState::Disabled);
    assert!(control.status_probe.is_some());

    sender.send(AlertServiceState::Enabled).unwrap();
    control.poll_service_status();
    assert_eq!(control.service_state, AlertServiceState::Enabled);
    assert!(control.status_probe.is_none());
}

#[test]
fn a_registration_drops_the_status_query_started_before_it() {
    let mut control = AlertRuntimeControl::new(AlertServiceState::Disabled, 0);
    let (sender, receiver) = mpsc::channel();
    control.status_probe = Some(receiver);

    control.retrying(AlertServiceState::Enabled, 10);
    let _ = sender.send(AlertServiceState::Disabled);
    control.poll_service_status();
    assert_eq!(control.service_state, AlertServiceState::Enabled);
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
