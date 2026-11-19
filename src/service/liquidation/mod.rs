//! The exchanges' public liquidation streams, kept in one [`Tape`] while somebody is looking.
//!
//! One thread per exchange runs a `net::ws::Session` that reconnects with backoff. The feed lives
//! on a lease the viewer renews each frame it draws: when the lease lapses (the panel closed, a tab
//! in the background) the threads end at their next message and the sockets close.

mod binance;
mod bybit;

use crate::lock::lock;
use crate::model::liquidation::{Liquidation, Tape};
use crate::service::net::ws::Session;
use crate::service::now_ms;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often a burst of liquidations asks for a repaint at most.
const REPAINT_EVERY_MS: i64 = 250;

/// Asks the window to draw a frame.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// The time until which the viewer wants the feed, in milliseconds since the epoch.
#[derive(Clone, Default)]
pub struct Lease(Arc<AtomicI64>);

impl Lease {
    /// Wanted for `span` from now.
    pub fn renew(&self, span: Duration) {
        self.0
            .store(now_ms() + span.as_millis() as i64, Ordering::Release);
    }

    fn is_held(&self) -> bool {
        now_ms() < self.0.load(Ordering::Acquire)
    }
}

/// What the threads of one feed share.
struct Shared {
    tape: Arc<Mutex<Tape>>,
    lease: Lease,
    repaint: Repaint,
    last_repaint: AtomicI64,
    shutdown: AtomicBool,
    running: AtomicUsize,
}

impl Shared {
    /// Puts a message's liquidations on the tape. `false` once nobody wants the feed.
    fn take(&self, liquidations: Vec<Liquidation>) -> bool {
        if !self.lease.is_held() {
            return false;
        }
        if liquidations.is_empty() {
            return true;
        }
        let now = now_ms();
        {
            let mut tape = lock(&self.tape);
            for liquidation in liquidations {
                tape.push(liquidation, now);
            }
        }
        if now - self.last_repaint.load(Ordering::Relaxed) >= REPAINT_EVERY_MS {
            self.last_repaint.store(now, Ordering::Relaxed);
            (self.repaint)();
        }
        true
    }
}

/// The running sockets; dropping it closes them.
pub struct LiquidationFeed {
    shared: Arc<Shared>,
}

impl LiquidationFeed {
    /// Starts every exchange's stream, adding what they announce to `tape`.
    pub fn start(tape: Arc<Mutex<Tape>>, lease: Lease, repaint: Repaint) -> Self {
        let shared = Arc::new(Shared {
            tape,
            lease,
            repaint,
            last_repaint: AtomicI64::new(0),
            shutdown: AtomicBool::new(false),
            running: AtomicUsize::new(0),
        });
        spawn(&shared, |shared| {
            let session = binance::session();
            run(shared, &session, binance::parse);
        });
        spawn(&shared, |shared| {
            let Some(session) = bybit::session(&shared.shutdown) else {
                return;
            };
            run(shared, &session, bybit::parse);
        });
        Self { shared }
    }

    /// Whether any stream still runs. They all end once the lease lapses, and then a new feed is
    /// needed.
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire) > 0
    }
}

impl Drop for LiquidationFeed {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
    }
}

fn spawn(shared: &Arc<Shared>, work: impl FnOnce(&Shared) + Send + 'static) {
    shared.running.fetch_add(1, Ordering::AcqRel);
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        work(&shared);
        shared.running.fetch_sub(1, Ordering::AcqRel);
    });
}

fn run(shared: &Shared, session: &Session, parse: fn(&str) -> Vec<Liquidation>) {
    session.run(&shared.shutdown, |text| shared.take(parse(text)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::liquidation::{Exchange, MinSize, Side};

    fn shared(lease: &Lease) -> Shared {
        Shared {
            tape: Arc::default(),
            lease: lease.clone(),
            repaint: Arc::new(|| {}),
            last_repaint: AtomicI64::new(0),
            shutdown: AtomicBool::new(false),
            running: AtomicUsize::new(0),
        }
    }

    fn liquidation() -> Liquidation {
        Liquidation {
            exchange: Exchange::Binance,
            symbol: "BTCUSDT".to_owned(),
            side: Side::Long,
            price: 100.0,
            quantity: 1.0,
            time_ms: now_ms(),
        }
    }

    #[test]
    fn a_held_lease_fills_the_tape_and_a_lapsed_one_stops_the_feed() {
        let lease = Lease::default();
        let shared = shared(&lease);
        assert!(!shared.take(vec![liquidation()]), "no lease yet");
        assert_eq!(lock(&shared.tape).revision(), 0);

        lease.renew(Duration::from_secs(5));
        assert!(shared.take(vec![liquidation()]));
        assert!(
            shared.take(Vec::new()),
            "a message without one keeps reading"
        );
        let snapshot = lock(&shared.tape).snapshot(now_ms(), MinSize::Any);
        assert_eq!(snapshot.latest.len(), 1);
    }

    #[test]
    fn a_burst_asks_for_one_repaint() {
        let lease = Lease::default();
        lease.renew(Duration::from_secs(5));
        let asked = Arc::new(AtomicUsize::new(0));
        let mut shared = shared(&lease);
        let counter = Arc::clone(&asked);
        shared.repaint = Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        for _ in 0..50 {
            shared.take(vec![liquidation()]);
        }
        assert_eq!(asked.load(Ordering::Relaxed), 1);
    }

    /// Listens to the real sockets until each exchange has announced one liquidation, and prints
    /// them. Run with
    /// `cargo test --lib liquidation::tests::live_exchanges -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_exchanges_announce_liquidations() {
        let tape = Arc::new(Mutex::new(Tape::default()));
        let lease = Lease::default();
        lease.renew(Duration::from_secs(300));
        let feed = LiquidationFeed::start(Arc::clone(&tape), lease, Arc::new(|| {}));
        let deadline = std::time::Instant::now() + Duration::from_secs(240);
        let mut seen = std::collections::HashSet::new();
        while std::time::Instant::now() < deadline && seen.len() < 2 {
            std::thread::sleep(Duration::from_secs(2));
            let snapshot = lock(&tape).snapshot(now_ms(), MinSize::Any);
            for row in &snapshot.latest {
                if seen.insert(row.exchange) {
                    println!("{row:?} worth {:.2}", row.notional());
                }
            }
        }
        assert!(feed.is_running());
        println!("heard from {} exchanges", seen.len());
        assert!(!seen.is_empty(), "no exchange announced a liquidation");
    }

    #[test]
    fn dropping_the_feed_stops_its_threads() {
        let shared = Arc::new(shared(&Lease::default()));
        let feed = LiquidationFeed {
            shared: Arc::clone(&shared),
        };
        assert!(!feed.is_running());
        drop(feed);
        assert!(shared.shutdown.load(Ordering::Acquire));
    }
}
