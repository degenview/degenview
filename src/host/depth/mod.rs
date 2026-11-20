//! The order-book feeds' owner, shared by every window: it opens the socket of each market a panel
//! asks about while one is drawn, closes it when none is, and hands each frame the latest
//! [`Ladder`] without blocking or recomputing it when nothing changed. The feeds' lifetimes are a
//! [`LeaseMap`], one entry per market.

use crate::clock::now_ms;
use crate::host::lease_map::{LeaseMap, Memo, Running};
use crate::lock::lock;
use crate::model::depth::{DepthMarket, DepthRecord, Ladder, View};
use crate::service::depth::DepthFeed;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A ladder's pulling and stacking roll off as time passes, so the panel looks again this often
/// even when the book is quiet.
const REFRESH: Duration = Duration::from_secs(1);

/// What a ladder was made for: the record's revision, the second and the view.
type Made = (u64, i64, View);

type Feeds<F> = LeaseMap<DepthMarket, DepthRecord, F, Memo<Made, Ladder>>;

/// A cheap handle: clones share the feeds.
#[derive(Clone)]
pub struct DepthHost {
    feeds: Arc<Mutex<Feeds<DepthFeed>>>,
}

impl Running for DepthFeed {
    fn is_running(&self) -> bool {
        DepthFeed::is_running(self)
    }
}

impl Default for DepthHost {
    fn default() -> Self {
        Self {
            feeds: Arc::new(Mutex::new(LeaseMap::default())),
        }
    }
}

impl DepthHost {
    /// Called by a panel each frame it is drawn: keeps the socket of `market` open (opening it,
    /// with an empty record, if it is not) and returns its ladder for `view`.
    pub fn watch(&self, ctx: &egui::Context, market: &DepthMarket, view: View) -> Arc<Ladder> {
        let mut feeds = lock(&self.feeds);
        let now = now_ms();
        let entry = feeds.entry_for(
            market,
            now,
            || DepthRecord::new(market.venue.exchange),
            |record, lease| {
                let repaint = {
                    let ctx = ctx.clone();
                    Arc::new(move || ctx.request_repaint())
                };
                DepthFeed::start(market.clone(), Arc::clone(record), lease.clone(), repaint)
            },
        );
        ctx.request_repaint_after(REFRESH);
        ladder(&entry.record, &mut entry.cache, view, now)
    }
}

/// The ladder for `view` as of `now_ms`, reused while the record's revision, the second and the
/// view hold.
fn ladder(
    record: &Mutex<DepthRecord>,
    cache: &mut Memo<Made, Ladder>,
    view: View,
    now_ms: i64,
) -> Arc<Ladder> {
    let record = lock(record);
    cache.get_or((record.revision(), now_ms / 1_000, view), || {
        record.ladder(view, now_ms)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::depth::{BookEvent, Event, Levels, Link, Price, Snapshot};
    use crate::model::exchange::Exchange;

    const NOW: i64 = 5_000_000_000;

    fn price(text: &str) -> Price {
        Price::parse(text).unwrap()
    }

    fn live_record() -> Mutex<DepthRecord> {
        let mut record = DepthRecord::new(Exchange::Bybit);
        record.set_tick(price("0.01"));
        record.link(true);
        record.apply(
            Event::Book(BookEvent::Snapshot(Snapshot {
                id: 1,
                levels: Levels {
                    bids: vec![(price("99"), 1.0)],
                    asks: vec![(price("101"), 1.0)],
                },
            })),
            NOW,
        );
        Mutex::new(record)
    }

    #[test]
    fn a_ladder_is_reused_until_the_book_the_second_or_the_view_changes() {
        let record = live_record();
        let mut cache = Memo::default();
        let view = View::default();
        let first = ladder(&record, &mut cache, view, NOW);
        assert_eq!(first.link, Link::Live);
        assert!(Arc::ptr_eq(
            &first,
            &ladder(&record, &mut cache, view, NOW + 500)
        ));

        let later = ladder(&record, &mut cache, view, NOW + 1_000);
        assert!(!Arc::ptr_eq(&first, &later));

        let grouped = View {
            group: Some(price("0.05")),
            centre: None,
        };
        let regrouped = ladder(&record, &mut cache, grouped, NOW + 1_000);
        assert!(!Arc::ptr_eq(&later, &regrouped));
        assert_eq!(regrouped.group, price("0.05"));

        lock(&record).link(false);
        let dropped = ladder(&record, &mut cache, grouped, NOW + 1_000);
        assert!(!Arc::ptr_eq(&regrouped, &dropped));
        assert_eq!(dropped.link, Link::Reconnecting);
    }
}
