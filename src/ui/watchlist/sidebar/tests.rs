use super::content::insertion_index;
use super::header::list_selector;
use super::menus::{MENU_WIDTH, SUBMENU_WIDTH};
use super::modals::{PROMPT_WIDTH, prompt_window};
use super::*;
use crate::model::DataSource;
use crate::model::InstrumentId;
use crate::model::watchlist::layout::Flags;
use crate::model::watchlist::quote::WatchlistQuote;
use crate::model::watchlist::{EntryId, Watchlist, WatchlistInstrument};
use crate::ui::kit;
use eframe::egui::{self, Id, Pos2, Rect};

/// What each menu's rows need, measured with the real font: the constants above must not be
/// narrower (a label would be cut) or more than a few points wider (blank space).
#[test]
fn menu_widths_fit_their_rows() {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let rows = |labels: &[&'static str]| -> Vec<(&'static str, Option<&'static str>)> {
        labels.iter().map(|label| (*label, None)).collect()
    };
    let mut measured = Vec::new();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        measured.push(kit::menu_width_for_rows(
            ui,
            &rows(&[
                "Add Section\u{2026}",
                "Sort By",
                "Columns",
                "Filter by Flag",
                "Show Details",
                "Import Symbols\u{2026}",
                "Export to File\u{2026}",
                "Copy as Text",
                "Open in Focused Chart",
                "Add as New Chart",
                "Open in New Tab",
                "Move to Section",
                "Set Flag",
                "Add to Watchlist",
                "Create Price Alert\u{2026}",
                "Copy Symbol",
                "Copy Qualified Symbol",
                "Remove from Watchlist",
            ]),
            0.0,
        ));
        measured.push(kit::menu_width_for_rows(
            ui,
            &rows(&[
                "Collapse Section",
                "Rename Section\u{2026}",
                "Add Section\u{2026}",
                "Delete Section",
                "Show Symbol Subtitle",
                "Descending",
                "Clear Flag",
                "New Watchlist\u{2026}",
            ]),
            0.0,
        ));
    });
    output.textures_delta.clear();
    eprintln!("measured menu widths: {measured:?}");
    for (name, have, need) in [
        ("options and row menus", MENU_WIDTH, measured[0]),
        ("submenus", SUBMENU_WIDTH, measured[1]),
    ] {
        assert!(have >= need - 1.0, "{name}: {have} is narrower than {need}");
        assert!(
            have <= need + 12.0,
            "{name}: {have} is more than 12 pt wider than the {need} the rows need"
        );
    }
}

#[test]
fn the_name_prompt_keeps_its_size_from_frame_to_frame() {
    let ctx = egui::Context::default();
    let mut state = WatchlistSidebarState {
        prompt: Some(PromptKind::NewSection(Watchlist::new("Crypto").id)),
        ..Default::default()
    };
    let mut actions = Vec::new();
    let sizes: Vec<egui::Vec2> = (0..8)
        .map(|_| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(900.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| prompt_window(ui.ctx(), &mut state, &mut actions),
            );
            output.textures_delta.clear();
            ctx.memory(|memory| {
                memory
                    .area_rect(Id::new("watchlist-prompt"))
                    .map_or(egui::Vec2::ZERO, |rect| rect.size())
            })
        })
        .collect();
    let settled = sizes[3];
    assert!(settled.y > 100.0, "the sheet laid out: {sizes:?}");
    assert!(
        sizes[3..]
            .iter()
            .all(|size| (*size - settled).abs().max_elem() < 0.5),
        "a sheet that grows each frame is broken: {sizes:?}"
    );
    assert!(settled.x < PROMPT_WIDTH + 8.0, "width stays put: {sizes:?}");
}

fn lists() -> Vec<Watchlist> {
    let mut favorites = Watchlist::new("Favorites");
    favorites.is_favorites = true;
    vec![Watchlist::new("Crypto"), favorites]
}

fn item(symbol: &str) -> WatchlistInstrument {
    WatchlistInstrument::new(
        InstrumentId::new(DataSource::Binance, symbol),
        symbol,
        symbol,
    )
}

#[test]
fn the_window_shows_its_pick_else_favorites_else_the_first() {
    let lists = lists();
    let mut state = WatchlistSidebarState::default();
    assert_eq!(state.resolved(&lists).unwrap().name, "Favorites");
    state.select_list(lists[0].id);
    assert_eq!(state.resolved(&lists).unwrap().name, "Crypto");
    // The picked list was deleted elsewhere: fall back, and remember the fallback.
    state.select_list(999);
    assert_eq!(state.resolved(&lists).unwrap().name, "Favorites");
    assert_eq!(state.selected_list_id(), Some(lists[1].id));
    assert!(state.resolved(&[]).is_none());
    assert_eq!(state.selected_list_id(), None);
    let only_one = vec![Watchlist::new("Solo")];
    assert_eq!(state.resolved(&only_one).unwrap().name, "Solo");
}

#[test]
fn highlight_follows_the_focused_chart_and_only_resyncs_on_change() {
    let mut list = Watchlist::new("L");
    let btc = item("BTCUSDT");
    let eth = item("ETHUSDT");
    let (btc_id, eth_id) = (btc.id, eth.id);
    list.add(btc, None).unwrap();
    list.add(eth, None).unwrap();

    let mut state = WatchlistSidebarState::default();
    let focused = InstrumentId::new(DataSource::Binance, "ethusdt");
    state.sync_selection(&list, Some(&focused));
    assert_eq!(state.selected_entry, Some(eth_id));

    // A click selects BTC; with the same focus the next frame must not undo it.
    state.selected_entry = Some(btc_id);
    state.sync_selection(&list, Some(&focused));
    assert_eq!(state.selected_entry, Some(btc_id));

    // Focus moves to a market not in the list: nothing is highlighted.
    let elsewhere = InstrumentId::new(DataSource::Coinbase, "BTC-USD");
    state.sync_selection(&list, Some(&elsewhere));
    assert_eq!(state.selected_entry, None);
    state.sync_selection(&list, None);
    assert_eq!(state.selected_entry, None);
}

#[test]
fn insertion_follows_row_middles() {
    let rect = |top: f32| Rect::from_min_size(Pos2::new(0.0, top), egui::Vec2::new(100.0, 40.0));
    let rects = [rect(0.0), rect(40.0), rect(80.0)];
    assert_eq!(insertion_index(&rects, -10.0), 0);
    assert_eq!(insertion_index(&rects, 19.0), 0);
    assert_eq!(insertion_index(&rects, 21.0), 1);
    assert_eq!(insertion_index(&rects, 95.0), 2);
    assert_eq!(insertion_index(&rects, 105.0), 3);
    assert_eq!(insertion_index(&[], 5.0), 0);
}

#[test]
fn prompts_describe_themselves() {
    assert_eq!(PromptKind::NewList.confirm(), "Create");
    assert_eq!(PromptKind::RenameList(1).confirm(), "Rename");
    assert_eq!(PromptKind::CopyList(1).title(), "Make a Copy");
    assert_eq!(
        PromptKind::RenameSection {
            list: 1,
            section: 2
        }
        .message(),
        "Enter a new name."
    );
}

#[test]
fn new_list_prompt_can_carry_a_market() {
    let mut state = WatchlistSidebarState::default();
    state.prompt_new_list_with(item("SOLUSDT"));
    assert!(matches!(state.prompt, Some(PromptKind::NewListWith(_))));
    assert_eq!(
        state.prompt.as_ref().unwrap().message(),
        "Name the new watchlist. SOLUSDT will be added to it."
    );
    assert!(state.prompt_text.is_empty());
}

/// The header's list selector, driven by pointer events.
struct SelectorRig {
    ctx: egui::Context,
    state: WatchlistSidebarState,
    lists: Vec<Watchlist>,
    time: f64,
}

impl SelectorRig {
    const RECT: Rect = Rect::from_min_max(Pos2::new(8.0, 8.0), Pos2::new(160.0, 34.0));

    fn new(lists: Vec<Watchlist>) -> Self {
        Self {
            ctx: egui::Context::default(),
            state: WatchlistSidebarState::default(),
            lists,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 600.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let flags = Flags::default();
        let quote = |_: &InstrumentId| None;
        let input = WatchlistSidebarInput {
            lists: &self.lists,
            flags: &flags,
            quote: &quote,
            focused_market: None,
            load_failed: false,
            error: None,
            import_report: None,
        };
        let state = &mut self.state;
        let mut output = self.ctx.run_ui(raw, |ui| {
            let list = state.resolved(input.lists);
            list_selector(ui, state, &input, list, Self::RECT);
        });
        output.textures_delta.clear();
    }

    fn click(&mut self, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    fn open(&self) -> bool {
        egui::Popup::is_any_open(&self.ctx)
    }

    fn open_menu(&mut self) {
        self.frame(Vec::new());
        self.click(Self::RECT.left_center() + egui::vec2(10.0, 0.0));
        assert!(self.open(), "the selector opens its menu");
    }

    /// A point in row `index` of the list rows: under the selector, the frame's margin and
    /// the heading.
    fn list_row(&self, index: usize) -> Pos2 {
        Pos2::new(
            40.0,
            Self::RECT.center().y + 14.0 + 6.0 + 24.0 + 12.0 + 24.0 * index as f32,
        )
    }
}

#[test]
fn the_list_menu_opens_from_the_title_and_a_click_picks_a_list() {
    let mut rig = SelectorRig::new(lists());
    let crypto = rig.lists[0].id;
    assert_ne!(rig.state.resolved(&rig.lists).map(|l| l.id), Some(crypto));
    rig.open_menu();
    rig.click(rig.list_row(0));
    assert_eq!(rig.state.selected_list_id(), Some(crypto));
    assert!(!rig.open(), "picking a list closes the menu");
}

#[test]
fn clicking_outside_the_list_menu_closes_it_and_changes_nothing() {
    let mut rig = SelectorRig::new(lists());
    let before = rig.state.resolved(&rig.lists).map(|l| l.id);
    rig.open_menu();
    rig.click(Pos2::new(560.0, 560.0));
    assert!(!rig.open());
    assert_eq!(rig.state.resolved(&rig.lists).map(|l| l.id), before);
}

#[test]
fn the_list_menu_asks_for_a_name_before_creating_a_list() {
    let mut rig = SelectorRig::new(lists());
    rig.open_menu();
    // Two lists, then the separator: the create row is the first row below them.
    let create = rig.list_row(2) + egui::vec2(0.0, 9.0);
    rig.click(create);
    assert!(matches!(rig.state.prompt, Some(PromptKind::NewList)));
}

/// The whole sidebar for one frame, with `events`; returns what it asked for.
fn show_frame(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    lists: &[Watchlist],
    events: Vec<egui::Event>,
) -> Vec<WatchlistAction> {
    let flags = Flags::default();
    let quote = |_: &InstrumentId| None;
    let input = WatchlistSidebarInput {
        lists,
        flags: &flags,
        quote: &quote,
        focused_market: None,
        load_failed: false,
        error: None,
        import_report: None,
    };
    let mut actions = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 600.0))),
            events,
            ..Default::default()
        },
        |ui| actions = show(ui, state, &input),
    );
    output.textures_delta.clear();
    actions
}

/// A sidebar that has drawn once on `list` (the first frame resets the highlight to the
/// focused chart's row), then has `selected` highlighted.
fn primed(ctx: &egui::Context, lists: &[Watchlist], selected: EntryId) -> WatchlistSidebarState {
    let mut state = WatchlistSidebarState::default();
    state.select_list(lists[0].id);
    show_frame(ctx, &mut state, lists, Vec::new());
    state.selected_entry = Some(selected);
    state
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

/// BTC and ETH at the root, then a section holding SOL.
fn sample_list() -> (Watchlist, [EntryId; 3], EntryId) {
    let mut list = Watchlist::new("Majors");
    let (btc, eth, sol) = (item("BTCUSDT"), item("ETHUSDT"), item("SOLUSDT"));
    let ids = [btc.id, eth.id, sol.id];
    list.add(btc, None).unwrap();
    list.add(eth, None).unwrap();
    let section = list.add_section("Alts", None).unwrap().id;
    list.add(sol, Some(section)).unwrap();
    (list, ids, section)
}

#[test]
fn only_drawn_market_rows_ask_for_quotes() {
    use crate::model::portfolio::card::{CardKind, PortfolioCard};
    let (mut shown, _, section) = sample_list();
    shown
        .add(
            WatchlistInstrument::for_card(PortfolioCard::new(CardKind::TopPerformers, None), "All"),
            None,
        )
        .unwrap();
    let mut other = Watchlist::new("Library");
    for symbol in ["LTCUSDT", "XRPUSDT", "DOGEUSDT"] {
        other.add(item(symbol), None).unwrap();
    }
    let lists = vec![shown, other];
    let ctx = egui::Context::default();
    let mut state = WatchlistSidebarState::default();
    state.select_list(lists[0].id);
    let demanded = |state: &WatchlistSidebarState| -> Vec<String> {
        state
            .quote_demand()
            .iter()
            .map(|id| id.symbol.clone())
            .collect()
    };

    show_frame(&ctx, &mut state, &lists, Vec::new());
    // The other list and the card do not ask; the open section's SOL does.
    assert_eq!(demanded(&state), ["BTCUSDT", "ETHUSDT", "SOLUSDT"]);

    let mut collapsed = lists.clone();
    collapsed[0].set_section_collapsed(section, true).unwrap();
    show_frame(&ctx, &mut state, &collapsed, Vec::new());
    assert_eq!(demanded(&state), ["BTCUSDT", "ETHUSDT"]);

    state.filter.text = "eth".to_owned();
    show_frame(&ctx, &mut state, &collapsed, Vec::new());
    assert_eq!(demanded(&state), ["ETHUSDT"]);

    state.filter.text = "nothing".to_owned();
    show_frame(&ctx, &mut state, &collapsed, Vec::new());
    assert!(demanded(&state).is_empty());
}

#[test]
fn keys_never_reach_a_row_a_filter_hides() {
    let (list, [btc, eth, _], _) = sample_list();
    let lists = vec![list.clone()];
    for hidden_key in [egui::Key::Delete, egui::Key::Backspace, egui::Key::Enter] {
        let ctx = egui::Context::default();
        let mut state = primed(&ctx, &lists, btc);
        state.filter.text = "eth".to_owned();
        let actions = show_frame(&ctx, &mut state, &lists, key(hidden_key));
        assert!(
            !actions.iter().any(|action| matches!(
                action,
                WatchlistAction::Open(_) | WatchlistAction::RemoveEntry { .. }
            )),
            "{hidden_key:?} acted on a hidden row"
        );
        assert_eq!(state.selected_entry, None, "a hidden row stays selected");
    }

    // The same key on a drawn row still works.
    let ctx = egui::Context::default();
    let mut state = primed(&ctx, &lists, eth);
    state.filter.text = "eth".to_owned();
    let actions = show_frame(&ctx, &mut state, &lists, key(egui::Key::Delete));
    assert!(actions.iter().any(
        |action| matches!(action, WatchlistAction::RemoveEntry { entry, .. } if *entry == eth)
    ));
}

#[test]
fn keys_reach_a_selected_row_that_is_drawn() {
    let (list, [btc, ..], _) = sample_list();
    let lists = vec![list.clone()];
    let ctx = egui::Context::default();
    let mut state = primed(&ctx, &lists, btc);
    let actions = show_frame(&ctx, &mut state, &lists, key(egui::Key::Enter));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, WatchlistAction::Open(item) if item.id == btc))
    );
}

#[test]
fn keys_never_reach_a_row_inside_a_collapsed_section() {
    let (mut list, [_, _, sol], section) = sample_list();
    list.set_section_collapsed(section, true).unwrap();
    let lists = vec![list.clone()];
    let ctx = egui::Context::default();
    let mut state = primed(&ctx, &lists, sol);
    let actions = show_frame(&ctx, &mut state, &lists, key(egui::Key::Delete));
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, WatchlistAction::RemoveEntry { .. }))
    );
    assert_eq!(state.selected_entry, None);
}

#[test]
fn a_filter_with_no_matches_leaves_nothing_selected() {
    let (list, [btc, ..], _) = sample_list();
    let lists = vec![list.clone()];
    let ctx = egui::Context::default();
    let mut state = primed(&ctx, &lists, btc);
    state.filter.text = "nothing".to_owned();
    let actions = show_frame(&ctx, &mut state, &lists, key(egui::Key::Delete));
    assert!(actions.is_empty());
    assert_eq!(state.selected_entry, None);
}

#[test]
fn the_detail_card_is_on_by_default() {
    assert!(WatchlistSidebarState::default().shows_detail);
}

/// Draws the detail card for a Binance market, with `quote`; returns the height it took.
fn detail_card_height(quote: Option<WatchlistQuote>) -> f32 {
    let mut list = Watchlist::new("L");
    let btc = item("BTCUSDT");
    let id = btc.id;
    list.add(btc, None).unwrap();
    let lists = vec![list];
    let state = WatchlistSidebarState {
        selected_entry: Some(id),
        ..Default::default()
    };
    let flags = Flags::default();
    let lookup = |_: &InstrumentId| quote.clone();
    let input = WatchlistSidebarInput {
        lists: &lists,
        flags: &flags,
        quote: &lookup,
        focused_market: None,
        load_failed: false,
        error: None,
        import_report: None,
    };
    let ctx = egui::Context::default();
    let mut height = 0.0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 600.0))),
            ..Default::default()
        },
        |ui| {
            detail_panel(ui, &state, &input, &lists[0]);
            height = ui.min_rect().height();
        },
    );
    output.textures_delta.clear();
    height
}

#[test]
fn the_detail_card_is_as_tall_as_it_says_whatever_the_quote_holds() {
    let plain = detail_card_height(None);
    assert_eq!(plain, detail_height(None));
    let noted = WatchlistQuote::unsupported(Some("Delayed by the provider".to_owned()));
    let with_note = detail_card_height(Some(noted.clone()));
    assert_eq!(with_note, detail_height(Some(&noted)));
    assert!(with_note > plain, "a note takes a band of its own");
}
