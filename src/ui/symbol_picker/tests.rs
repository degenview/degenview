use super::*;
use crate::model::watchlist_quote::Freshness;
use crate::ui::render_tests::{Screen, gallery_dir};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn pair(source: DataSource, symbol: &str, base: &str, quote: &str) -> SearchResult {
    SearchResult::new(source, symbol, symbol).pair(base, quote)
}

/// Canned provider answers: what each exchange lists for a handful of queries.
fn stub_search(source: DataSource, query: &str) -> Result<Vec<SearchResult>, String> {
    use DataSource::{Alpaca, Binance, CoinGecko, Coinbase, DexScreener};
    match (source, query) {
        (Binance, "btc") => Ok(vec![
            pair(Binance, "BTCTRY", "BTC", "TRY"),
            pair(Binance, "BTCFDUSD", "BTC", "FDUSD"),
            pair(Binance, "BTCUSDC", "BTC", "USDC"),
            pair(Binance, "BTCUSDT", "BTC", "USDT"),
            pair(Binance, "ETHBTC", "ETH", "BTC"),
        ]),
        (Coinbase, "btc") => Ok(vec![
            pair(Coinbase, "BTC-EUR", "BTC", "EUR"),
            pair(Coinbase, "BTC-USDT", "BTC", "USDT"),
            pair(Coinbase, "BTC-USD", "BTC", "USD"),
        ]),
        (DexScreener, "btc") => Ok(vec![
            pair(DexScreener, "WBTC/WETH", "WBTC", "WETH"),
            pair(DexScreener, "BTC/USDC", "BTC", "USDC"),
        ]),
        (CoinGecko, "btc") => Ok(vec![SearchResult::new(CoinGecko, "BTC", "bitcoin")]),
        (Alpaca, "btc") => Ok(vec![SearchResult::new(
            Alpaca,
            "BTC — Grayscale Bitcoin Mini Trust",
            "BTC",
        )]),
        (Binance, "btcusdt") => Ok(vec![pair(Binance, "BTCUSDT", "BTC", "USDT")]),
        (Alpaca, "aapl") => Ok(vec![SearchResult::new(Alpaca, "AAPL — Apple Inc.", "AAPL")]),
        (Binance, "eth") => Ok(vec![
            pair(Binance, "ETHUSDT", "ETH", "USDT"),
            pair(Binance, "ETHBTC", "ETH", "BTC"),
        ]),
        (Coinbase, "eth") => Ok(vec![pair(Coinbase, "ETH-USD", "ETH", "USD")]),
        (Alpaca, _) => Err(service::ALPACA_MISSING_KEY.to_owned()),
        _ => Ok(Vec::new()),
    }
}

fn stub_related(asset: &str) -> Vec<SearchResult> {
    if asset == "BTC" {
        vec![
            pair(DataSource::Binance, "BTCUSDT", "BTC", "USDT"),
            pair(DataSource::Binance, "BTCUSDC", "BTC", "USDC"),
            pair(DataSource::Binance, "BTCUSDS", "BTC", "USDS"),
            pair(DataSource::Binance, "BTCFDUSD", "BTC", "FDUSD"),
            pair(DataSource::Coinbase, "BTC-USD", "BTC", "USD"),
            pair(DataSource::Coinbase, "BTC-USDT", "BTC", "USDT"),
            pair(DataSource::Coinbase, "BTC-USDC", "BTC", "USDC"),
        ]
    } else {
        Vec::new()
    }
}

const STUB: Backend = Backend {
    search: stub_search,
    related: stub_related,
};

fn press(key: Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn quote(last: f64, change: f64, percent: f64, freshness: Freshness) -> WatchlistQuote {
    WatchlistQuote {
        last: Some(last),
        change: Some(change),
        change_percent: Some(percent),
        freshness,
        ..WatchlistQuote::default()
    }
}

struct Rig {
    screen: Screen,
    picker: SymbolPicker,
    store: SharedStore,
    on_tab: Vec<InstrumentId>,
    quote: Option<WatchlistQuote>,
    replacing: Option<String>,
}

impl Rig {
    /// A picker opened on `query`, with the answers in.
    fn new(width: f32, query: &str) -> Self {
        Self::sized(width, 760.0, query)
    }

    fn sized(width: f32, height: f32, query: &str) -> Self {
        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let mut picker = SymbolPicker::with_backend(store.clone(), STUB);
        picker.open(query);
        let screen = Screen::new(vec2(width, height));
        egui_extras::install_image_loaders(&screen.ctx);
        let mut rig = Self {
            screen,
            picker,
            store,
            on_tab: Vec::new(),
            quote: None,
            replacing: Some("ETHUSDT".to_owned()),
        };
        rig.settle();
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<Output> {
        self.frame_with_output(events).0
    }

    fn frame_with_output(
        &mut self,
        events: Vec<egui::Event>,
    ) -> (Option<Output>, egui::FullOutput) {
        let mut found = None;
        let picker = &mut self.picker;
        let env = Env {
            on_tab: &self.on_tab,
            quote: self.quote.as_ref(),
            replacing: self.replacing.as_deref(),
        };
        let output = self.screen.run(events, |ui| {
            if let Some(output) = picker.show(ui.ctx(), &env) {
                found = Some(output);
            }
        });
        (found, output)
    }

    /// Runs frames until the searches have answered.
    fn settle(&mut self) {
        for _ in 0..40 {
            self.frame(Vec::new());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> Option<Output> {
        let output = self.frame(vec![press(key, modifiers)]);
        self.frame(Vec::new());
        output
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    /// The captions in the list.
    fn heads(&mut self) -> Vec<String> {
        let ctx = self.screen.ctx.clone();
        let listing = self.picker.listing(&ctx);
        self.picker
            .rows(&listing)
            .heads
            .into_iter()
            .map(|(_, head)| head)
            .collect()
    }

    /// What the list holds now, as the exchanges name the markets.
    fn names(&mut self) -> Vec<String> {
        let ctx = self.screen.ctx.clone();
        let listing = self.picker.listing(&ctx);
        self.picker
            .rows(&listing)
            .results
            .into_iter()
            .map(|result| result.symbol)
            .collect()
    }
}

#[test]
fn it_opens_searching_for_the_current_market_with_the_text_selected() {
    let mut rig = Rig::new(1100.0, "BTCUSDT");
    assert!(rig.picker.is_open());
    assert_eq!(rig.picker.query(), "BTCUSDT");
    assert_eq!(rig.names(), ["BTCUSDT"]);
    assert!(rig.screen.ctx.memory(|memory| memory.focused()).is_some());
    // The first thing typed replaces the whole ticker.
    rig.type_text("btc");
    assert_eq!(rig.picker.query(), "btc");
}

#[test]
fn every_exchanges_dollar_markets_of_the_asset_come_before_the_rest() {
    let mut rig = Rig::new(1100.0, "btc");
    let names = rig.names();
    let at = |name: &str| {
        names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("{name} is listed"))
    };
    let dollars = [
        "BTCUSDT",
        "BTCUSDC",
        "BTCFDUSD",
        "BTC-USD",
        "BTC-USDT",
        "BTC/USDC",
        "BTC",
        "BTC — Grayscale Bitcoin Mini Trust",
    ];
    assert_eq!(names[..dollars.len()], dollars);
    for odd in ["BTCTRY", "BTC-EUR", "ETHBTC", "WBTC/WETH"] {
        assert!(
            at(odd) >= dollars.len(),
            "{odd} comes after the dollar markets"
        );
    }
    // Other quotes of the asset itself come before lookalikes that merely contain it.
    assert!(at("BTCTRY") < at("ETHBTC"));
    assert!(at("BTC-EUR") < at("WBTC/WETH"));
}

#[test]
fn the_arrows_move_the_highlight_and_wrap() {
    let mut rig = Rig::new(1100.0, "btc");
    assert_eq!(rig.picker.highlight, 0);
    rig.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(rig.picker.highlight, 1);
    rig.key(Key::ArrowUp, Modifiers::NONE);
    rig.key(Key::ArrowUp, Modifiers::NONE);
    let last = rig.names().len() - 1;
    assert_eq!(rig.picker.highlight, last);
    rig.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(rig.picker.highlight, 0);
}

#[test]
fn return_replaces_with_the_highlighted_market_and_closes() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::Replace(result)) = output else {
        panic!("expected a replace, got {output:?}");
    };
    assert_eq!(result.symbol, "BTCUSDC");
    assert!(!rig.picker.is_open());
    let recents = add_chart_sheet::load_recents(&rig.store.lock().unwrap());
    assert_eq!(recents[0].full_symbol, "BTCUSDC");
}

#[test]
fn shift_return_adds_instead_of_replacing() {
    let mut rig = Rig::new(1100.0, "btc");
    let output = rig.frame(vec![press(Key::Enter, Modifiers::SHIFT)]);
    assert!(matches!(output, Some(Output::Add(result)) if result.symbol == "BTCUSDT"));
    assert!(!rig.picker.is_open());
}

#[test]
fn tab_walks_the_market_tabs_and_each_narrows_the_list() {
    let mut rig = Rig::new(1100.0, "btc");
    assert_eq!(rig.picker.scope, Scope::All);
    rig.key(Key::Tab, Modifiers::NONE);
    rig.settle();
    assert_eq!(rig.picker.scope, Scope::Stocks);
    assert_eq!(rig.names(), ["BTC — Grayscale Bitcoin Mini Trust"]);
    rig.key(Key::Tab, Modifiers::NONE);
    rig.settle();
    assert_eq!(rig.picker.scope, Scope::Crypto);
    assert!(!rig.names().iter().any(|name| name.contains("Grayscale")));
    rig.key(Key::Tab, Modifiers::NONE);
    assert_eq!(rig.picker.scope, Scope::Watchlists);
    rig.key(Key::Tab, Modifiers::NONE);
    assert_eq!(rig.picker.scope, Scope::All);
}

#[test]
fn a_venue_narrows_the_list_to_one_exchange() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.picker.set_venue(Some(DataSource::Coinbase));
    rig.settle();
    assert_eq!(rig.names(), ["BTC-USD", "BTC-USDT", "BTC-EUR"]);
    // A venue the next scope does not list is dropped with it.
    rig.picker.set_scope(Scope::Stocks);
    assert_eq!(rig.picker.venue, None);
}

#[test]
fn a_stock_needs_an_alpaca_key_before_it_opens() {
    let mut rig = Rig::new(1100.0, "aapl");
    // Tests run with no key saved.
    assert_eq!(rig.names(), ["AAPL — Apple Inc."]);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(output.is_none());
    assert!(rig.picker.is_open());
    assert!(rig.picker.needs_alpaca);
}

#[test]
fn a_missing_alpaca_key_is_a_card_with_a_settings_button_not_a_hint() {
    let mut rig = Rig::new(1100.0, "eth");
    let ctx = rig.screen.ctx.clone();
    let listing = rig.picker.listing(&ctx);
    let notice = rig
        .picker
        .notice(&listing)
        .expect("stocks cannot be searched");
    assert_eq!(notice.title, "Stocks need an Alpaca key");
    assert!(notice.settings);
    // Another failure of the stock search has nothing to set up.
    let mut listing = rig.picker.listing(&ctx);
    listing.errors = vec![(DataSource::Alpaca, "Network error: timed out".to_owned())];
    let notice = rig.picker.notice(&listing).expect("a failure");
    assert_eq!(notice.title, "Stocks could not be searched");
    assert!(!notice.settings);
    // Nothing to say while stocks are not in the scope.
    rig.picker.set_scope(Scope::Crypto);
    rig.settle();
    let listing = rig.picker.listing(&ctx);
    assert!(rig.picker.notice(&listing).is_none());
}

fn lists() -> Vec<WatchlistGroup> {
    vec![
        WatchlistGroup {
            name: "Favorites".into(),
            markets: vec![
                pair(DataSource::Binance, "BTCUSDT", "BTC", "USDT"),
                pair(DataSource::Coinbase, "ETH-USD", "ETH", "USD"),
            ],
        },
        WatchlistGroup {
            name: "Stocks".into(),
            markets: vec![SearchResult::new(
                DataSource::Alpaca,
                "AAPL — Apple Inc.",
                "AAPL",
            )],
        },
    ]
}

#[test]
fn the_watchlists_tab_lists_each_list_in_order_and_filters_without_searching() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.set_scope(Scope::Watchlists);
    assert!(rig.picker.wants_watchlists());
    assert!(rig.names().is_empty(), "nothing before the lists arrive");
    rig.picker.set_watchlists(lists());
    assert_eq!(rig.names(), ["BTCUSDT", "ETH-USD", "AAPL — Apple Inc."]);
    assert_eq!(rig.heads(), ["Favorites", "Stocks"]);
    // The box filters what the lists hold; a list with no match loses its caption.
    rig.type_text("eth");
    assert_eq!(rig.names(), ["ETH-USD"]);
    assert_eq!(rig.heads(), ["Favorites"]);
    // A venue narrows it too.
    rig.picker.query.clear();
    rig.picker.set_venue(Some(DataSource::Alpaca));
    assert_eq!(rig.names(), ["AAPL — Apple Inc."]);
    // Return opens the highlighted market.
    rig.picker.set_venue(None);
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(matches!(output, Some(Output::Replace(result)) if result.symbol == "ETH-USD"));
}

#[test]
fn the_watchlists_tab_is_the_last_tab_and_tab_wraps_to_all() {
    let mut rig = Rig::new(1100.0, "btc");
    for expected in [Scope::Stocks, Scope::Crypto, Scope::Watchlists, Scope::All] {
        rig.key(Key::Tab, Modifiers::NONE);
        assert_eq!(rig.picker.scope, expected);
    }
}

#[test]
fn escape_closes() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.key(Key::Escape, Modifiers::NONE);
    assert!(!rig.picker.is_open());
    assert!(rig.picker.highlighted().is_none());
}

fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        })
        .into()
}

fn right_click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        })
        .into()
}

/// Right-clicks the second row, then clicks `offset` below the pointer, where the menu's rows are.
fn pick_from_the_row_menu(rig: &mut Rig, offset: f32) -> Option<Output> {
    // The list starts at y = 167 in a 1100 x 760 window; a row is 56 tall.
    let row = pos2(300.0, 167.0 + 56.0 + 28.0);
    rig.frame(vec![egui::Event::PointerMoved(row)]);
    rig.frame(right_click_at(row));
    rig.settle();
    let item = row + vec2(24.0, offset);
    rig.frame(vec![egui::Event::PointerMoved(item)]);
    let output = rig.frame(click_at(item));
    rig.frame(Vec::new());
    output
}

#[test]
fn right_clicking_a_row_offers_replace_and_add_as_new_chart() {
    let mut rig = Rig::new(1100.0, "btc");
    let output = pick_from_the_row_menu(&mut rig, 18.0);
    let Some(Output::Replace(result)) = output else {
        panic!("the first item replaces, got {output:?}");
    };
    assert_eq!(result.symbol, "BTCUSDC", "the row that was right-clicked");
    assert!(!rig.picker.is_open());

    let mut rig = Rig::new(1100.0, "btc");
    let output = pick_from_the_row_menu(&mut rig, 18.0 + 24.0);
    assert!(
        matches!(&output, Some(Output::Add(result)) if result.symbol == "BTCUSDC"),
        "the second item adds, got {output:?}"
    );
}

#[test]
fn with_no_chart_to_replace_the_menu_only_adds() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.replacing = None;
    let output = pick_from_the_row_menu(&mut rig, 18.0);
    assert!(matches!(&output, Some(Output::Add(result)) if result.symbol == "BTCUSDC"));
}

#[test]
fn a_click_outside_the_sheet_closes_it_and_a_click_inside_does_not() {
    let mut rig = Rig::new(1100.0, "btc");
    // The sheet is centred: its empty footer is inside it and the window's corner is not.
    rig.frame(vec![egui::Event::PointerMoved(pos2(950.0, 690.0))]);
    rig.frame(click_at(pos2(950.0, 690.0)));
    rig.frame(Vec::new());
    assert!(rig.picker.is_open(), "a click in the sheet leaves it open");
    rig.frame(vec![egui::Event::PointerMoved(pos2(6.0, 6.0))]);
    rig.frame(click_at(pos2(6.0, 6.0)));
    rig.frame(Vec::new());
    assert!(!rig.picker.is_open(), "a click outside closes it");
}

#[test]
fn an_empty_box_lists_the_recent_picks_in_scope() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    rig.picker.open("");
    rig.settle();
    assert_eq!(rig.names(), ["BTCUSDT"]);
    assert_eq!(rig.heads(), ["Recent"]);
    // Nothing recent in this scope: the popular stocks stand in.
    rig.picker.set_scope(Scope::Stocks);
    assert_eq!(rig.heads(), ["Popular"]);
    assert_eq!(rig.names()[0], "AAPL");
}

#[test]
fn before_anything_is_picked_the_popular_markets_are_listed() {
    let mut rig = Rig::new(1100.0, "");
    assert_eq!(rig.heads(), ["Popular"]);
    let names = rig.names();
    assert_eq!(names[0], "BTCUSDT");
    assert!(names.contains(&"AAPL".to_owned()));
    rig.picker.set_scope(Scope::Crypto);
    assert!(rig.names().iter().all(|name| name.ends_with("USDT")));
    // Return opens the first of them.
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(matches!(output, Some(Output::Replace(result)) if result.symbol == "BTCUSDT"));
}

#[test]
fn the_highlighted_market_is_the_one_the_workspace_prices() {
    let mut rig = Rig::new(1100.0, "btc");
    let market = rig.picker.highlighted().expect("a row is highlighted");
    assert_eq!(
        market.key(),
        InstrumentId::new(DataSource::Binance, "BTCUSDT").key()
    );
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let next = rig.picker.highlighted().expect("another row");
    assert_ne!(market.key(), next.key());
}

#[test]
fn it_draws_at_every_size_with_and_without_the_detail_panel() {
    for width in [480.0, 700.0, 780.0, 1100.0] {
        let mut rig = Rig::new(width, "btc");
        rig.quote = Some(quote(76_075.70, 208.12, 0.27, Freshness::Live));
        rig.on_tab = vec![InstrumentId::new(DataSource::Binance, "BTCUSDT")];
        rig.settle();
        rig.picker.set_scope(Scope::Stocks);
        rig.settle();
    }
}

/// The picker as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_symbol_picker() {
    if gallery_dir().is_none() {
        return;
    }
    let bitcoin = quote(76_075.70, 208.12, 0.27, Freshness::Live);
    let apple = quote(228.40, 1.15, 0.51, Freshness::Live);
    for (name, width, query, scope, price) in [
        ("symbol-picker-all", 1100.0, "btc", Scope::All, &bitcoin),
        (
            "symbol-picker-stocks",
            1100.0,
            "aapl",
            Scope::Stocks,
            &apple,
        ),
        ("symbol-picker-narrow", 640.0, "btc", Scope::All, &bitcoin),
        ("symbol-picker-notice", 1100.0, "eth", Scope::All, &bitcoin),
        (
            "symbol-picker-watchlists",
            1100.0,
            "",
            Scope::Watchlists,
            &bitcoin,
        ),
        ("symbol-picker-short", 1100.0, "btc", Scope::All, &bitcoin),
        ("symbol-picker-none", 1100.0, "zzzz", Scope::All, &bitcoin),
        ("symbol-picker-empty", 1100.0, "", Scope::All, &bitcoin),
    ] {
        let height = if name.ends_with("short") {
            560.0
        } else {
            760.0
        };
        let mut rig = Rig::sized(width, height, query);
        rig.quote = Some(price.clone());
        rig.on_tab = vec![InstrumentId::new(DataSource::Binance, "BTCUSDT")];
        rig.picker.set_watchlists(lists());
        rig.picker.set_scope(scope);
        rig.settle();
        let (_, output) = rig.frame_with_output(Vec::new());
        rig.screen.save(name, output);
    }

    // A row's right-click menu.
    let mut rig = Rig::new(1100.0, "btc");
    rig.quote = Some(bitcoin.clone());
    let row = pos2(300.0, 167.0 + 56.0 + 28.0);
    rig.frame(vec![egui::Event::PointerMoved(row)]);
    rig.frame(right_click_at(row));
    rig.settle();
    let (_, output) = rig.frame_with_output(Vec::new());
    rig.screen.save("symbol-picker-row-menu", output);
}
