//! The exchanges' public trade streams for one asset, added up into one [`Flow`] while somebody is
//! looking: the `{ASSET}USDT` spot and perpetual markets of Binance and Bybit. One thread per
//! market (`service::stream`), on the lease the viewer renews each frame it draws.

mod binance;
mod bybit;

use crate::clock::now_ms;
use crate::lock::lock;
use crate::model::taker_flow::{Flow, Trade, Venue};
use crate::service::net::ws::Session;
pub use crate::service::stream::{Lease, Repaint};
use crate::service::stream::{Stream, Worker};
use std::sync::{Arc, Mutex};

/// Turns one message of a venue's socket into trades.
type Parser = fn(Venue, &str) -> Vec<Trade>;

/// The running sockets of one asset; dropping it closes them.
pub struct TakerFlowFeed {
    stream: Stream,
}

impl TakerFlowFeed {
    /// Starts every venue's trade stream for `asset` (its upper case letters and digits: `BTC`),
    /// adding what it trades to `flow`.
    pub fn start(asset: &str, flow: Arc<Mutex<Flow>>, lease: Lease, repaint: Repaint) -> Self {
        let stream = Stream::new(lease, repaint);
        for venue in Venue::ALL {
            let flow = Arc::clone(&flow);
            let asset = asset.to_owned();
            stream.spawn(move |worker| {
                use crate::model::liquidation::Exchange;
                let (session, parse): (Session, Parser) = match venue.exchange {
                    Exchange::Binance => (binance::session(venue.market, &asset), binance::parse),
                    Exchange::Bybit => (bybit::session(venue.market, &asset), bybit::parse),
                };
                follow(worker, &flow, &session, venue, parse);
            });
        }
        Self { stream }
    }

    /// Whether every stream still runs; see [`Stream::is_running`].
    pub fn is_running(&self) -> bool {
        self.stream.is_running()
    }
}

fn follow(worker: &Worker, flow: &Mutex<Flow>, session: &Session, venue: Venue, parse: Parser) {
    worker.run_linked(
        session,
        |up| {
            lock(flow).link(venue, up, now_ms());
            worker.wake();
        },
        |text| {
            if add(flow, parse(venue, text)) {
                worker.wake();
            }
            true
        },
    );
}

/// Puts a message's trades on the flow; whether there were any.
fn add(flow: &Mutex<Flow>, trades: Vec<Trade>) -> bool {
    if trades.is_empty() {
        return false;
    }
    let now = now_ms();
    let mut flow = lock(flow);
    for trade in trades {
        flow.push(trade, now);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::taker_flow::Taker;
    use std::time::Duration;

    #[test]
    fn a_message_with_trades_fills_the_flow_and_one_without_changes_nothing() {
        let flow = Mutex::new(Flow::default());
        assert!(!add(&flow, Vec::new()));
        assert_eq!(lock(&flow).revision(), 0);
        let trade = Trade {
            venue: Venue::ALL[0],
            notional: 250.0,
            taker: Taker::Buy,
            time_ms: now_ms(),
        };
        assert!(add(&flow, vec![trade, trade]));
        assert_eq!(lock(&flow).snapshot(now_ms()).all.minute.buy, 500.0);
    }

    /// Listens to the real sockets for BTC until every venue has traded, and prints one trade of
    /// each. Run with
    /// `cargo test --lib taker_flow::tests::live_venues -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_venues_trade_btc() {
        let flow = Arc::new(Mutex::new(Flow::default()));
        let lease = Lease::default();
        lease.renew(Duration::from_secs(120));
        let feed = TakerFlowFeed::start("BTC", Arc::clone(&flow), lease, Arc::new(|| {}));
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut heard = [false; 4];
        while std::time::Instant::now() < deadline && heard.iter().any(|seen| !seen) {
            std::thread::sleep(Duration::from_secs(1));
            let snapshot = lock(&flow).snapshot(now_ms());
            for (index, (venue, spans)) in snapshot.venues.iter().enumerate() {
                if !heard[index] && spans.hour.total() > 0.0 {
                    heard[index] = true;
                    println!(
                        "{}: bought {:.0}, sold {:.0}",
                        venue.label(),
                        spans.hour.buy,
                        spans.hour.sell
                    );
                }
            }
        }
        assert!(feed.is_running());
        assert!(heard.iter().all(|seen| *seen), "heard from {heard:?}");
        let observed = lock(&flow).snapshot(now_ms()).all.observed;
        assert!(observed.minute > 0, "every socket reported being connected");
    }
}
