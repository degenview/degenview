use super::*;
use crate::model::overlay::OverlayList;
use crate::workspace::test_support::candle;
use std::sync::Mutex;

const HOUR: i64 = 3_600_000;

/// A provider that records what it is asked and answers from a table; jobs wait until the test
/// runs them, so a test can change the world between a request and its answer.
#[derive(Default)]
struct Fake {
    jobs: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    calls: Mutex<Vec<String>>,
    answers: Mutex<HashMap<String, Result<Vec<Kline>, String>>>,
    pages: Mutex<HashMap<String, Vec<Kline>>>,
}

impl Fake {
    fn answer(&self, symbol: &str, result: Result<Vec<Kline>, String>) {
        self.answers
            .lock()
            .unwrap()
            .insert(symbol.to_owned(), result);
    }

    fn run_jobs(&self) {
        let jobs: Vec<_> = self.jobs.lock().unwrap().drain(..).collect();
        for job in jobs {
            job();
        }
    }

    fn pending(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl OverlayBackend for Fake {
    fn submit(&self, _: DataSource, job: Box<dyn FnOnce() + Send>) {
        self.jobs.lock().unwrap().push(job);
    }

    fn klines(&self, _: DataSource, symbol: &str, _: &str, _: u32) -> Result<Vec<Kline>, String> {
        self.calls.lock().unwrap().push(format!("klines {symbol}"));
        self.answers
            .lock()
            .unwrap()
            .get(symbol)
            .cloned()
            .unwrap_or_else(|| Ok(vec![candle(10 * HOUR, 1.0), candle(11 * HOUR, 2.0)]))
    }

    fn page(&self, _: DataSource, symbol: &str, _: &str, end: i64) -> Result<Vec<Kline>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("page {symbol} {end}"));
        Ok(self
            .pages
            .lock()
            .unwrap()
            .get(symbol)
            .cloned()
            .unwrap_or_default())
    }
}

fn hub() -> (OverlayHub, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    (OverlayHub::with_backend(fake.clone()), fake)
}

fn market(symbol: &str) -> InstrumentId {
    InstrumentId::new(DataSource::Binance, symbol)
}

fn overlays(symbols: &[&str]) -> OverlayList {
    let main = market("BTCUSDT");
    let mut list = OverlayList::new();
    for symbol in symbols {
        list.add(&main, market(symbol), None);
    }
    list
}

/// One frame: the hub is told the chart's overlays, then polled. Jobs are not run.
fn frame(hub: &mut OverlayHub, chart: u64, list: &OverlayList, now: Instant) {
    hub.sync_chart(chart, list, Some(10 * HOUR));
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
}

#[test]
fn one_market_on_two_charts_is_fetched_once() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    frame(&mut hub, 1, &list, now);
    frame(&mut hub, 2, &list, now);
    assert_eq!(hub.series_count(), 1);
    assert_eq!(fake.pending(), 1);
    fake.run_jobs();
    assert_eq!(fake.calls(), ["klines ETHUSDT"]);
}

#[test]
fn a_market_is_dropped_when_its_last_holder_goes_and_a_pending_job_does_nothing() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    frame(&mut hub, 1, &list, now);
    frame(&mut hub, 2, &list, now);
    hub.forget_chart(1);
    assert_eq!(hub.series_count(), 1, "chart 2 still holds it");
    hub.forget_chart(2);
    assert_eq!(hub.series_count(), 0);
    // The request was queued before the overlay was removed: it must not reach the provider.
    fake.run_jobs();
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

#[test]
fn removing_an_overlay_while_its_request_is_in_flight_is_safe() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    frame(&mut hub, 1, &overlays(&["ETHUSDT", "SOLUSDT"]), now);
    // The user removes ETH before the answers arrive.
    frame(&mut hub, 1, &overlays(&["SOLUSDT"]), now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(hub.series_count(), 1);
    assert!(hub.feed(&market("ETHUSDT")).is_none());
    assert_eq!(
        hub.feed(&market("SOLUSDT")).unwrap().status,
        OverlayStatus::Ready
    );
}

#[test]
fn a_market_that_stays_keeps_its_candles_when_the_chart_changes() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    frame(&mut hub, 1, &overlays(&["ETHUSDT"]), now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(hub.feed(&market("ETHUSDT")).unwrap().candles.len(), 2);
    // Adding a second overlay re-syncs the chart; ETH must not be refetched from nothing.
    frame(&mut hub, 1, &overlays(&["ETHUSDT", "SOLUSDT"]), now);
    assert_eq!(hub.feed(&market("ETHUSDT")).unwrap().candles.len(), 2);
}

#[test]
fn only_a_few_requests_run_at_once_and_the_rest_wait_their_turn() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let symbols: Vec<String> = (0..10).map(|n| format!("S{n}USDT")).collect();
    let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let list = overlays(&refs);
    frame(&mut hub, 1, &list, now);
    assert_eq!(fake.pending(), MAX_IN_FLIGHT);
    let queued = symbols
        .iter()
        .filter(|s| hub.feed(&market(s)).unwrap().status == OverlayStatus::Queued)
        .count();
    assert_eq!(queued, 10 - MAX_IN_FLIGHT);
    // Answers free the slots; the next frame starts the next ones. Nothing is dropped.
    for _ in 0..5 {
        fake.run_jobs();
        frame(&mut hub, 1, &list, now);
    }
    assert!(
        symbols
            .iter()
            .all(|s| hub.feed(&market(s)).unwrap().status == OverlayStatus::Ready)
    );
    assert_eq!(hub.series_count(), 10);
}

#[test]
fn hidden_overlays_are_not_fetched_until_shown() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let mut list = overlays(&["ETHUSDT"]);
    let id = list.as_slice()[0].id;
    list.set_visible(id, false);
    frame(&mut hub, 1, &list, now);
    assert_eq!(fake.pending(), 0);
    assert!(hub.binance_symbols().is_empty());
    list.set_visible(id, true);
    frame(&mut hub, 1, &list, now);
    assert_eq!(fake.pending(), 1);
    assert_eq!(hub.binance_symbols(), ["ETHUSDT"]);
}

#[test]
fn one_failing_market_does_not_touch_the_others_and_is_retried_later() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    fake.answer("BADUSDT", Err("no such market".into()));
    let list = overlays(&["BADUSDT", "ETHUSDT"]);
    frame(&mut hub, 1, &list, now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(
        hub.feed(&market("BADUSDT")).unwrap().status,
        OverlayStatus::Error("no such market".into())
    );
    assert_eq!(
        hub.feed(&market("ETHUSDT")).unwrap().status,
        OverlayStatus::Ready
    );

    // Not hammered: nothing before the back-off passes.
    hub.poll(
        Timeframe::Hour,
        &egui::Context::default(),
        now + Duration::from_secs(6),
    );
    assert!(fake.calls().iter().filter(|c| c.contains("BAD")).count() == 1);
    // After it, the failing market is asked again.
    fake.answer("BADUSDT", Ok(vec![candle(10 * HOUR, 3.0)]));
    let later = now + RETRY_AFTER + Duration::from_secs(1);
    hub.poll(Timeframe::Hour, &egui::Context::default(), later);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), later);
    assert_eq!(
        hub.feed(&market("BADUSDT")).unwrap().status,
        OverlayStatus::Ready
    );
}

#[test]
fn refreshing_slows_down_as_more_markets_are_active() {
    assert_eq!(
        refresh_after(DataSource::Binance, 1),
        Duration::from_secs(5)
    );
    assert_eq!(
        refresh_after(DataSource::Binance, 100),
        Duration::from_secs(25)
    );
    assert_eq!(
        refresh_after(DataSource::Binance, 1_000),
        Duration::from_secs(250)
    );
    assert_eq!(
        refresh_after(DataSource::CoinGecko, 1),
        Duration::from_secs(15)
    );
}

#[test]
fn a_market_is_refreshed_only_after_its_interval() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    frame(&mut hub, 1, &list, now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    hub.poll(
        Timeframe::Hour,
        &egui::Context::default(),
        now + Duration::from_secs(2),
    );
    assert_eq!(fake.pending(), 0);
    hub.poll(
        Timeframe::Hour,
        &egui::Context::default(),
        now + Duration::from_secs(6),
    );
    assert_eq!(fake.pending(), 1);
}

#[test]
fn a_new_timeframe_discards_the_candles_and_a_late_answer_for_the_old_one() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    frame(&mut hub, 1, &list, now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(hub.feed(&market("ETHUSDT")).unwrap().candles.len(), 2);

    // Switch to daily while a refresh is queued for hourly.
    let later = now + Duration::from_secs(10);
    hub.poll(Timeframe::Hour, &egui::Context::default(), later);
    assert_eq!(fake.pending(), 1);
    hub.poll(Timeframe::Day, &egui::Context::default(), later);
    let feed = hub.feed(&market("ETHUSDT")).unwrap();
    assert!(feed.candles.is_empty());
    assert_eq!(
        feed.status,
        OverlayStatus::Loading,
        "the daily request was started"
    );
    fake.run_jobs();
    // Only the daily request reached the provider; the stale hourly job did nothing.
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| *c == "klines ETHUSDT")
            .count(),
        2
    );
    hub.poll(Timeframe::Day, &egui::Context::default(), later);
    assert_eq!(
        hub.feed(&market("ETHUSDT")).unwrap().status,
        OverlayStatus::Ready
    );
}

#[test]
fn a_live_candle_updates_every_holder_of_the_symbol_and_nothing_else() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    frame(&mut hub, 1, &overlays(&["ETHUSDT", "SOLUSDT"]), now);
    frame(&mut hub, 2, &overlays(&["ETHUSDT"]), now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    let before = hub.version();
    let sol_before = hub.feed(&market("SOLUSDT")).unwrap();

    // The forming bar is replaced; a new bar is appended.
    hub.apply_live("ETHUSDT", &candle(11 * HOUR, 5.0));
    let eth = hub.feed(&market("ETHUSDT")).unwrap();
    assert_eq!(eth.candles.last().unwrap().close, 5.0);
    assert_eq!(eth.candles.len(), 2);
    assert_eq!(eth.structure, 1, "the tail changed, not the history");
    hub.apply_live("ethusdt", &candle(12 * HOUR, 6.0));
    assert_eq!(hub.feed(&market("ETHUSDT")).unwrap().candles.len(), 3);
    assert!(hub.version() > before);

    let sol_after = hub.feed(&market("SOLUSDT")).unwrap();
    assert_eq!(sol_after.revision, sol_before.revision);
    // A bar older than the newest is ignored.
    hub.apply_live("ETHUSDT", &candle(5 * HOUR, 9.0));
    assert_eq!(hub.feed(&market("ETHUSDT")).unwrap().candles.len(), 3);
}

#[test]
fn a_refresh_does_not_undo_a_newer_live_candle() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    frame(&mut hub, 1, &list, now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    hub.apply_live("ETHUSDT", &candle(11 * HOUR, 7.5));
    // The REST answer is older than the live bar.
    fake.answer(
        "ETHUSDT",
        Ok(vec![candle(10 * HOUR, 1.0), candle(11 * HOUR, 2.0)]),
    );
    let later = now + Duration::from_secs(6);
    hub.poll(Timeframe::Hour, &egui::Context::default(), later);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), later);
    assert_eq!(
        hub.feed(&market("ETHUSDT"))
            .unwrap()
            .candles
            .last()
            .unwrap()
            .close,
        7.5
    );
}

#[test]
fn history_is_paged_back_toward_the_charts_own_oldest_candle_and_stops() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    // The chart holds candles from hour 2; the overlay's first is hour 10.
    hub.sync_chart(1, &list, Some(2 * HOUR));
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    hub.sync_chart(1, &list, Some(2 * HOUR));
    fake.pages.lock().unwrap().insert(
        "ETHUSDT".into(),
        (2..10).map(|n| candle(n * HOUR, 1.0)).collect(),
    );
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(fake.pending(), 1);
    let structure = hub.feed(&market("ETHUSDT")).unwrap().structure;
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    let feed = hub.feed(&market("ETHUSDT")).unwrap();
    assert_eq!(feed.candles.first().unwrap().open_time, 2 * HOUR);
    assert!(feed.structure > structure);
    assert!(
        fake.calls()
            .contains(&format!("page ETHUSDT {}", 10 * HOUR))
    );
    // It reaches the chart's oldest candle: no more pages are asked for.
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(fake.pending(), 0);
}

#[test]
fn a_market_with_no_older_history_stops_asking() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT"]);
    hub.sync_chart(1, &list, Some(0));
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    fake.run_jobs();
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    fake.run_jobs(); // the page: empty, so the provider has nothing older
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    hub.poll(Timeframe::Hour, &egui::Context::default(), now);
    assert_eq!(fake.pending(), 0);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| c.starts_with("page"))
            .count(),
        1
    );
}

#[test]
fn telling_the_hub_the_same_overlays_again_changes_nothing() {
    let (mut hub, _fake) = hub();
    let now = Instant::now();
    let list = overlays(&["ETHUSDT", "SOLUSDT"]);
    frame(&mut hub, 1, &list, now);
    let version = hub.version();
    hub.sync_chart(1, &list, Some(10 * HOUR));
    assert_eq!(hub.version(), version);
}

#[test]
fn hundreds_of_markets_are_held_without_a_cap() {
    let (mut hub, fake) = hub();
    let now = Instant::now();
    let symbols: Vec<String> = (0..300).map(|n| format!("S{n}USDT")).collect();
    let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let list = overlays(&refs);
    frame(&mut hub, 1, &list, now);
    assert_eq!(hub.series_count(), 300);
    assert_eq!(fake.pending(), MAX_IN_FLIGHT);
    hub.forget_chart(1);
    assert_eq!(hub.series_count(), 0);
}
