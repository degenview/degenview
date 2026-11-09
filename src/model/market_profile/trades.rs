//! Exchange trades with an aggressor side, deduplicated and bounded.
//!
//! A trade is what the exchange reports (Binance `aggTrade`): an id, a time, a price, a size and
//! whether the **buyer was the maker**. The aggressor is the other side, so `buyer_is_maker == false`
//! is a market buy (the exchange-reported method, TradingView's first choice for tick-based
//! footprints).
//!
//! [`TradeBook`] is where the history REST pages and the live stream meet. It sorts by `(time, id)`,
//! drops a trade it already holds (a reconnect or a page overlap re-delivers them), and keeps at most
//! `capacity` trades, dropping the oldest. It also knows **how far back it is complete**: trade ids
//! are consecutive on the exchanges that matter, so a gap in the ids is a gap in the data, and a bar
//! is only built from trades when the whole bar lies after the last gap. A bar the book does not
//! fully cover falls back to candles; the two are never mixed inside one bar.

use std::cmp::Ordering;

/// Trades kept by default (about 40 MB).
pub const DEFAULT_CAPACITY: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trade {
    /// The exchange's id for the trade; consecutive per market.
    pub id: u64,
    pub time_ms: i64,
    pub price: f64,
    /// Base-asset size.
    pub quantity: f64,
    /// True when the buyer's order was resting (so the seller hit it: a sell).
    pub buyer_is_maker: bool,
}

impl Trade {
    /// A market buy: the aggressor was the buyer.
    pub fn is_buy(&self) -> bool {
        !self.buyer_is_maker
    }

    /// A usable trade: finite positive price, finite non-negative size.
    pub fn is_valid(&self) -> bool {
        self.price.is_finite()
            && self.price > 0.0
            && self.quantity.is_finite()
            && self.quantity >= 0.0
    }

    fn key(&self) -> (i64, u64) {
        (self.time_ms, self.id)
    }
}

/// What an insert did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Inserted {
    pub added: usize,
    pub duplicates: usize,
    pub invalid: usize,
}

#[derive(Clone, Debug)]
pub struct TradeBook {
    /// Sorted by `(time_ms, id)`, no two with the same key.
    trades: Vec<Trade>,
    capacity: usize,
    /// Trades dropped from the front for capacity: what came before them is not held.
    evicted: bool,
    /// Cached index of the first trade of the last gap-free run, `None` when stale.
    run_start: Option<usize>,
}

impl Default for TradeBook {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl TradeBook {
    /// A book that keeps at most `capacity` trades (at least one).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            trades: Vec::new(),
            capacity: capacity.max(1),
            evicted: false,
            run_start: None,
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.trades.len()
    }

    #[cfg(test)]
    #[allow(dead_code)] // the pair of `len`, which clippy asks for
    pub fn is_empty(&self) -> bool {
        self.trades.is_empty()
    }

    #[cfg(test)]
    pub fn trades(&self) -> &[Trade] {
        &self.trades
    }

    #[cfg(test)]
    pub fn oldest(&self) -> Option<&Trade> {
        self.trades.first()
    }

    pub fn newest(&self) -> Option<&Trade> {
        self.trades.last()
    }

    /// Adds trades in any order. A trade already held (same time and id) or not valid is skipped.
    ///
    /// A batch is sorted and merged into the book in one pass, so a page of history that is older
    /// than everything held (history arrives newest first) costs one pass over the book, not a move of
    /// the whole book per trade.
    pub fn insert(&mut self, incoming: &[Trade]) -> Inserted {
        let mut result = Inserted::default();
        let mut batch: Vec<Trade> = Vec::with_capacity(incoming.len());
        for trade in incoming {
            if trade.is_valid() {
                batch.push(*trade);
            } else {
                result.invalid += 1;
            }
        }
        batch.sort_unstable_by_key(Trade::key);
        let before = batch.len();
        batch.dedup_by_key(|trade| trade.key());
        result.duplicates += before - batch.len();
        let Some(first) = batch.first().copied() else {
            return result;
        };
        match self.trades.last() {
            // The common case, a live trade or a page after the newest: append.
            Some(last) if last.key() >= first.key() => {}
            _ => {
                result.added = batch.len();
                self.trades.extend(batch);
                self.run_start = None;
                self.trim();
                return result;
            }
        }
        // Merge two sorted lists, dropping what the book already holds.
        let mut merged = Vec::with_capacity(self.trades.len() + batch.len());
        let (mut held, mut new) = (self.trades.iter().peekable(), batch.iter().peekable());
        loop {
            match (held.peek(), new.peek()) {
                (Some(h), Some(n)) => match h.key().cmp(&n.key()) {
                    Ordering::Less => merged.push(*held.next().expect("peeked")),
                    Ordering::Greater => {
                        merged.push(*new.next().expect("peeked"));
                        result.added += 1;
                    }
                    Ordering::Equal => {
                        merged.push(*held.next().expect("peeked"));
                        new.next();
                        result.duplicates += 1;
                    }
                },
                (Some(_), None) => merged.push(*held.next().expect("peeked")),
                (None, Some(_)) => {
                    merged.push(*new.next().expect("peeked"));
                    result.added += 1;
                }
                (None, None) => break,
            }
        }
        if result.added > 0 {
            self.trades = merged;
            self.run_start = None;
            self.trim();
        }
        result
    }

    fn trim(&mut self) {
        if self.trades.len() > self.capacity {
            let excess = self.trades.len() - self.capacity;
            self.trades.drain(..excess);
            self.evicted = true;
        }
    }

    /// The trades with `start_ms <= time < end_ms`.
    pub fn between(&self, start_ms: i64, end_ms: i64) -> &[Trade] {
        let from = self.trades.partition_point(|t| t.time_ms < start_ms);
        let to = self.trades.partition_point(|t| t.time_ms < end_ms);
        &self.trades[from..to.max(from)]
    }

    /// The first instant from which the book holds **every** trade up to its newest: trades after
    /// the last gap in the ids. `None` for an empty book. A trade shares its millisecond with
    /// others that may be missing before the gap, so coverage starts the millisecond after it.
    pub fn complete_from_ms(&mut self) -> Option<i64> {
        if self.trades.is_empty() {
            return None;
        }
        let start = match self.run_start {
            Some(start) => start,
            None => {
                let mut start = self.trades.len() - 1;
                while start > 0 {
                    let (before, after) = (&self.trades[start - 1], &self.trades[start]);
                    if after.id != before.id.wrapping_add(1) {
                        break;
                    }
                    start -= 1;
                }
                self.run_start = Some(start);
                start
            }
        };
        let first = &self.trades[start];
        // The very first trade the book ever saw is not preceded by a gap we can see, but the
        // history before it is unknown: the same one-millisecond margin applies.
        Some(first.time_ms + 1)
    }

    /// Whether every trade of the bar starting at `bar_start_ms` is held (the bar may still be
    /// forming: only its start has to be inside the complete run).
    pub fn covers_from(&mut self, bar_start_ms: i64) -> bool {
        self.complete_from_ms()
            .is_some_and(|from| bar_start_ms >= from)
    }

    #[cfg(test)]
    /// Whether older trades were dropped for capacity.
    pub fn has_evicted(&self) -> bool {
        self.evicted
    }

    #[cfg(test)]
    /// Drops every trade older than `time_ms` (a bound on age as well as count).
    pub fn forget_before(&mut self, time_ms: i64) {
        let cut = self.trades.partition_point(|t| t.time_ms < time_ms);
        if cut > 0 {
            self.trades.drain(..cut);
            self.evicted = true;
            self.run_start = None;
        }
    }

    /// The id to ask the exchange for next to continue after the newest trade.
    #[cfg(test)]
    pub fn next_id(&self) -> Option<u64> {
        self.newest().map(|t| t.id.wrapping_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(id: u64, time_ms: i64, price: f64, quantity: f64, buyer_is_maker: bool) -> Trade {
        Trade {
            id,
            time_ms,
            price,
            quantity,
            buyer_is_maker,
        }
    }

    fn run(from: u64, count: u64) -> Vec<Trade> {
        (from..from + count)
            .map(|id| trade(id, 1_000 + id as i64 * 10, 100.0, 1.0, id % 2 == 0))
            .collect()
    }

    #[test]
    fn the_aggressor_is_the_side_that_was_not_resting() {
        assert!(trade(1, 0, 1.0, 1.0, false).is_buy());
        assert!(!trade(1, 0, 1.0, 1.0, true).is_buy());
    }

    #[test]
    fn invalid_trades_are_skipped_and_counted() {
        let mut book = TradeBook::default();
        let result = book.insert(&[
            trade(1, 0, f64::NAN, 1.0, false),
            trade(2, 0, 0.0, 1.0, false),
            trade(3, 0, 1.0, -1.0, false),
            trade(4, 0, 1.0, f64::INFINITY, false),
            trade(5, 0, 1.0, 0.0, false),
        ]);
        assert_eq!(
            result,
            Inserted {
                added: 1,
                duplicates: 0,
                invalid: 4
            }
        );
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn a_redelivered_trade_is_not_counted_twice() {
        let mut book = TradeBook::default();
        book.insert(&run(1, 5));
        let again = book.insert(&run(3, 5)); // 3, 4, 5 repeat; 6, 7 are new
        assert_eq!(
            again,
            Inserted {
                added: 2,
                duplicates: 3,
                invalid: 0
            }
        );
        assert_eq!(book.len(), 7);
        let total: f64 = book.trades().iter().map(|t| t.quantity).sum();
        assert_eq!(total, 7.0);
    }

    #[test]
    fn trades_arriving_out_of_order_end_up_sorted() {
        let mut book = TradeBook::default();
        let mut shuffled = run(1, 6);
        shuffled.swap(0, 5);
        shuffled.swap(2, 3);
        book.insert(&shuffled);
        let ids: Vec<u64> = book.trades().iter().map(|t| t.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn two_trades_in_the_same_millisecond_are_ordered_by_id() {
        let mut book = TradeBook::default();
        book.insert(&[trade(8, 50, 1.0, 1.0, false), trade(7, 50, 1.0, 1.0, false)]);
        let ids: Vec<u64> = book.trades().iter().map(|t| t.id).collect();
        assert_eq!(ids, [7, 8]);
    }

    #[test]
    fn between_is_half_open_on_time() {
        let mut book = TradeBook::default();
        book.insert(&run(1, 10)); // times 1010 ..= 1100
        assert_eq!(book.between(1_030, 1_060).len(), 3); // 1030, 1040, 1050
        assert!(book.between(2_000, 3_000).is_empty());
        assert!(book.between(1_060, 1_030).is_empty());
    }

    #[test]
    fn the_oldest_trades_go_when_the_book_is_full() {
        let mut book = TradeBook::with_capacity(5);
        book.insert(&run(1, 8));
        assert_eq!(book.len(), 5);
        assert_eq!(book.oldest().unwrap().id, 4);
        assert!(book.has_evicted());
        assert_eq!(book.next_id(), Some(9));
    }

    #[test]
    fn forgetting_by_age_drops_the_old_and_remembers_it_did() {
        let mut book = TradeBook::default();
        book.insert(&run(1, 10));
        book.forget_before(1_050);
        assert_eq!(book.oldest().unwrap().id, 5);
        assert!(book.has_evicted());
    }

    #[test]
    fn a_gap_in_the_ids_moves_the_start_of_what_is_complete() {
        let mut book = TradeBook::default();
        assert_eq!(book.complete_from_ms(), None);
        book.insert(&run(1, 5)); // ids 1..=5
        assert_eq!(book.complete_from_ms(), Some(1_011));
        book.insert(&run(10, 5)); // ids 10..=14; 6..=9 missing
        // Complete only from trade 10 (time 1100), plus the one-millisecond margin.
        assert_eq!(book.complete_from_ms(), Some(1_101));
        assert!(!book.covers_from(1_050));
        assert!(book.covers_from(1_101));
        // Filling the hole joins the runs.
        book.insert(&run(6, 4));
        assert_eq!(book.complete_from_ms(), Some(1_011));
        assert!(book.covers_from(1_050));
    }

    #[test]
    fn a_bar_before_the_first_trade_is_not_covered() {
        let mut book = TradeBook::default();
        book.insert(&run(100, 5));
        assert!(!book.covers_from(0));
        assert!(!book.covers_from(book.oldest().unwrap().time_ms));
        assert!(book.covers_from(book.oldest().unwrap().time_ms + 1));
    }

    #[test]
    fn the_run_cache_is_dropped_when_trades_arrive() {
        let mut book = TradeBook::default();
        book.insert(&run(1, 3));
        assert_eq!(book.complete_from_ms(), Some(1_011));
        book.insert(&run(5, 3)); // gap at 4
        assert_eq!(book.complete_from_ms(), Some(1_051));
    }

    #[test]
    fn pages_of_history_that_are_older_than_everything_held_merge_in_order_without_loss() {
        let mut book = TradeBook::default();
        book.insert(&run(1_000, 500)); // 1000..1500
        // Three older pages, newest first, each handed in reverse to prove the order is not assumed.
        for from in [700u64, 400, 100] {
            let mut page = run(from, 300);
            page.reverse();
            let result = book.insert(&page);
            assert_eq!(result.added, 300, "page {from}");
            assert_eq!(result.duplicates, 0);
        }
        // 100..700 and 1000..1500, with the pages 400..1000 overlapping 700..1000 by design.
        let ids: Vec<u64> = book.trades().iter().map(|t| t.id).collect();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "strictly ascending, no duplicate"
        );
        assert_eq!(ids.first(), Some(&100));
        assert_eq!(ids.last(), Some(&1_499));
        // A page overlapping what is held counts the overlap as duplicates and adds only the rest.
        let result = book.insert(&run(1_400, 200));
        assert_eq!((result.added, result.duplicates), (100, 100));
    }

    #[test]
    fn a_big_newest_first_history_is_fast_enough() {
        // 360 pages of 1000, each older than the last: the old per-trade insert at the front was
        // quadratic. A loose bound that only a quadratic version misses.
        let mut book = TradeBook::with_capacity(1_000_000);
        let started = std::time::Instant::now();
        for page in (0..360u64).rev() {
            book.insert(&run(page * 1_000 + 1, 1_000));
        }
        assert_eq!(book.len(), 360_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_batch_with_its_own_duplicates_and_bad_trades_is_counted_honestly() {
        let mut book = TradeBook::default();
        let mut batch = run(1, 5);
        batch.push(batch[2]);
        batch.push(trade(99, 0, f64::NAN, 1.0, true));
        let result = book.insert(&batch);
        assert_eq!(
            result,
            Inserted {
                added: 5,
                duplicates: 1,
                invalid: 1
            }
        );
        assert_eq!(book.len(), 5);
    }
}
