use super::*;
use crate::model::DataSource;
use crate::model::alert::AlertServiceState;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

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
    let registration = AlertRegistration::new(
        notifier,
        BackgroundAlertService::default(),
        AlertRuntimeControl::new(AlertServiceState::Disabled, 0),
    );
    let alert = Box::new(crate::model::alert::PriceAlert::new(
        crate::model::alert::AlertAsset::new(
            crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        ),
        crate::model::alert::AlertCondition::CrossesAbove { target: 1.0 },
        1,
    ));
    let result = registration.commit_checked(&store, alert, None, None, 1);
    assert!(result.is_err());
    // Give a wrongly spawned authorization thread the chance to show itself.
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        counting.0.load(Ordering::SeqCst),
        0,
        "no authorization is requested for an alert that does not exist"
    );
    assert_eq!(
        registration.service_state(),
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
