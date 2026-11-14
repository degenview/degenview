use super::*;
use crate::model::portfolio::card::CardKind;
use crate::model::watchlist::quote::Freshness;
use crate::service::EventInfo;
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
        (DataSource::Polymarket, "fed") => Ok(vec![
            chance(DataSource::Polymarket, "Fed decision", "Cut", Some(0.2)),
            chance(DataSource::Polymarket, "Fed decision", "Hold", Some(0.7)),
            chance(DataSource::Polymarket, "Fed decision", "Hike", Some(0.1)),
            chance(DataSource::Polymarket, "Rain tomorrow?", "Yes", Some(0.4)),
        ]),
        (DataSource::Kalshi, "fed") => Ok(vec![
            chance(DataSource::Kalshi, "Fed rate", "Above 5%", Some(0.6)),
            chance(DataSource::Kalshi, "Fed rate", "Below 5%", Some(0.3)),
        ]),
        (DataSource::Kalshi, "boom") => Err("Kalshi is down".to_owned()),
        _ => Ok(Vec::new()),
    }
}

/// One choice of a prediction event, as a provider lists it.
fn chance(source: DataSource, event: &str, label: &str, chance: Option<f64>) -> SearchResult {
    SearchResult {
        price: chance,
        event: Some(EventInfo {
            title: event.into(),
            stats: Box::new(if event == "Fed decision" {
                crate::service::EventStats {
                    volume: Some(31_625_167),
                    volume_24h: Some(4_615_988),
                    liquidity: Some(1_834_558),
                    open_interest: Some(5_149_829),
                    ends_at_ms: Some(1_796_000_000_000),
                    category: None,
                    about: Some(
                        "The FED interest rates are defined in this market by the upper bound of the target federal funds range, which the Federal Open Market Committee sets at each of its meetings."
                            .into(),
                    ),
                }
            } else {
                crate::service::EventStats::default()
            }),
            ..EventInfo::default()
        }),
        ..SearchResult::new(source, label, format!("{event}/{label}"))
    }
}

fn stub_trending(source: DataSource, _limit: usize) -> Result<Vec<SearchResult>, String> {
    match source {
        DataSource::Polymarket => Ok(vec![
            chance(source, "Election", "Left", Some(0.45)),
            chance(source, "Election", "Right", Some(0.55)),
            chance(source, "Bitcoin above 100k?", "Yes", Some(0.62)),
        ]),
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
    trending: stub_trending,
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
    overlaid: Vec<InstrumentId>,
    can_overlay: bool,
    cards_on_tab: Vec<PortfolioCard>,
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
            overlaid: Vec::new(),
            can_overlay: false,
            cards_on_tab: Vec::new(),
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
            overlaid: &self.overlaid,
            can_overlay: self.can_overlay,
            cards_on_tab: &self.cards_on_tab,
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
            .items
            .iter()
            .map(item_name)
            .collect()
    }
}

/// What a row is called.
fn item_name(item: &Item) -> String {
    match item {
        Item::Market(result) | Item::Choice(result) => result.symbol.clone(),
        Item::Event(head) => head.title.clone(),
        Item::Tile(tile) => tile.title.clone(),
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
    let recents = recents::load_recents(&rig.store.lock().unwrap());
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
fn option_return_draws_the_market_over_the_chart_and_closes() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.can_overlay = true;
    let output = rig.frame(vec![press(Key::Enter, Modifiers::ALT)]);
    assert!(matches!(output, Some(Output::Overlay(result)) if result.symbol == "BTCUSDT"));
    assert!(!rig.picker.is_open(), "a one-off compare closes the picker");
}

#[test]
fn a_chart_that_takes_no_overlay_ignores_option_return() {
    let mut rig = Rig::new(1100.0, "btc");
    // The key still replaces: the chart is not asked for something it cannot do.
    let output = rig.frame(vec![press(Key::Enter, Modifiers::ALT)]);
    assert!(matches!(output, Some(Output::Replace(_))), "{output:?}");
}

#[test]
fn comparing_keeps_the_picker_open_so_several_markets_can_be_added() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.picker.open_compare();
    assert_eq!(rig.picker.mode(), PickMode::Compare);
    rig.type_text("btc");
    let first = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(matches!(first, Some(Output::Overlay(result)) if result.symbol == "BTCUSDT"));
    assert!(rig.picker.is_open());
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let second = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(matches!(second, Some(Output::Overlay(result)) if result.symbol == "BTCUSDC"));
    assert!(rig.picker.is_open());
}

#[test]
fn comparing_still_adds_a_chart_on_shift_return_and_then_closes() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.picker.open_compare();
    rig.type_text("btc");
    let output = rig.frame(vec![press(Key::Enter, Modifiers::SHIFT)]);
    assert!(matches!(output, Some(Output::Add(_))));
    assert!(!rig.picker.is_open());
}

#[test]
fn a_market_already_on_the_chart_is_not_picked_again_while_comparing() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.picker.open_compare();
    rig.type_text("btc");
    rig.overlaid = vec![InstrumentId::new(DataSource::Binance, "BTCUSDT")];
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(output.is_none(), "{output:?}");
    assert!(rig.picker.is_open());
}

#[test]
fn the_footer_names_what_return_will_do_in_each_mode() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.can_overlay = true;
    let footer = |rig: &mut Rig| {
        let (_, output) = rig.frame_with_output(Vec::new());
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let texts = footer(&mut rig);
    assert!(texts.iter().any(|t| t == "Replace ETHUSDT"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Compare"), "{texts:?}");
    rig.picker.open_compare();
    rig.frame(Vec::new());
    let texts = footer(&mut rig);
    assert!(
        texts.iter().any(|t| t == "Compare with ETHUSDT"),
        "{texts:?}"
    );
    assert!(texts.iter().all(|t| t != "Replace ETHUSDT"));
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
    assert_eq!(rig.picker.scope, Scope::Predictions);
    rig.key(Key::Tab, Modifiers::NONE);
    assert_eq!(rig.picker.scope, Scope::Dashboards);
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
    for expected in [
        Scope::Stocks,
        Scope::Crypto,
        Scope::Predictions,
        Scope::Dashboards,
        Scope::Watchlists,
        Scope::All,
    ] {
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
fn a_chart_that_takes_overlays_adds_compare_to_the_row_menu() {
    let mut rig = Rig::new(1100.0, "btc");
    rig.can_overlay = true;
    // Replace, Compare with ETHUSDT, Add as new chart.
    let output = pick_from_the_row_menu(&mut rig, 18.0 + 24.0);
    assert!(
        matches!(&output, Some(Output::Overlay(result)) if result.symbol == "BTCUSDC"),
        "the second item compares, got {output:?}"
    );
    let mut rig = Rig::new(1100.0, "btc");
    rig.can_overlay = true;
    let output = pick_from_the_row_menu(&mut rig, 18.0 + 48.0);
    assert!(
        matches!(&output, Some(Output::Add(_))),
        "the third adds, got {output:?}"
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
    for width in [480.0, 568.0, 700.0, 780.0, 1100.0] {
        let mut rig = Rig::new(width, "btc");
        rig.quote = Some(quote(76_075.70, 208.12, 0.27, Freshness::Live));
        rig.on_tab = vec![InstrumentId::new(DataSource::Binance, "BTCUSDT")];
        rig.settle();
        rig.picker.set_scope(Scope::Stocks);
        rig.settle();
    }
}

/// The text the last frame drew, as the footer and the rows say it.
fn texts(rig: &mut Rig) -> Vec<String> {
    let (_, output) = rig.frame_with_output(Vec::new());
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// A picker on the Predictions tab with the stub's answers to "fed" in.
fn predictions_rig() -> Rig {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.set_scope(Scope::Predictions);
    rig.type_text("fed");
    rig
}

/// The choice rows of the open event called `title`.
fn choice_of(rig: &mut Rig, label: &str) -> Item {
    let ctx = rig.screen.ctx.clone();
    let listing = rig.picker.listing(&ctx);
    rig.picker
        .rows(&listing)
        .items
        .into_iter()
        .find(|item| matches!(item, Item::Choice(result) if result.symbol == label))
        .unwrap_or_else(|| panic!("{label} is listed"))
}

#[test]
fn the_predictions_tab_groups_choices_under_their_event_and_folds_them() {
    let mut rig = predictions_rig();
    // Providers answer in order; several events start folded, and a lone choice is a market.
    assert_eq!(rig.names(), ["Fed decision", "Yes", "Fed rate"]);
    assert!(rig.heads().is_empty(), "answers to a query have no caption");
    // Return on a header shows its choices, likeliest first, and picks nothing.
    let output = rig.key(Key::Enter, Modifiers::NONE);
    assert!(output.is_none(), "{output:?}");
    assert_eq!(
        rig.names(),
        ["Fed decision", "Hold", "Cut", "Hike", "Yes", "Fed rate"]
    );
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.names(), ["Fed decision", "Yes", "Fed rate"]);
    assert!(rig.picker.is_open());
}

#[test]
fn a_venue_narrows_the_predictions_to_one_provider() {
    let mut rig = predictions_rig();
    rig.picker.set_venue(Some(DataSource::Kalshi));
    rig.settle();
    // One event alone starts open.
    assert_eq!(rig.names(), ["Fed rate", "Above 5%", "Below 5%"]);
    rig.picker.set_venue(Some(DataSource::Polymarket));
    rig.settle();
    assert_eq!(rig.names(), ["Fed decision", "Yes"]);
}

#[test]
fn return_on_a_lone_prediction_choice_uses_it_like_any_market() {
    let mut rig = predictions_rig();
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::Replace(result)) = output else {
        panic!("expected a replace, got {output:?}");
    };
    assert_eq!(result.symbol, "Yes");
    assert_eq!(result.event.expect("event").title, "Rain tomorrow?");
    // Predictions are never remembered as recent markets.
    assert!(recents::load_recents(&rig.store.lock().unwrap()).is_empty());
}

#[test]
fn ticked_choices_are_charted_together_led_by_the_first_in_event_order() {
    let mut rig = predictions_rig();
    rig.key(Key::Enter, Modifiers::NONE);
    for label in ["Cut", "Hold"] {
        let item = choice_of(&mut rig, label);
        rig.picker.tick(&item);
    }
    assert_eq!(predictions::ticked_count(rig.picker.ticked.as_ref()), 2);
    assert!(
        texts(&mut rig).iter().any(|text| text == "Chart 2 choices"),
        "the footer offers to chart them"
    );
    // Return on a choice of that event takes every tick, not just the row.
    rig.key(Key::ArrowDown, Modifiers::NONE);
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::Replace(result)) = output else {
        panic!("expected a replace, got {output:?}");
    };
    let series = result.event.expect("event").series;
    let labels: Vec<&str> = series.iter().map(|choice| choice.label.as_str()).collect();
    assert_eq!(labels, ["Hold", "Cut"]);
    assert_eq!(result.symbol, "Hold");
}

#[test]
fn one_ticked_choice_gets_its_own_button_and_is_what_return_takes() {
    let mut rig = predictions_rig();
    rig.key(Key::Enter, Modifiers::NONE);
    let item = choice_of(&mut rig, "Cut");
    rig.picker.tick(&item);
    assert!(
        texts(&mut rig).iter().any(|text| text == "Chart 1 choice"),
        "the footer offers to chart it"
    );
    // Return on another choice of the event takes the tick, not the highlighted row.
    rig.key(Key::ArrowDown, Modifiers::NONE);
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::Replace(result)) = output else {
        panic!("expected a replace, got {output:?}");
    };
    assert_eq!(result.symbol, "Cut");
    // Unticking it takes the button away again.
    rig.picker.tick(&item);
    assert!(
        texts(&mut rig)
            .iter()
            .all(|text| !text.starts_with("Chart ") || text.ends_with("chart"))
    );
}

#[test]
fn a_watchlist_names_the_button_for_ticked_choices_as_an_add() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_for_watchlist("Favorites");
    rig.picker.set_scope(Scope::Predictions);
    rig.type_text("fed");
    rig.key(Key::Enter, Modifiers::NONE);
    for label in ["Cut", "Hold"] {
        let item = choice_of(&mut rig, label);
        rig.picker.tick(&item);
    }
    assert!(texts(&mut rig).iter().any(|text| text == "Add 2 choices"));
}

#[test]
fn ticks_do_not_follow_the_user_to_another_event_or_tab() {
    let mut rig = predictions_rig();
    rig.key(Key::Enter, Modifiers::NONE);
    for label in ["Cut", "Hold"] {
        let item = choice_of(&mut rig, label);
        rig.picker.tick(&item);
    }
    // Return elsewhere (the lone choice) uses that row alone.
    for _ in 0..4 {
        rig.key(Key::ArrowDown, Modifiers::NONE);
    }
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(
        matches!(&output, Some(Output::Replace(r)) if r.symbol == "Yes" && r.event.as_ref().is_some_and(|e| e.series.is_empty())),
        "{output:?}"
    );

    let mut rig = predictions_rig();
    rig.key(Key::Enter, Modifiers::NONE);
    let item = choice_of(&mut rig, "Cut");
    rig.picker.tick(&item);
    rig.picker.set_scope(Scope::All);
    assert!(rig.picker.ticked.is_none());
}

#[test]
fn an_empty_predictions_box_lists_the_trending_events() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.set_scope(Scope::Predictions);
    rig.settle();
    assert_eq!(rig.heads(), ["Trending on Polymarket"]);
    assert_eq!(rig.names(), ["Election", "Yes"]);
    // Kalshi cannot say what is busiest: nothing is listed, and the topics stay.
    rig.picker.set_venue(Some(DataSource::Kalshi));
    rig.settle();
    assert!(rig.names().is_empty());
    assert!(rig.heads().is_empty());
    rig.picker.set_venue(None);
    assert_eq!(rig.names(), ["Election", "Yes"]);
}

#[test]
fn a_provider_that_fails_is_named_and_the_others_still_answer() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.set_scope(Scope::Predictions);
    rig.type_text("boom");
    let ctx = rig.screen.ctx.clone();
    let listing = rig.picker.listing(&ctx);
    assert_eq!(listing.errors.len(), 1);
    assert_eq!(listing.errors[0].0, DataSource::Kalshi);
    assert!(rig.names().is_empty());
}

fn portfolios() -> Vec<Portfolio> {
    vec![Portfolio::new(1, "Main", 0), Portfolio::new(2, "Alts", 0)]
}

fn dashboards_rig(with_portfolios: bool) -> Rig {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.set_scope(Scope::Dashboards);
    assert!(rig.picker.wants_portfolios());
    if with_portfolios {
        rig.picker.set_portfolios(portfolios());
    }
    rig.settle();
    rig
}

#[test]
fn the_dashboards_tab_lists_the_indices_then_the_portfolio_views() {
    let mut rig = dashboards_rig(true);
    let names = rig.names();
    assert_eq!(names.len(), 8, "{names:?}");
    assert_eq!(names[0], "Altcoin Season History");
    assert_eq!(names[4], "Portfolio Value Chart");
    assert_eq!(rig.heads(), ["Market indices", "Portfolio"]);
    // Nothing on this tab has a price to fetch.
    assert!(rig.picker.highlighted().is_none());
    rig.type_text("allocation");
    assert_eq!(rig.names(), ["Allocation"]);
}

#[test]
fn an_index_replaces_the_chart_like_a_market_and_a_view_adds_a_card() {
    let mut rig = dashboards_rig(true);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::Replace(result)) = output else {
        panic!("an index replaces, got {output:?}");
    };
    assert_eq!(result.source, DataSource::CoinMarketCap);
    assert_eq!(
        result.full_symbol,
        crate::model::market::cmc::CHART_TYPES[0].0
    );
    assert!(recents::load_recents(&rig.store.lock().unwrap()).is_empty());

    let mut rig = dashboards_rig(true);
    for _ in 0..4 {
        rig.key(Key::ArrowDown, Modifiers::NONE);
    }
    assert!(texts(&mut rig).iter().any(|text| text == "Add card"));
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::AddCard(card)) = output else {
        panic!("a view adds a card, got {output:?}");
    };
    assert_eq!((card.kind, card.portfolio_id), (CardKind::ValueChart, None));
}

#[test]
fn the_portfolio_filter_scopes_the_cards() {
    let mut rig = dashboards_rig(true);
    rig.picker.set_filter(Filter::Portfolio(Some(2)));
    for _ in 0..5 {
        rig.key(Key::ArrowDown, Modifiers::NONE);
    }
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    let Some(Output::AddCard(card)) = output else {
        panic!("a view adds a card, got {output:?}");
    };
    assert_eq!((card.kind, card.portfolio_id), (CardKind::Value, Some(2)));
    // The filter is the portfolios', not the exchanges'.
    let menu = rig.picker.filter_menu().expect("a filter");
    assert_eq!(menu.label, "Alts");
    assert!(menu.active);
}

#[test]
fn without_a_portfolio_the_tab_offers_the_indices_and_says_where_views_come_from() {
    let mut rig = dashboards_rig(false);
    assert_eq!(rig.names().len(), 4);
    assert_eq!(rig.heads(), ["Market indices"]);
    let ctx = rig.screen.ctx.clone();
    let listing = rig.picker.listing(&ctx);
    assert!(rig.picker.rows(&listing).note.is_some());
    assert!(rig.picker.filter_menu().is_none(), "nothing to filter by");
    // A card already on the tab is checked.
    let mut rig = dashboards_rig(true);
    rig.cards_on_tab = vec![PortfolioCard::new(CardKind::Allocation, None)];
    rig.settle();
    let item = Item::Tile(
        dashboards::groups(&portfolios(), None, "")
            .remove(1)
            .tiles
            .remove(2),
    );
    let state = rig.picker.row_state(
        0,
        &item,
        &Env {
            on_tab: &[],
            quote: None,
            replacing: None,
            overlaid: &[],
            can_overlay: false,
            cards_on_tab: &rig.cards_on_tab,
        },
    );
    assert!(state.on_chart);
}

#[test]
fn the_add_mode_adds_a_chart_and_says_so() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_add();
    rig.type_text("btc");
    assert_eq!(rig.picker.mode(), PickMode::Add);
    let shown = texts(&mut rig);
    assert!(shown.iter().any(|text| text == "Add chart"), "{shown:?}");
    assert!(shown.iter().all(|text| !text.starts_with("Replace")));
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(matches!(output, Some(Output::Add(result)) if result.symbol == "BTCUSDT"));
    assert!(!rig.picker.is_open());
}

#[test]
fn the_watchlist_mode_follows_markets_and_stays_open_for_the_next() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_for_watchlist("Favorites");
    rig.type_text("btc");
    assert_eq!(rig.picker.mode(), PickMode::Watchlist);
    let shown = texts(&mut rig);
    assert!(
        shown.iter().any(|text| text == "Add to Favorites"),
        "{shown:?}"
    );
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(matches!(output, Some(Output::Watchlist(result)) if result.symbol == "BTCUSDT"));
    assert!(
        rig.picker.is_open(),
        "the next market is one keystroke away"
    );
    assert!(texts(&mut rig).iter().any(|text| text == "Added BTCUSDT"));
    // ⇧Return does the same: a watchlist has nothing to replace.
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::SHIFT)]);
    assert!(matches!(output, Some(Output::Watchlist(_))), "{output:?}");
    // What a watchlist follows is not a recent chart.
    assert!(recents::load_recents(&rig.store.lock().unwrap()).is_empty());
}

#[test]
fn the_watchlist_mode_adds_views_and_predictions_and_skips_its_own_tab() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_for_watchlist("Favorites");
    let mut seen = vec![rig.picker.scope];
    for _ in 0..Scope::TABS.len() {
        rig.key(Key::Tab, Modifiers::NONE);
        seen.push(rig.picker.scope);
    }
    assert!(!seen.contains(&Scope::Watchlists), "{seen:?}");
    assert!(seen.contains(&Scope::Predictions) && seen.contains(&Scope::Dashboards));

    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_for_watchlist("Favorites");
    rig.picker.set_scope(Scope::Dashboards);
    rig.picker.set_portfolios(portfolios());
    rig.settle();
    for _ in 0..4 {
        rig.key(Key::ArrowDown, Modifiers::NONE);
    }
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(
        matches!(output, Some(Output::WatchlistCard(_))),
        "{output:?}"
    );
    assert!(rig.picker.is_open());

    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_for_watchlist("Favorites");
    rig.picker.set_scope(Scope::Predictions);
    rig.type_text("fed");
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let output = rig.frame(vec![press(Key::Enter, Modifiers::NONE)]);
    assert!(
        matches!(&output, Some(Output::Watchlist(r)) if r.symbol == "Yes"),
        "{output:?}"
    );
}

#[test]
fn comparing_offers_only_the_tabs_whose_markets_can_be_drawn_over_a_chart() {
    let mut rig = Rig::new(1100.0, "");
    rig.picker.open_compare();
    let mut seen = Vec::new();
    for _ in 0..Scope::TABS.len() {
        rig.key(Key::Tab, Modifiers::NONE);
        seen.push(rig.picker.scope);
    }
    assert_eq!(
        seen[..4],
        [Scope::Stocks, Scope::Crypto, Scope::Watchlists, Scope::All]
    );
}

#[test]
fn the_tab_row_gives_up_room_in_order_and_never_asks_for_more_than_it_has() {
    // The six tab titles and the "All venues" pull-down, as they measure at 15 and 14 pt.
    let labels = [16.9, 45.5, 45.6, 74.7, 80.3, 69.3];
    let filter = Some(161.7);
    let roomy = TabFit {
        padding: 28.0,
        compact: false,
    };
    assert_eq!(fit_tabs(&labels, filter, 900.0), roomy);
    // Tighter tabs go first, then the pull-down reduces to its icon, then the tabs tighten again.
    assert_eq!(
        fit_tabs(&labels, filter, 630.0),
        TabFit {
            padding: 20.0,
            compact: false
        }
    );
    assert_eq!(
        fit_tabs(&labels, filter, 600.0),
        TabFit {
            padding: 20.0,
            compact: true
        }
    );
    assert_eq!(
        fit_tabs(&labels, filter, 490.0),
        TabFit {
            padding: 14.0,
            compact: true
        }
    );
    assert_eq!(
        fit_tabs(&labels, filter, 100.0),
        TabFit {
            padding: 14.0,
            compact: true
        }
    );
    // With no pull-down the tabs keep their room longer.
    assert_eq!(fit_tabs(&labels, None, 520.0), roomy);
    // The smallest picker (520 wide) still holds every tab and the pull-down's icon.
    let room = 520.0 - 2.0 * PADDING;
    let fit = fit_tabs(&labels, filter, room);
    let used: f32 = labels.iter().map(|label| label + fit.padding).sum::<f32>()
        + FILTER_GAP
        + if fit.compact { FILTER_COMPACT } else { 161.7 };
    assert!(used <= room, "{fit:?} uses {used} of {room}");
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

    // The other tabs and modes, in both appearances.
    type Setup = fn(&mut Rig);
    let scenes: [(&str, f32, Setup); 13] = [
        ("symbol-picker-predictions", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
            rig.type_text("fed");
        }),
        ("symbol-picker-predictions-open", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
            rig.type_text("fed");
            rig.key(Key::Enter, Modifiers::NONE);
            rig.key(Key::ArrowDown, Modifiers::NONE);
        }),
        ("symbol-picker-predictions-ticked", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
            rig.type_text("fed");
            rig.key(Key::Enter, Modifiers::NONE);
            for label in ["Cut", "Hold"] {
                let item = choice_of(rig, label);
                rig.picker.tick(&item);
            }
            rig.key(Key::ArrowDown, Modifiers::NONE);
        }),
        ("symbol-picker-predictions-trending", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
        }),
        ("symbol-picker-predictions-none", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
            rig.type_text("zzzz");
        }),
        ("symbol-picker-dashboards", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Dashboards);
            rig.picker.set_portfolios(portfolios());
            rig.key(Key::ArrowDown, Modifiers::NONE);
            rig.key(Key::ArrowDown, Modifiers::NONE);
            rig.key(Key::ArrowDown, Modifiers::NONE);
            rig.key(Key::ArrowDown, Modifiers::NONE);
            rig.key(Key::ArrowDown, Modifiers::NONE);
        }),
        ("symbol-picker-dashboards-none", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Dashboards);
        }),
        ("symbol-picker-narrow-predictions", 568.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
        }),
        ("symbol-picker-narrow-dashboards", 568.0, |rig| {
            rig.picker.set_scope(Scope::Dashboards);
            rig.picker.set_portfolios(portfolios());
        }),
        ("symbol-picker-narrow-all", 568.0, |rig| {
            rig.type_text("btc");
        }),
        ("symbol-picker-venue-menu", 1100.0, |rig| {
            rig.picker.set_scope(Scope::Predictions);
            rig.picker.set_venue(Some(DataSource::Polymarket));
            rig.type_text("fed");
            rig.settle();
            // The pull-down at the right of the tabs.
            let button = pos2(960.0, 138.0);
            rig.frame(vec![egui::Event::PointerMoved(button)]);
            rig.frame(click_at(button));
            rig.settle();
        }),
        ("symbol-picker-add", 1100.0, |rig| {
            rig.picker.open_add();
            rig.type_text("btc");
        }),
        ("symbol-picker-watchlist", 1100.0, |rig| {
            rig.picker.open_for_watchlist("Favorites");
            rig.type_text("btc");
            rig.key(Key::Enter, Modifiers::NONE);
        }),
    ];
    for (suffix, appearance) in [
        ("dark", theme::AppTheme::Dark),
        ("light", theme::AppTheme::Light),
    ] {
        for (name, width, setup) in scenes {
            let mut rig = Rig::sized(width, 760.0, "");
            theme::apply(&rig.screen.ctx, appearance);
            rig.quote = Some(bitcoin.clone());
            setup(&mut rig);
            rig.settle();
            let (_, output) = rig.frame_with_output(Vec::new());
            rig.screen.save(&format!("{name}-{suffix}"), output);
        }
    }
}
