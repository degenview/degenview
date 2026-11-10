//! The app-wide Trade Journal: the trades built from the journal's executions, the user's reviews
//! and the saved table preferences. Per-tab presentation is `journal_tab`.
//!
//! Trades are derived data. They are rebuilt on a worker thread whenever the paper ledger changes
//! (the host is told the ledger's revision each frame; reading SQLite is not done per frame) and
//! the result replaces the old one only once complete, so a frame never waits on a rebuild. A
//! rebuild that started before a newer change is followed by another.
//!
//! The host never touches paper trading: it reads the journal's own tables.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::{Deserialize, Serialize};

use crate::service::work_queue;
use crate::store::SharedStore;
use degenview_journal::daily::DailyEntry;
use degenview_journal::filters::RangePreset;
use degenview_journal::model::{CostBasis, Execution, JournalAccount, Trade};
use degenview_journal::playbook::Playbook;
use degenview_journal::reconstruct::reconstruct;
use degenview_journal::reviews::Review;
use degenview_journal::table::{Column, Sort};

const PREFS_KEY: &str = "journal.prefs";

/// Everything a view reads about the journal's trades. Immutable once built.
#[derive(Clone, Debug, Default)]
pub struct JournalData {
    /// Counts builds, so a view can tell when its cached rows are stale.
    pub build_id: u64,
    pub accounts: Vec<JournalAccount>,
    /// Oldest open first.
    pub trades: Vec<Trade>,
    pub executions: usize,
    /// Why the last attempt to copy paper fills into the journal failed, if it did.
    pub ingest_error: Option<String>,
}

impl JournalData {
    pub fn account(&self, uid: &str) -> Option<&JournalAccount> {
        self.accounts.iter().find(|account| account.uid == uid)
    }

    pub fn account_names(&self) -> HashMap<String, String> {
        self.accounts
            .iter()
            .map(|account| (account.uid.clone(), account.name.clone()))
            .collect()
    }

    /// Distinct markets with trades, as `(market key, label)`, sorted by label.
    pub fn markets(&self) -> Vec<(String, String)> {
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for trade in &self.trades {
            seen.entry(trade.instrument.market_key.clone())
                .or_insert_with(|| trade.instrument.display_name.clone());
        }
        let mut markets: Vec<_> = seen.into_iter().collect();
        markets.sort_by_key(|market| market.1.to_lowercase());
        markets
    }
}

/// Builds the data from stored rows. Pure; a row that no longer decodes fails the build, because
/// leaving one out would change a trade's result.
pub fn build_data(
    payloads: &[String],
    accounts: Vec<JournalAccount>,
    build_id: u64,
    ingest_error: Option<String>,
) -> Result<JournalData, String> {
    let mut executions = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let execution: Execution = serde_json::from_str(payload)
            .map_err(|error| format!("A stored execution could not be read: {error}"))?;
        executions.push(execution);
    }
    let basis: HashMap<&str, CostBasis> = accounts
        .iter()
        .map(|account| (account.uid.as_str(), account.cost_basis))
        .collect();
    let trades = reconstruct(&executions, |uid| {
        basis.get(uid).copied().unwrap_or_default()
    });
    Ok(JournalData {
        build_id,
        accounts,
        trades,
        executions: executions.len(),
        ingest_error,
    })
}

/// The pages of the Journal tab.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    #[default]
    Overview,
    Calendar,
    Trades,
    Analytics,
    Compare,
    Playbooks,
    Daily,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeChoice {
    Preset(RangePreset),
    /// Local dates `YYYY-MM-DD`, either end open when empty.
    Custom {
        from: String,
        to: String,
    },
}

/// What the journal remembers about how the user left it. Local to this Mac.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JournalPrefs {
    #[serde(default)]
    pub section: Section,
    #[serde(default = "default_columns")]
    pub columns: Vec<Column>,
    #[serde(default)]
    pub widths: BTreeMap<Column, f32>,
    #[serde(default)]
    pub sort: Sort,
    #[serde(default = "default_range")]
    pub range: RangeChoice,
    /// The account shown; `None` is all accounts.
    #[serde(default)]
    pub account: Option<String>,
}

fn default_columns() -> Vec<Column> {
    Column::ALL
        .into_iter()
        .filter(|column| column.default_visible())
        .collect()
}

fn default_range() -> RangeChoice {
    RangeChoice::Preset(RangePreset::All)
}

impl Default for JournalPrefs {
    fn default() -> Self {
        Self {
            section: Section::default(),
            columns: default_columns(),
            widths: BTreeMap::new(),
            sort: Sort::default(),
            range: default_range(),
            account: None,
        }
    }
}

impl JournalPrefs {
    pub fn width(&self, column: Column) -> f32 {
        self.widths
            .get(&column)
            .copied()
            .unwrap_or_else(|| column.default_width())
            .max(column.min_width())
    }

    /// The columns in table order, whatever order they were switched on in.
    pub fn visible(&self) -> Vec<Column> {
        Column::ALL
            .into_iter()
            .filter(|column| self.columns.contains(column))
            .collect()
    }
}

struct Built {
    data: Result<JournalData, String>,
    reviews: Vec<Review>,
    playbooks: Vec<Playbook>,
    days: Vec<DailyEntry>,
}

pub struct JournalHost {
    store: SharedStore,
    data: Option<Arc<JournalData>>,
    load_error: Option<String>,
    reviews: Arc<HashMap<String, Review>>,
    playbooks: Arc<Vec<Playbook>>,
    playbook_names: Arc<HashMap<String, String>>,
    days: Arc<HashMap<String, DailyEntry>>,
    /// Moves whenever a review, a playbook or a daily entry changes: what the user wrote.
    reviews_revision: u64,
    build_id: u64,
    last_paper_revision: Option<u64>,
    dirty: bool,
    in_flight: bool,
    sender: Sender<Built>,
    receiver: Receiver<Built>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    prefs: JournalPrefs,
    prefs_saved: JournalPrefs,
}

impl JournalHost {
    pub fn new(store: SharedStore, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        let prefs = store
            .lock()
            .ok()
            .and_then(|store| store.setting::<JournalPrefs>(PREFS_KEY))
            .unwrap_or_default();
        let (sender, receiver) = channel();
        Self {
            store,
            data: None,
            load_error: None,
            reviews: Arc::new(HashMap::new()),
            playbooks: Arc::new(Vec::new()),
            playbook_names: Arc::new(HashMap::new()),
            days: Arc::new(HashMap::new()),
            reviews_revision: 0,
            build_id: 0,
            last_paper_revision: None,
            dirty: true,
            in_flight: false,
            sender,
            receiver,
            repaint,
            prefs_saved: prefs.clone(),
            prefs,
        }
    }

    /// Tells the host the paper ledger is at `paper_revision`; a change starts a rebuild. Cheap
    /// enough for every frame.
    pub fn sync(&mut self, paper_revision: u64) {
        if self.last_paper_revision != Some(paper_revision) {
            self.last_paper_revision = Some(paper_revision);
            self.dirty = true;
        }
        self.start_if_wanted();
    }

    /// Something other than the paper ledger changed the journal's rows (an import).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn start_if_wanted(&mut self) {
        if !self.dirty || self.in_flight {
            return;
        }
        self.dirty = false;
        self.in_flight = true;
        self.build_id += 1;
        let build_id = self.build_id;
        let store = self.store.clone();
        let sender = self.sender.clone();
        let repaint = self.repaint.clone();
        work_queue::submit(move || {
            let built = read_and_build(&store, build_id);
            let _ = sender.send(built);
            repaint();
        });
    }

    /// Takes in a finished rebuild, if there is one. Cheap enough for every frame.
    pub fn poll(&mut self) {
        let mut latest = None;
        while let Ok(built) = self.receiver.try_recv() {
            latest = Some(built);
        }
        let Some(built) = latest else { return };
        self.in_flight = false;
        self.apply(built);
        self.start_if_wanted();
    }

    fn apply(&mut self, built: Built) {
        match built.data {
            Ok(data) => {
                self.data = Some(Arc::new(data));
                self.load_error = None;
            }
            Err(error) => self.load_error = Some(error),
        }
        self.reviews = Arc::new(
            built
                .reviews
                .into_iter()
                .map(|review| (review.trade_key.clone(), review))
                .collect(),
        );
        self.set_playbooks(built.playbooks);
        self.days = Arc::new(
            built
                .days
                .into_iter()
                .map(|day| (day.date.clone(), day))
                .collect(),
        );
        self.reviews_revision += 1;
    }

    fn set_playbooks(&mut self, playbooks: Vec<Playbook>) {
        self.playbook_names = Arc::new(
            playbooks
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect(),
        );
        self.playbooks = Arc::new(playbooks);
    }

    /// Rebuilds on this thread and waits. For tests.
    #[cfg(test)]
    pub fn rebuild_blocking(&mut self) {
        self.build_id += 1;
        let built = read_and_build(&self.store, self.build_id);
        self.dirty = false;
        self.apply(built);
    }

    pub fn data(&self) -> Option<&Arc<JournalData>> {
        self.data.as_ref()
    }

    #[cfg(test)]
    pub fn is_building(&self) -> bool {
        self.in_flight || self.dirty
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn reviews(&self) -> &Arc<HashMap<String, Review>> {
        &self.reviews
    }

    /// Moves whenever a review, a playbook or a daily entry is saved or everything is reloaded.
    pub fn reviews_revision(&self) -> u64 {
        self.reviews_revision
    }

    pub fn playbooks(&self) -> &Arc<Vec<Playbook>> {
        &self.playbooks
    }

    /// Playbook id to name.
    pub fn playbook_names(&self) -> &Arc<HashMap<String, String>> {
        &self.playbook_names
    }

    pub fn days(&self) -> &Arc<HashMap<String, DailyEntry>> {
        &self.days
    }

    /// Saves a playbook; one with no name is refused with a reason.
    pub fn save_playbook(&mut self, playbook: &Playbook) -> Result<Playbook, String> {
        let saved = {
            let store = self
                .store
                .lock()
                .map_err(|_| "The journal's store is unavailable.".to_owned())?;
            store
                .journal_save_playbook(playbook, now_ms())
                .map_err(|error| format!("The playbook could not be saved: {error}"))?
        }
        .ok_or_else(|| "A playbook needs a name.".to_owned())?;
        let mut all: Vec<Playbook> = self.playbooks.iter().cloned().collect();
        match all.iter_mut().find(|p| p.id == saved.id) {
            Some(existing) => *existing = saved.clone(),
            None => all.push(saved.clone()),
        }
        all.sort_by_key(|p| (p.name.to_lowercase(), p.id.clone()));
        self.set_playbooks(all);
        self.reviews_revision += 1;
        Ok(saved)
    }

    pub fn delete_playbook(&mut self, id: &str) -> Result<(), String> {
        {
            let store = self
                .store
                .lock()
                .map_err(|_| "The journal's store is unavailable.".to_owned())?;
            store
                .journal_delete_playbook(id)
                .map_err(|error| format!("The playbook could not be deleted: {error}"))?;
        }
        let all: Vec<Playbook> = self
            .playbooks
            .iter()
            .filter(|p| p.id != id)
            .cloned()
            .collect();
        self.set_playbooks(all);
        self.reviews_revision += 1;
        Ok(())
    }

    /// Saves one day's entry. An entry with nothing in it is removed.
    pub fn save_day(&mut self, entry: &DailyEntry) -> Result<(), String> {
        let saved = {
            let store = self
                .store
                .lock()
                .map_err(|_| "The journal's store is unavailable.".to_owned())?;
            store
                .journal_save_day(entry, now_ms())
                .map_err(|error| format!("The entry could not be saved: {error}"))?
        };
        let days = Arc::make_mut(&mut self.days);
        match saved {
            Some(saved) => days.insert(saved.date.clone(), saved),
            None => days.remove(&entry.date),
        };
        self.reviews_revision += 1;
        Ok(())
    }

    /// A fresh playbook id.
    pub fn new_playbook_id() -> String {
        crate::model::watchlist::new_id().to_string()
    }

    /// Saves one review. An emptied review is removed. A review never changes a trade, an order or
    /// a balance.
    /// The store, for work that runs off the UI thread (reading a picture).
    pub fn store(&self) -> SharedStore {
        self.store.clone()
    }

    pub fn save_review(&mut self, review: &Review) -> Result<(), String> {
        let saved = {
            let store = self
                .store
                .lock()
                .map_err(|_| "The journal's store is unavailable.".to_owned())?;
            store
                .journal_save_review(review, now_ms())
                .map_err(|error| format!("The review could not be saved: {error}"))?
        };
        let reviews = Arc::make_mut(&mut self.reviews);
        match saved {
            Some(saved) => reviews.insert(saved.trade_key.clone(), saved),
            None => reviews.remove(&review.trade_key),
        };
        self.reviews_revision += 1;
        Ok(())
    }

    pub fn prefs(&self) -> &JournalPrefs {
        &self.prefs
    }

    pub fn prefs_mut(&mut self) -> &mut JournalPrefs {
        &mut self.prefs
    }

    /// `flush_prefs` unless the pointer is down (a column is being dragged).
    pub fn flush_prefs_if_idle(&mut self, ctx: &eframe::egui::Context) {
        if !ctx.input(|input| input.pointer.any_down()) {
            self.flush_prefs();
        }
    }

    /// Writes the preferences if they changed. Called when the user lets go of a control, not
    /// while a drag is in progress.
    pub fn flush_prefs(&mut self) {
        if self.prefs == self.prefs_saved {
            return;
        }
        let saved = self
            .store
            .lock()
            .ok()
            .is_some_and(|store| store.set_setting(PREFS_KEY, &self.prefs).is_ok());
        if saved {
            self.prefs_saved = self.prefs.clone();
        }
    }
}

fn read_and_build(store: &SharedStore, build_id: u64) -> Built {
    // Hold the store only for the reads; decoding and reconstruction happen after it is released.
    let read = store
        .lock()
        .map_err(|_| "The journal's store is unavailable.".to_owned())
        .and_then(|store| {
            let payloads = store
                .journal_execution_payloads()
                .map_err(|error| error.to_string())?;
            let accounts = store
                .journal_accounts()
                .map_err(|error| error.to_string())?;
            let reviews = store.journal_reviews().map_err(|error| error.to_string())?;
            let playbooks = store
                .journal_playbooks()
                .map_err(|error| error.to_string())?;
            let days = store.journal_days().map_err(|error| error.to_string())?;
            Ok((
                payloads,
                accounts,
                reviews,
                playbooks,
                days,
                store.journal_last_error(),
            ))
        });
    match read {
        Ok((payloads, accounts, reviews, playbooks, days, ingest_error)) => Built {
            data: build_data(&payloads, accounts, build_id, ingest_error),
            reviews,
            playbooks,
            days,
        },
        Err(error) => Built {
            data: Err(format!("The journal could not be read: {error}")),
            reviews: Vec::new(),
            playbooks: Vec::new(),
            days: Vec::new(),
        },
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperAccountSettings, PaperCurrency, PaperInstrument, PaperOrderRequest, PaperOrderSide,
        PaperOrderType, PaperQuote, PaperTimeInForce,
    };
    use crate::store::Store;
    use degenview_journal::model::TradeStatus;
    use std::sync::Mutex;

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    fn host(store: &SharedStore) -> JournalHost {
        JournalHost::new(store.clone(), Arc::new(|| {}))
    }

    fn trade_through(store: &SharedStore) {
        let mut paper = crate::host::paper_host::PaperHost::load(store.clone());
        let id = paper
            .create_account(
                "Main",
                PaperCurrency::Usd,
                100_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        let instrument = PaperInstrument::chart("BTCUSDT", "BTCUSDT", DataSource::Binance);
        let request = |side| PaperOrderRequest {
            account_id: id,
            instrument: instrument.clone(),
            side,
            kind: PaperOrderType::Market,
            quantity: 1.0,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        };
        let quote = |last: f64| PaperQuote {
            instrument_key: instrument.key.clone(),
            bid: None,
            ask: None,
            last: Some(last),
            timestamp: now_ms(),
            is_market_open: true,
        };
        paper.process_quote(quote(100.0)).unwrap();
        paper.submit(request(PaperOrderSide::Buy)).unwrap();
        paper.process_quote(quote(110.0)).unwrap();
        paper.submit(request(PaperOrderSide::Sell)).unwrap();
    }

    #[test]
    fn a_paper_trade_becomes_a_journal_trade() {
        let store = store();
        trade_through(&store);
        let mut host = host(&store);
        host.rebuild_blocking();
        let data = host.data().unwrap();
        assert_eq!(data.accounts.len(), 1);
        assert_eq!(data.executions, 2);
        assert_eq!(data.trades.len(), 1);
        assert_eq!(data.trades[0].status, TradeStatus::Closed);
        assert_eq!(
            data.markets(),
            [("BINANCE:BTCUSDT".to_owned(), "BTCUSDT".to_owned())]
        );
        assert_eq!(data.account_names().values().next().unwrap(), "Main");
        assert!(host.load_error().is_none());
    }

    #[test]
    fn an_empty_journal_builds_an_empty_result_not_an_error() {
        let store = store();
        let mut host = host(&store);
        host.rebuild_blocking();
        let data = host.data().unwrap();
        assert!(data.trades.is_empty() && data.accounts.is_empty());
    }

    #[test]
    fn the_worker_builds_off_thread_and_poll_takes_it_in() {
        let store = store();
        trade_through(&store);
        let mut host = host(&store);
        assert!(host.data().is_none());
        host.sync(1);
        assert!(host.is_building());
        let started = std::time::Instant::now();
        while host.data().is_none() && started.elapsed() < std::time::Duration::from_secs(10) {
            host.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(host.data().unwrap().trades.len(), 1);
        assert!(!host.is_building());
        // The same revision starts nothing; a new one does.
        host.sync(1);
        assert!(!host.is_building());
        host.sync(2);
        assert!(host.is_building());
    }

    #[test]
    fn a_row_that_cannot_be_read_fails_the_build_instead_of_vanishing() {
        let store = store();
        trade_through(&store);
        store
            .lock()
            .unwrap()
            .raw_connection()
            .execute(
                "UPDATE journal_execution SET payload = 'not json' WHERE seq = 1",
                [],
            )
            .unwrap();
        let mut host = host(&store);
        host.rebuild_blocking();
        assert!(host.data().is_none());
        assert!(host.load_error().unwrap().contains("could not be read"));
    }

    #[test]
    fn a_failed_rebuild_keeps_the_last_good_data() {
        let store = store();
        trade_through(&store);
        let mut host = host(&store);
        host.rebuild_blocking();
        store
            .lock()
            .unwrap()
            .raw_connection()
            .execute("UPDATE journal_execution SET payload = 'not json'", [])
            .unwrap();
        host.rebuild_blocking();
        assert_eq!(
            host.data().unwrap().trades.len(),
            1,
            "the old trades stay on screen"
        );
        assert!(host.load_error().is_some());
    }

    #[test]
    fn reviews_save_reload_and_vanish_when_emptied() {
        let store = store();
        trade_through(&store);
        let mut host = host(&store);
        host.rebuild_blocking();
        let key = host.data().unwrap().trades[0].key.clone();
        let mut review = Review::new(&key, &host.data().unwrap().trades[0].account_uid);
        review.notes = "Good exit.".into();
        let before = host.reviews_revision();
        host.save_review(&review).unwrap();
        assert!(host.reviews_revision() > before);
        assert_eq!(host.reviews()[&key].notes, "Good exit.");

        // A fresh host (a restart) reads it back.
        let mut again = JournalHost::new(store.clone(), Arc::new(|| {}));
        again.rebuild_blocking();
        assert_eq!(again.reviews()[&key].notes, "Good exit.");

        let empty = Review::new(&key, &review.account_uid);
        host.save_review(&empty).unwrap();
        assert!(host.reviews().is_empty());
    }

    #[test]
    fn saving_a_review_changes_nothing_about_paper_trading() {
        let store = store();
        trade_through(&store);
        let paper_before = store.lock().unwrap().paper_trading().unwrap();
        let mut host = host(&store);
        host.rebuild_blocking();
        let trade = host.data().unwrap().trades[0].clone();
        let mut review = Review::new(&trade.key, &trade.account_uid);
        review.notes = "x".into();
        review.stop = Some("1".parse().unwrap());
        host.save_review(&review).unwrap();
        assert_eq!(store.lock().unwrap().paper_trading().unwrap(), paper_before);
    }

    #[test]
    fn playbooks_and_days_save_and_reload() {
        let store = store();
        let mut host = host(&store);
        host.rebuild_blocking();
        let mut playbook = Playbook::new(JournalHost::new_playbook_id(), "Breakout");
        playbook.checklist = vec!["Volume".into()];
        let before = host.reviews_revision();
        let saved = host.save_playbook(&playbook).unwrap();
        assert!(host.reviews_revision() > before);
        assert_eq!(host.playbook_names()[&saved.id], "Breakout");
        assert!(host.save_playbook(&Playbook::new("x".into(), " ")).is_err());

        let mut day = DailyEntry::new("2026-11-09");
        day.plan = "Trade the open.".into();
        host.save_day(&day).unwrap();
        assert_eq!(host.days()["2026-11-09"].plan, "Trade the open.");

        let mut again = JournalHost::new(store.clone(), Arc::new(|| {}));
        again.rebuild_blocking();
        assert_eq!(again.playbooks().len(), 1);
        assert_eq!(again.days().len(), 1);

        host.delete_playbook(&saved.id).unwrap();
        assert!(host.playbooks().is_empty() && host.playbook_names().is_empty());
        host.save_day(&DailyEntry::new("2026-11-09")).unwrap();
        assert!(host.days().is_empty());
    }

    #[test]
    fn preferences_persist_only_when_they_change() {
        let store = store();
        let mut host = host(&store);
        assert_eq!(host.prefs(), &JournalPrefs::default());
        host.flush_prefs();
        assert!(
            store
                .lock()
                .unwrap()
                .setting::<JournalPrefs>(PREFS_KEY)
                .is_none()
        );

        host.prefs_mut().sort = Sort {
            column: Column::Net,
            descending: false,
        };
        host.prefs_mut().widths.insert(Column::Symbol, 200.0);
        host.prefs_mut().account = Some("paper:1".into());
        host.flush_prefs();
        let again = JournalHost::new(store.clone(), Arc::new(|| {}));
        assert_eq!(again.prefs().sort.column, Column::Net);
        assert_eq!(again.prefs().width(Column::Symbol), 200.0);
        assert_eq!(again.prefs().account.as_deref(), Some("paper:1"));
    }

    #[test]
    fn widths_never_go_below_a_readable_minimum_and_columns_keep_table_order() {
        let mut prefs = JournalPrefs::default();
        prefs.widths.insert(Column::Net, 1.0);
        assert_eq!(prefs.width(Column::Net), Column::Net.min_width());
        prefs.columns = vec![Column::Review, Column::Open];
        assert_eq!(prefs.visible(), [Column::Open, Column::Review]);
    }

    #[test]
    fn old_preference_rows_decode_with_defaults() {
        let prefs: JournalPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs, JournalPrefs::default());
    }
}
