//! Time spent waiting for data is not time the script spent.
//!
//! `request.security()` asks a [`SecurityDataProvider`] for another symbol's candles, and the
//! provider may fetch them over the network, blocking. That happens inside whichever statement made
//! the request, so a profile that only watched the clock would charge the network to that line. A
//! [`WaitMeter`] stands between the script and its provider, measures each call and adds it to a
//! counter the recorder takes out of the clock it reads. What the script *does* with the data is
//! still timed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::model::Kline;
use crate::pine::runtime_support::{ChartSpan, SecurityDataProvider, SecurityKey};

pub struct WaitMeter {
    inner: Box<dyn SecurityDataProvider>,
    waited: Arc<AtomicU64>,
}

impl WaitMeter {
    pub fn new(inner: Box<dyn SecurityDataProvider>, waited: Arc<AtomicU64>) -> Self {
        Self { inner, waited }
    }

    fn timed<T>(&self, ask: impl FnOnce(&dyn SecurityDataProvider) -> T) -> T {
        let started = Instant::now();
        let answer = ask(self.inner.as_ref());
        self.waited
            .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        answer
    }
}

impl SecurityDataProvider for WaitMeter {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        self.timed(|inner| inner.candles(key))
    }

    fn candles_over(&self, key: &SecurityKey, chart: ChartSpan) -> Option<Vec<Kline>> {
        self.timed(|inner| inner.candles_over(key, chart))
    }
}
