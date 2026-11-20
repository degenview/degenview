//! The order-book feeds' owner, shared by every window: it opens the socket of each market a panel
//! asks about while one is drawn, closes it when none is, and hands each frame the latest
//! [`DepthSnapshot`] without blocking or recomputing it when nothing changed. The feeds' lifetimes are a
//! [`LeaseMap`], one entry per market.

use crate::clock::now_ms;
use crate::host::lease_map::{LeaseMap, Memo, Running};
use crate::lock::lock;
use crate::model::depth::{DepthMarket, DepthRecord, DepthSnapshot, View};
use crate::service::depth::{DepthFeed, new_record};
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A ladder's pulling and stacking roll off as time passes, so the panel looks again this often
/// even when the book is quiet.
const REFRESH: Duration = Duration::from_secs(1);

/// What a snapshot was made for: the record's revision, the second and the view.
type Made = (u64, i64, View);

type Feeds<F> = LeaseMap<DepthMarket, DepthRecord, F, Memo<Made, DepthSnapshot>>;

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
    /// with an empty record, if it is not) and returns its depth for `view`.
    pub fn watch(
        &self,
        ctx: &egui::Context,
        market: &DepthMarket,
        view: View,
    ) -> Arc<DepthSnapshot> {
        let mut feeds = lock(&self.feeds);
        let now = now_ms();
        let entry = feeds.entry_for(
            market,
            now,
            || new_record(market),
            |record, lease| {
                let repaint = {
                    let ctx = ctx.clone();
                    Arc::new(move || ctx.request_repaint())
                };
                DepthFeed::start(market.clone(), Arc::clone(record), lease.clone(), repaint)
            },
        );
        ctx.request_repaint_after(REFRESH);
        snapshot(&entry.record, &mut entry.cache, view, now)
    }
}

/// The snapshot for `view` as of `now_ms`, reused while the record's revision, the second and the
/// view hold.
fn snapshot(
    record: &Mutex<DepthRecord>,
    cache: &mut Memo<Made, DepthSnapshot>,
    view: View,
    now_ms: i64,
) -> Arc<DepthSnapshot> {
    let record = lock(record);
    cache.get_or((record.revision(), now_ms / 1_000, view), || {
        record.snapshot(view, now_ms)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::depth::{BookEvent, Event, Levels, Link, Price, Snapshot, SnapshotAndDeltas};

    const NOW: i64 = 5_000_000_000;

    fn price(text: &str) -> Price {
        Price::parse(text).unwrap()
    }

    fn live_record() -> Mutex<DepthRecord> {
        let mut record = DepthRecord::new(Box::new(SnapshotAndDeltas::default()));
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
    fn a_snapshot_is_reused_until_the_book_the_second_or_the_view_changes() {
        let record = live_record();
        let mut cache = Memo::default();
        let view = View {
            group: None,
            centre: None,
            rows: 30,
        };
        let first = snapshot(&record, &mut cache, view, NOW);
        assert_eq!(first.link, Link::Live);
        assert!(Arc::ptr_eq(
            &first,
            &snapshot(&record, &mut cache, view, NOW + 500)
        ));

        let later = snapshot(&record, &mut cache, view, NOW + 1_000);
        assert!(!Arc::ptr_eq(&first, &later));

        let grouped = View {
            group: Some(price("0.05")),
            ..view
        };
        let regrouped = snapshot(&record, &mut cache, grouped, NOW + 1_000);
        assert!(!Arc::ptr_eq(&later, &regrouped));
        assert_eq!(regrouped.ladder.group, price("0.05"));

        let taller = View {
            rows: 31,
            ..grouped
        };
        let resized = snapshot(&record, &mut cache, taller, NOW + 1_000);
        assert!(!Arc::ptr_eq(&regrouped, &resized));
        assert_eq!(resized.ladder.rows.len(), 31);

        lock(&record).link(false);
        let dropped = snapshot(&record, &mut cache, taller, NOW + 1_000);
        assert!(!Arc::ptr_eq(&resized, &dropped));
        assert_eq!(dropped.link, Link::Reconnecting);
    }
}
