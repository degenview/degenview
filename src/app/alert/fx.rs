//! What a US dollar is worth in the other currencies a price alert can be set in.

use crate::model::alert::AlertCurrency;
use crate::model::portfolio::PortfolioCurrency;
use crate::service::market::fx_rates::FxRateService;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The rates for the chart windows' and the Alerts window's price-alert sheets. Cheap to clone;
/// the window callbacks each hold one.
#[derive(Clone, Default)]
pub(in crate::app) struct SharedFxRates(Arc<Mutex<HashMap<AlertCurrency, f64>>>);

impl SharedFxRates {
    /// Runs `read` on the rates, unless they cannot be reached.
    pub(in crate::app) fn with<R>(
        &self,
        read: impl FnOnce(&HashMap<AlertCurrency, f64>) -> R,
    ) -> Option<R> {
        self.0.lock().ok().map(|rates| read(&rates))
    }

    fn set(&self, rates: &HashMap<AlertCurrency, f64>) {
        if let Ok(mut shared) = self.0.lock() {
            shared.clone_from(rates);
        }
    }
}

/// USD rates of the alert currencies, fetched off the frame at most once an hour while an alert
/// or an open sheet needs them. The app thread reads its own copy; the windows read the shared one.
pub(super) struct AlertFxFeed {
    service: Arc<FxRateService>,
    rates: HashMap<AlertCurrency, f64>,
    shared: SharedFxRates,
    receiver: Option<Receiver<HashMap<AlertCurrency, f64>>>,
    refreshed_at: Option<Instant>,
}

impl AlertFxFeed {
    pub(super) fn new(service: Arc<FxRateService>) -> Self {
        Self {
            service,
            rates: HashMap::new(),
            shared: SharedFxRates::default(),
            receiver: None,
            refreshed_at: None,
        }
    }

    pub(super) fn rates(&self) -> &HashMap<AlertCurrency, f64> {
        &self.rates
    }

    pub(super) fn shared(&self) -> &SharedFxRates {
        &self.shared
    }

    /// Takes a finished fetch's rates. True when new rates came in.
    pub(super) fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };
        match receiver.try_recv() {
            Ok(rates) => {
                self.shared.set(&rates);
                self.rates = rates;
                self.receiver = None;
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                false
            }
            Err(mpsc::TryRecvError::Empty) => false,
        }
    }

    /// Starts a fetch, unless one is running or the last started less than an hour ago.
    pub(super) fn refresh_if_stale(&mut self) {
        if self.receiver.is_some()
            || self
                .refreshed_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(3600))
        {
            return;
        }
        let fx = Arc::clone(&self.service);
        let (sender, receiver) = mpsc::channel();
        crate::service::work_queue::submit(move || {
            let rates = [
                AlertCurrency::Eur,
                AlertCurrency::Gbp,
                AlertCurrency::Jpy,
                AlertCurrency::Chf,
            ]
            .into_iter()
            .filter_map(|currency| {
                fx.rate(
                    PortfolioCurrency::Usd,
                    match currency {
                        AlertCurrency::Eur => PortfolioCurrency::Eur,
                        AlertCurrency::Gbp => PortfolioCurrency::Gbp,
                        AlertCurrency::Jpy => PortfolioCurrency::Jpy,
                        AlertCurrency::Chf => PortfolioCurrency::Chf,
                        AlertCurrency::Usd => PortfolioCurrency::Usd,
                    },
                )
                .map(|rate| (currency, rate))
            })
            .collect();
            let _ = sender.send(rates);
        });
        self.receiver = Some(receiver);
        self.refreshed_at = Some(Instant::now());
    }
}
