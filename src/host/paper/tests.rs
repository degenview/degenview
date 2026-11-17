use super::*;
use crate::model::DataSource;
use crate::model::paper::{PaperInstrument, PaperOrderStatus};
use crate::store::Store;
use std::sync::{Arc, Mutex};

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
}

fn account(host: &mut PaperHost) -> u64 {
    host.create_account(
        "Main",
        PaperCurrency::Usd,
        100_000.0,
        PaperAccountSettings::default(),
    )
    .expect("account")
}

fn market_request(account_id: u64) -> PaperOrderRequest {
    PaperOrderRequest {
        account_id,
        instrument: PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance),
        side: PaperOrderSide::Buy,
        kind: PaperOrderType::Market,
        quantity: 1.0,
        limit_price: None,
        stop_price: None,
        time_in_force: PaperTimeInForce::GoodTilCanceled,
        take_profit: None,
        stop_loss: None,
    }
}

#[test]
fn a_tick_that_fills_nothing_changes_no_list_and_saves_nothing() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let account_id = account(&mut host);
    let key = "Binance:BTCUSDT";
    let tick = |last: f64, timestamp: i64| PaperQuote {
        instrument_key: key.to_owned(),
        bid: None,
        ask: None,
        last: Some(last),
        timestamp,
        is_market_open: true,
    };
    let now = crate::model::watchlist::now_ms();
    let before = host.revision();
    host.process_quote(tick(100.0, now)).unwrap();
    host.process_quote(tick(101.0, now + 1)).unwrap();
    assert_eq!(
        host.revision(),
        before,
        "no order to evaluate: only the mark moves"
    );
    assert_eq!(host.snapshot().quotes[key].last, Some(101.0));
    assert!(
        store
            .lock()
            .unwrap()
            .paper_trading()
            .unwrap()
            .quotes
            .is_empty(),
        "a bare tick is not saved"
    );

    // With a resting order the tick is evaluated, but while nothing fills nothing is saved
    // or counted either.
    let mut request = market_request(account_id);
    request.kind = PaperOrderType::Limit;
    request.limit_price = Some(50.0);
    host.submit(request).unwrap();
    let after_submit = host.revision();
    host.process_quote(tick(102.0, now + 2)).unwrap();
    assert_eq!(host.revision(), after_submit);
    assert_eq!(host.snapshot().quotes[key].last, Some(102.0));

    // A tick that fills it is saved and counted.
    host.process_quote(tick(40.0, now + 3)).unwrap();
    assert!(host.revision() > after_submit);
    assert_eq!(host.snapshot().positions.len(), 1);
    assert_eq!(
        store
            .lock()
            .unwrap()
            .paper_trading()
            .unwrap()
            .positions
            .len(),
        1
    );
}

#[test]
fn data_that_disagrees_with_itself_is_not_loaded() {
    let store = store();
    {
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.submit(market_request(id)).ok();
    }
    {
        let guard = store.lock().unwrap();
        let mut snapshot = guard.paper_trading().unwrap();
        snapshot.accounts[0].id = 424_242;
        guard.save_paper_trading(&snapshot).unwrap();
    }
    let mut host = PaperHost::load(store);
    assert!(host.load_failed(), "orders now point at a missing account");
    assert!(matches!(
        host.last_error(),
        Some(PaperHostError::Integrity(_))
    ));
    assert!(host.submit(market_request(424_242)).is_err());
}

/// `cargo test --lib paper_host_cost -- --ignored --nocapture`
#[test]
#[ignore = "timing"]
fn paper_host_cost() {
    use std::time::Instant;
    for fills in [100usize, 1_000, 5_000] {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        let key = "Binance:BTCUSDT";
        let now = crate::model::watchlist::now_ms();
        let tick = |last: f64, n: i64| PaperQuote {
            instrument_key: key.to_owned(),
            bid: Some(last - 0.5),
            ask: Some(last + 0.5),
            last: Some(last),
            timestamp: now + n,
            is_market_open: true,
        };
        host.process_quote(tick(100.0, 0)).unwrap();
        // Build the history inside one engine, then save it once.
        let mut engine = PaperTradingEngine::new(host.snapshot().clone());
        for index in 0..fills {
            let mut request = market_request(id);
            request.side = if index % 2 == 0 {
                PaperOrderSide::Buy
            } else {
                PaperOrderSide::Sell
            };
            engine.submit(request).unwrap();
        }
        store
            .lock()
            .unwrap()
            .save_paper_trading(engine.snapshot())
            .unwrap();
        let mut host = PaperHost::load(store.clone());
        host.process_quote(tick(100.0, 1_000_000)).unwrap();

        let started = Instant::now();
        for n in 0..100 {
            host.process_quote(tick(100.0 + n as f64, 2_000_000 + n))
                .unwrap();
        }
        let bare_tick = started.elapsed() / 100;

        let mut request = market_request(id);
        request.kind = PaperOrderType::Limit;
        request.limit_price = Some(50.0);
        let started = Instant::now();
        host.submit(request).unwrap();
        let submit = started.elapsed();

        let started = Instant::now();
        for n in 0..10 {
            host.process_quote(tick(100.0 + n as f64, 3_000_000 + n))
                .unwrap();
        }
        let evaluated_tick = started.elapsed() / 10;

        let started = Instant::now();
        let view = crate::ui::paper::panel::PanelView::build(&host);
        let build_view = started.elapsed();
        println!(
            "{fills:>5} fills: bare tick {bare_tick:?}, tick with a working order {evaluated_tick:?}, submit {submit:?}, panel view build {build_view:?} ({} events)",
            view.events.len()
        );
    }
}

#[test]
fn tracks_only_instruments_with_a_working_order_or_a_position() {
    let mut host = PaperHost::load(store());
    let account_id = account(&mut host);
    let instrument = PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance);
    assert!(!host.tracks(&instrument.key));

    host.process_quote(PaperQuote {
        instrument_key: instrument.key.clone(),
        bid: None,
        ask: None,
        last: Some(100.0),
        timestamp: crate::model::watchlist::now_ms(),
        is_market_open: true,
    })
    .unwrap();
    let mut request = market_request(account_id);
    request.kind = PaperOrderType::Limit;
    request.limit_price = Some(50.0);
    let order = host.submit(request).unwrap();
    assert!(host.tracks(&instrument.key), "a resting order is tracked");
    assert!(!host.tracks("binance:ETHUSDT"));

    host.cancel(order).unwrap();
    assert!(!host.tracks(&instrument.key));

    host.submit(market_request(account_id)).unwrap();
    assert!(host.tracks(&instrument.key), "an open position is tracked");
}

#[test]
fn the_note_for_a_mirrored_account_names_the_mac_or_says_it_is_paused() {
    let named = Elsewhere {
        name: "Studio".into(),
        paused: false,
    };
    assert_eq!(
        owner_text(&named),
        "Running on Studio. Placing an order here takes it over."
    );
    let unknown = Elsewhere {
        name: String::new(),
        paused: false,
    };
    assert!(owner_text(&unknown).starts_with("Running on another Mac."));
    let paused = Elsewhere {
        name: String::new(),
        paused: true,
    };
    assert!(owner_text(&paused).starts_with("Paused"));
}

#[test]
fn reload_picks_up_what_sync_wrote_and_keeps_live_quotes() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let id = host
        .create_account(
            "Mine",
            PaperCurrency::Usd,
            1_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    host.process_quote(PaperQuote {
        instrument_key: "Binance:BTCUSDT".into(),
        bid: Some(1.0),
        ask: Some(2.0),
        last: Some(1.5),
        timestamp: 5,
        is_market_open: true,
    })
    .unwrap();
    {
        let guard = store.lock().unwrap();
        let mut snapshot = guard.paper_trading().unwrap();
        snapshot
            .accounts
            .push(crate::model::paper::PaperAccount::new(77, "From iCloud", 1));
        snapshot.selected_account_id = Some(999);
        guard.save_paper_trading(&snapshot).unwrap();
        guard.set_paper_owner("77", Some("other-mac")).unwrap();
    }
    host.reload();
    assert_eq!(host.snapshot().accounts.len(), 2);
    assert_eq!(host.snapshot().selected_account_id, Some(id));
    assert!(host.snapshot().quotes.contains_key("Binance:BTCUSDT"));
    assert!(host.owned_elsewhere(77).is_some());
    assert!(host.owned_elsewhere(id).is_none());
}

#[test]
fn account_mutations_are_durable() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let first = account(&mut host);
    let second = host
        .create_account(
            "Second",
            PaperCurrency::Eur,
            50_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    host.select_account(first).unwrap();
    host.delete_account(first).unwrap();

    assert_eq!(host.snapshot().selected_account_id, Some(second));
    assert_eq!(
        store.lock().unwrap().paper_trading().unwrap(),
        *host.snapshot()
    );
}

#[test]
fn default_account_is_created_once_and_persisted() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let id = host.ensure_default_account().unwrap().expect("new account");
    assert_eq!(host.ensure_default_account().unwrap(), None);
    assert_eq!(host.snapshot().selected_account_id, Some(id));
    assert_eq!(host.snapshot().accounts[0].name, "Paper Trading");
    assert_eq!(
        store
            .lock()
            .unwrap()
            .paper_trading()
            .unwrap()
            .accounts
            .len(),
        1
    );
}

#[test]
fn a_rejected_order_is_saved_before_its_error_is_returned() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let id = account(&mut host);
    let error = host.submit(market_request(id)).expect_err("no quote");
    assert!(matches!(
        error,
        PaperHostError::Engine(PaperTradingError::NoMarketData)
    ));
    assert_eq!(host.snapshot().orders.len(), 1);
    assert_eq!(host.snapshot().orders[0].status, PaperOrderStatus::Rejected);
    assert_eq!(
        store.lock().unwrap().paper_trading().unwrap().orders[0].status,
        PaperOrderStatus::Rejected
    );
}

#[test]
fn quotes_and_fills_persist_together() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let id = account(&mut host);
    host.process_quote(PaperQuote {
        instrument_key: "Binance:BTCUSDT".into(),
        bid: Some(49_999.0),
        ask: Some(50_001.0),
        last: Some(50_000.0),
        timestamp: now_ms(),
        is_market_open: true,
    })
    .unwrap();
    host.submit(market_request(id)).unwrap();

    assert_eq!(host.snapshot().positions.len(), 1);
    assert_eq!(host.snapshot().fills.len(), 1);
    assert_eq!(
        store.lock().unwrap().paper_trading().unwrap(),
        *host.snapshot()
    );
}

#[test]
fn a_quote_that_crosses_a_working_order_persists_the_fill() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    let id = account(&mut host);
    let initial_timestamp = now_ms();
    host.process_quote(PaperQuote {
        instrument_key: "Binance:BTCUSDT".into(),
        bid: None,
        ask: None,
        last: Some(50_000.0),
        timestamp: initial_timestamp,
        is_market_open: true,
    })
    .unwrap();
    let mut request = market_request(id);
    request.kind = PaperOrderType::Limit;
    request.limit_price = Some(40_000.0);
    host.submit(request).expect("working limit");
    assert!(host.snapshot().orders[0].status.is_working());

    host.process_quote(PaperQuote {
        instrument_key: "Binance:BTCUSDT".into(),
        bid: None,
        ask: None,
        last: Some(39_000.0),
        timestamp: now_ms() + 1_000,
        is_market_open: true,
    })
    .expect("crossing quote");

    assert_eq!(host.snapshot().orders[0].status, PaperOrderStatus::Filled);
    assert_eq!(
        store.lock().unwrap().paper_trading().unwrap(),
        *host.snapshot()
    );
}

#[test]
fn marks_use_the_executable_side_and_returns_use_entry_value() {
    let store = store();
    let mut host = PaperHost::load(store);
    let id = account(&mut host);
    host.process_quote(PaperQuote {
        instrument_key: "Binance:BTCUSDT".into(),
        bid: Some(90.0),
        ask: Some(110.0),
        last: Some(100.0),
        timestamp: now_ms(),
        is_market_open: true,
    })
    .unwrap();
    let instrument = PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance);
    let long = PaperPosition {
        account_id: id,
        instrument: instrument.clone(),
        signed_quantity: 2.0,
        average_entry_price: 100.0,
        realized_gross_pnl: 0.0,
        commissions: 0.0,
        opened_at: now_ms(),
        updated_at: now_ms(),
    };
    let short = PaperPosition {
        signed_quantity: -2.0,
        ..long.clone()
    };

    assert_eq!(host.mark(&long), Some(90.0));
    assert_eq!(host.mark(&short), Some(110.0));
    assert_eq!(host.return_ratio(&long), Some(-0.1));
    assert_eq!(host.return_ratio(&short), Some(-0.1));
}

#[test]
fn restored_quotes_are_discarded_as_stale() {
    let store = store();
    let mut host = PaperHost::load(store.clone());
    host.process_quote(PaperQuote {
        instrument_key: "Binance:BTCUSDT".into(),
        bid: Some(49_999.0),
        ask: Some(50_001.0),
        last: Some(50_000.0),
        timestamp: now_ms(),
        is_market_open: true,
    })
    .unwrap();
    assert!(!host.snapshot().quotes.is_empty());
    assert!(
        store
            .lock()
            .unwrap()
            .paper_trading()
            .unwrap()
            .quotes
            .is_empty(),
        "a mark with no working order stays transient"
    );

    let restored = PaperHost::load(store);
    assert!(restored.snapshot().quotes.is_empty());
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

mod fill_sound {
    use super::*;
    use crate::model::paper::prefs::PaperPrefs;
    use crate::service::sound::{Sound, Sounder};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const KEY: &str = "Binance:BTCUSDT";

    #[derive(Default)]
    struct Counting(AtomicUsize);

    impl Counting {
        fn played(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl Sounder for Counting {
        fn play(&self, sound: Sound) {
            assert_eq!(sound, Sound::OrderFilled);
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn quote(last: f64, offset: i64) -> PaperQuote {
        PaperQuote {
            instrument_key: KEY.into(),
            bid: None,
            ask: None,
            last: Some(last),
            timestamp: now_ms() + offset,
            is_market_open: true,
        }
    }

    fn small_market(account_id: u64) -> PaperOrderRequest {
        let mut request = market_request(account_id);
        request.quantity = 0.1;
        request
    }

    fn limit_buy(account_id: u64, price: f64) -> PaperOrderRequest {
        let mut request = market_request(account_id);
        request.kind = PaperOrderType::Limit;
        request.limit_price = Some(price);
        request
    }

    #[test]
    fn a_market_order_counts_one_fill() {
        let mut host = PaperHost::load(store());
        let id = account(&mut host);
        host.process_quote(quote(50_000.0, 0)).unwrap();
        assert_eq!(host.drain_fills(), 0, "a mark is not a fill");
        host.submit(market_request(id)).unwrap();
        assert_eq!(host.drain_fills(), 1);
        assert_eq!(host.drain_fills(), 0, "each fill is taken once");
    }

    #[test]
    fn a_limit_order_counts_once_when_a_quote_crosses_it() {
        let mut host = PaperHost::load(store());
        let id = account(&mut host);
        host.process_quote(quote(50_000.0, 0)).unwrap();
        host.submit(limit_buy(id, 40_000.0)).unwrap();
        assert_eq!(host.drain_fills(), 0, "resting is not filled");
        host.process_quote(quote(45_000.0, 500)).unwrap();
        assert_eq!(host.drain_fills(), 0, "not crossed yet");
        host.process_quote(quote(39_000.0, 1_000)).unwrap();
        assert_eq!(host.drain_fills(), 1);
        host.process_quote(quote(38_000.0, 1_500)).unwrap();
        assert_eq!(host.drain_fills(), 0);
    }

    #[test]
    fn a_reload_of_what_another_mac_filled_counts_nothing() {
        let store = store();
        let mut other = PaperHost::load(store.clone());
        let id = account(&mut other);
        other.process_quote(quote(50_000.0, 0)).unwrap();
        other.submit(market_request(id)).unwrap();
        assert_eq!(other.snapshot().fills.len(), 1);

        let mut host = PaperHost::load(store);
        host.reload();
        assert_eq!(host.snapshot().fills.len(), 1);
        assert_eq!(host.drain_fills(), 0);
    }

    #[test]
    fn an_account_another_mac_runs_is_not_filled_here() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(quote(50_000.0, 0)).unwrap();
        host.submit(limit_buy(id, 40_000.0)).unwrap();
        store
            .lock()
            .unwrap()
            .set_paper_owner(&id.to_string(), Some("other-mac"))
            .unwrap();
        host.reload();
        host.process_quote(quote(39_000.0, 1_000)).unwrap();
        assert_eq!(host.drain_fills(), 0);
        assert!(host.snapshot().orders[0].status.is_working());
    }

    #[test]
    fn the_choice_is_on_by_default_and_remembered() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        assert!(host.prefs().fill_sound);
        let muted = PaperPrefs { fill_sound: false };
        host.set_prefs(&store.lock().unwrap(), muted).unwrap();
        assert_eq!(host.prefs(), muted);
        assert_eq!(PaperHost::load(store).prefs(), muted);
    }

    #[test]
    fn a_burst_of_fills_plays_once_and_a_muted_host_never_plays() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(quote(50_000.0, 0)).unwrap();
        let sounder = Counting::default();

        host.announce_fills(&sounder);
        assert_eq!(sounder.played(), 0, "nothing was filled");

        host.submit(small_market(id)).unwrap();
        host.submit(small_market(id)).unwrap();
        host.announce_fills(&sounder);
        assert_eq!(sounder.played(), 1, "two fills, one sound");
        host.announce_fills(&sounder);
        assert_eq!(sounder.played(), 1);

        host.set_prefs(&store.lock().unwrap(), PaperPrefs { fill_sound: false })
            .unwrap();
        host.submit(small_market(id)).unwrap();
        host.announce_fills(&sounder);
        assert_eq!(sounder.played(), 1, "muted");
        host.set_prefs(&store.lock().unwrap(), PaperPrefs::default())
            .unwrap();
        host.announce_fills(&sounder);
        assert_eq!(
            sounder.played(),
            1,
            "a fill heard-as-muted is not played later"
        );
    }
}
