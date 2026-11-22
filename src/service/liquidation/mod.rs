//! The exchanges' public liquidation streams, kept in one [`Tape`] while somebody is looking.
//!
//! One thread per exchange (`service::stream`) reconnects with backoff, on a lease the viewer
//! renews each frame it draws.

mod binance;
mod bybit;

use crate::clock::now_ms;
use crate::lock::lock;
use crate::model::liquidation::{Liquidation, Tape};
use crate::service::net::ws::Session;
pub use crate::service::stream::{Lease, Repaint};
use crate::service::stream::{Stream, Worker};
use std::sync::{Arc, Mutex};

/// The running sockets; dropping it closes them.
pub struct LiquidationFeed {
    stream: Stream,
}

impl LiquidationFeed {
    /// Starts every exchange's stream, adding what they announce to `tape`.
    pub fn start(tape: Arc<Mutex<Tape>>, lease: Lease, repaint: Repaint) -> Self {
        let stream = Stream::new(lease, repaint);
        let binance_tape = Arc::clone(&tape);
        stream.spawn(move |worker| {
            follow(worker, &binance_tape, &binance::session(), binance::parse);
        });
        stream.spawn(move |worker| {
            let Some(session) = bybit::session(worker) else {
                return;
            };
            follow(worker, &tape, &session, bybit::parse);
        });
        Self { stream }
    }

    /// Whether any stream still runs. They all end once the lease lapses, and then a new feed is
    /// needed.
    pub fn is_running(&self) -> bool {
        self.stream.is_running()
    }
}

fn follow(
    worker: &Worker,
    tape: &Mutex<Tape>,
    session: &Session,
    parse: fn(&str) -> Vec<Liquidation>,
) {
    worker.run(session, |text| {
        if add(tape, parse(text)) {
            worker.wake();
        }
        true
    });
}

/// Puts a message's liquidations on the tape; whether there were any.
fn add(tape: &Mutex<Tape>, liquidations: Vec<Liquidation>) -> bool {
    if liquidations.is_empty() {
        return false;
    }
    let now = now_ms();
    let mut tape = lock(tape);
    for liquidation in liquidations {
        tape.push(liquidation, now);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::exchange::Exchange;
    use crate::model::liquidation::{MinSize, Side};
    use std::time::Duration;

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
    fn a_message_with_liquidations_fills_the_tape_and_one_without_changes_nothing() {
        let tape = Mutex::new(Tape::default());
        assert!(!add(&tape, Vec::new()));
        assert_eq!(lock(&tape).revision(), 0);
        assert!(add(&tape, vec![liquidation(), liquidation()]));
        let snapshot = lock(&tape).snapshot(now_ms(), MinSize::Any);
        assert_eq!(snapshot.latest.len(), 2);
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
}
