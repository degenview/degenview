//! Fixtures the workspace modules' tests share.

use super::ChartWorkspace;
use crate::model::{DataSource, Drawing, InstrumentId, Kline, Timeframe, Tool};
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a test lets a background job take before it calls the job hung. A time, never a count
/// of polls: a poll's own cost is the frame it draws, which differs by an order of magnitude
/// between a quiet and a loaded machine. Generous, because a test build's own code is unoptimised
/// (the dev profile optimises dependencies only): the snapshot rasterizer takes ~30 s over a 2x
/// layout export here against ~0.1 s in a release build, and a hung job costs only this long once.
pub(super) const JOB_DEADLINE: Duration = Duration::from_secs(300);

/// Calls `poll` (which advances the frame or the job and says whether it is done) every few
/// milliseconds until it is, or panics saying `what` never happened after [`JOB_DEADLINE`].
pub(super) fn wait_for(what: &str, mut poll: impl FnMut() -> bool) {
    let deadline = Instant::now() + JOB_DEADLINE;
    while Instant::now() < deadline {
        if poll() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("{what} never happened within {JOB_DEADLINE:?}");
}

pub(super) fn candle(open_time: i64, close: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + 59_999,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1.0,
    }
}

pub(super) fn empty_workspace(id: u64) -> ChartWorkspace {
    let store: SharedStore = Arc::new(Mutex::new(
        crate::store::Store::open_in_memory().expect("in-memory store"),
    ));
    let book = WatchlistQuoteBook::default();
    ChartWorkspace::empty(
        id,
        Timeframe::DAY_1,
        Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
        book.clone(),
        WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
        store,
    )
}

pub(super) fn rectangle(id: u64, left: i64) -> Drawing {
    Drawing::build(
        Tool::Rectangle,
        id,
        &[
            crate::model::Anchor {
                time_ms: left,
                price: 1.0,
            },
            crate::model::Anchor {
                time_ms: left + 10,
                price: 2.0,
            },
        ],
        Tool::Rectangle.default_style(),
    )
    .expect("rectangle")
}

pub(super) fn chart_with_drawings(
    workspace: &mut ChartWorkspace,
    symbol: &str,
    ids: &[u64],
) -> u64 {
    let ctx = egui::Context::default();
    let chart_id = workspace
        .add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &ctx)
        .chart_id;
    let chart = workspace
        .charts
        .iter_mut()
        .find(|chart| chart.id == chart_id)
        .expect("chart");
    chart.drawings = ids
        .iter()
        .map(|id| rectangle(*id, *id as i64 * 100))
        .collect();
    chart_id
}

pub(super) fn drawing_ids(workspace: &ChartWorkspace, chart_id: u64) -> Vec<u64> {
    workspace
        .charts
        .iter()
        .find(|chart| chart.id == chart_id)
        .expect("chart")
        .drawings
        .iter()
        .map(Drawing::id)
        .collect()
}

/// A drawing this version cannot read, as an older build holds a newer build's tool.
pub(super) fn unreadable(id: u64) -> Drawing {
    Drawing::Unreadable(serde_json::json!({
        "Shape": {"id": id, "geometry": {"hologram": {"at": id}}}
    }))
}
