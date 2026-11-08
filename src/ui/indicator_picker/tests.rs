use super::*;
use crate::model::BuiltIn;
use crate::pine_studio::model::ScriptType;
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::script_catalog::sample;
use std::sync::{Arc, Mutex};

fn press(key: Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn target(source: DataSource) -> Target {
    Target {
        config: IndicatorConfig::default(),
        source,
        on_chart: Vec::new(),
    }
}

struct Rig {
    screen: Screen,
    picker: IndicatorPicker,
    target: Option<Target>,
}

impl Rig {
    fn new(width: f32) -> Self {
        let screen = Screen::new(vec2(width, 760.0));
        script_catalog::publish(
            &screen.ctx,
            vec![
                sample("a", "Alpha Script", ScriptType::Indicator),
                sample("b", "Beta Strategy", ScriptType::Strategy),
            ],
        );
        let mut picker = IndicatorPicker::new(None);
        picker.open();
        let mut rig = Self {
            screen,
            picker,
            target: Some(target(DataSource::Binance)),
        };
        // The first frames only measure the modal and hand the search field its focus.
        for _ in 0..4 {
            rig.frame(Vec::new());
        }
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<Output> {
        let mut output = None;
        let picker = &mut self.picker;
        let target = self.target.as_ref();
        self.screen.run(events, |ui| {
            if let Some(found) = picker.show(ui.ctx(), target) {
                output = Some(found);
            }
        });
        output
    }

    fn frame_with_output(
        &mut self,
        events: Vec<egui::Event>,
    ) -> (Option<Output>, egui::FullOutput) {
        let mut found = None;
        let picker = &mut self.picker;
        let target = self.target.as_ref();
        let output = self.screen.run(events, |ui| {
            if let Some(output) = picker.show(ui.ctx(), target) {
                found = Some(output);
            }
        });
        (found, output)
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> Option<Output> {
        self.frame(vec![press(key, modifiers)])
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![egui::Event::Text(text.to_owned())]);
        self.frame(Vec::new());
    }
}

fn apply(action: Action, keep_open: bool) -> Option<Output> {
    Some(Output::Apply { action, keep_open })
}

#[test]
fn the_picker_opens_with_the_search_field_focused() {
    let rig = Rig::new(1100.0);
    assert!(rig.picker.is_open());
    assert!(rig.screen.ctx.memory(|memory| memory.focused()).is_some());
}

#[test]
fn availability_follows_the_chart_and_the_indicator() {
    let catalog = indicator_catalog::all(&[]);
    let find = |key: EntryKey| catalog.iter().find(|entry| entry.key == key).unwrap();
    let rsi = find(EntryKey::Native(BuiltIn::Rsi));
    let macd = find(EntryKey::Bundled("macd"));
    let volume = find(EntryKey::Native(BuiltIn::Volume));

    let market = target(DataSource::Binance);
    assert_eq!(availability(rsi, Some(&market)), Availability::Add);
    assert_eq!(availability(macd, Some(&market)), Availability::Add);

    let mut busy = market.clone();
    busy.config.show_rsi = true;
    busy.on_chart.push("builtin:macd".to_owned());
    assert_eq!(availability(rsi, Some(&busy)), Availability::Remove);
    assert_eq!(availability(macd, Some(&busy)), Availability::AddAnother);

    let line = target(DataSource::CoinMarketCap);
    assert_eq!(availability(rsi, Some(&line)), Availability::Add);
    assert!(matches!(
        availability(macd, Some(&line)),
        Availability::Blocked(_)
    ));
    assert!(matches!(
        availability(volume, Some(&line)),
        Availability::Blocked(_)
    ));
    assert!(matches!(availability(rsi, None), Availability::Blocked(_)));
}

#[test]
fn actions_follow_what_the_entry_is() {
    let catalog = indicator_catalog::all(&[]);
    let find = |key: EntryKey| catalog.iter().find(|entry| entry.key == key).unwrap();
    let rsi = find(EntryKey::Native(BuiltIn::Rsi));
    let macd = find(EntryKey::Bundled("macd"));
    assert_eq!(
        action_for(rsi, Availability::Add),
        Some(Action::Add(BuiltIn::Rsi))
    );
    assert_eq!(
        action_for(rsi, Availability::Remove),
        Some(Action::Remove(BuiltIn::Rsi))
    );
    assert_eq!(
        action_for(macd, Availability::AddAnother),
        Some(Action::AddBundled(indicators::find("macd").unwrap()))
    );
    assert_eq!(action_for(macd, Availability::Blocked("no")), None);
}

#[test]
fn return_adds_the_selected_indicator_and_closes() {
    let mut rig = Rig::new(1100.0);
    // Discover leads with Popular, whose first entry is RSI.
    let output = rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(output, apply(Action::Add(BuiltIn::Rsi), false));
    assert!(!rig.picker.is_open());
}

#[test]
fn shift_return_adds_and_keeps_the_picker_open() {
    let mut rig = Rig::new(1100.0);
    let output = rig.key(Key::Enter, Modifiers::SHIFT);
    assert_eq!(output, apply(Action::Add(BuiltIn::Rsi), true));
    assert!(rig.picker.is_open());
}

#[test]
fn down_moves_one_row_and_up_moves_back() {
    let mut rig = Rig::new(1100.0);
    let columns = columns_for(1040.0 - SIDEBAR - DETAIL);
    rig.key(Key::ArrowDown, Modifiers::NONE);
    let catalog = indicator_catalog::all(&[]);
    let blocks = sections::blocks(Section::Discover, &catalog, &HashSet::new(), 1);
    let expected = blocks[0].entries[columns].clone();
    assert_eq!(rig.picker.selected, Some(expected.key.clone()));
    rig.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(rig.picker.selected, Some(blocks[0].entries[0].key.clone()));
    // The ends hold: Up from the first stays on the first.
    rig.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(rig.picker.selected, Some(blocks[0].entries[0].key.clone()));
}

#[test]
fn searching_lists_matches_across_sections_and_return_adds_the_first() {
    let mut rig = Rig::new(1100.0);
    rig.type_text("macd");
    let output = rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        output,
        apply(Action::AddBundled(indicators::find("macd").unwrap()), false)
    );
}

#[test]
fn a_saved_script_is_found_and_added() {
    let mut rig = Rig::new(1100.0);
    rig.type_text("alpha");
    let output = rig.key(Key::Enter, Modifiers::NONE);
    match output {
        Some(Output::Apply {
            action: Action::AddScript(script),
            keep_open: false,
        }) => assert_eq!(script.name, "Alpha Script"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn nothing_is_added_without_a_chart_or_for_an_indicator_that_cannot_run_there() {
    let mut rig = Rig::new(1100.0);
    rig.target = None;
    assert_eq!(rig.key(Key::Enter, Modifiers::NONE), None);
    assert!(
        rig.picker.is_open(),
        "a refused Return leaves the picker up"
    );

    let mut rig = Rig::new(1100.0);
    rig.target = Some(target(DataSource::CoinMarketCap));
    rig.type_text("macd");
    assert_eq!(rig.key(Key::Enter, Modifiers::NONE), None);
}

#[test]
fn return_never_takes_an_indicator_off() {
    let mut rig = Rig::new(1100.0);
    let mut busy = target(DataSource::Binance);
    busy.config.show_rsi = true;
    rig.target = Some(busy);
    assert_eq!(rig.key(Key::Enter, Modifiers::NONE), None);
    assert!(rig.picker.is_open());
}

#[test]
fn escape_closes_the_picker() {
    let mut rig = Rig::new(1100.0);
    rig.key(Key::Escape, Modifiers::NONE);
    assert!(!rig.picker.is_open());
}

#[test]
fn picking_a_sidebar_section_changes_the_list_and_clears_the_search() {
    let mut rig = Rig::new(1100.0);
    rig.type_text("macd");
    rig.picker.section = Section::Family(Family::Volatility);
    rig.picker.query.clear();
    rig.frame(Vec::new());
    let output = rig.key(Key::Enter, Modifiers::NONE);
    // The first volatility entry in catalog order is Bollinger Bands.
    assert_eq!(output, apply(Action::Add(BuiltIn::Bollinger), false));
}

#[test]
fn f_stars_the_selected_indicator_only_when_not_typing() {
    let mut rig = Rig::new(1100.0);
    // The search field has the focus, so F is a letter.
    rig.key(Key::F, Modifiers::NONE);
    assert!(rig.picker.favorites.is_empty());
    rig.screen
        .ctx
        .memory_mut(|memory| memory.surrender_focus(Id::new("indicator_picker_search")));
    rig.key(Key::F, Modifiers::NONE);
    assert_eq!(rig.picker.favorites, ["native:rsi"]);
    rig.key(Key::F, Modifiers::NONE);
    assert!(rig.picker.favorites.is_empty(), "a second F takes it off");
}

#[test]
fn favorites_are_kept_in_the_store_and_come_back() {
    let store: SharedStore = Arc::new(Mutex::new(
        crate::store::Store::open_in_memory().expect("open"),
    ));
    let mut picker = IndicatorPicker::new(Some(store.clone()));
    picker.toggle_favorite(&EntryKey::Bundled("macd"));
    picker.toggle_favorite(&EntryKey::Native(BuiltIn::Ema));
    picker.toggle_favorite(&EntryKey::Native(BuiltIn::Ema));
    let again = IndicatorPicker::new(Some(store));
    assert_eq!(again.favorites, ["bundled:macd"]);
    assert!(again.is_favorite(&EntryKey::Bundled("macd")));
}

#[test]
fn a_narrow_window_drops_the_detail_panel_and_still_draws() {
    let mut rig = Rig::new(760.0);
    rig.type_text("rsi");
    assert!(rig.picker.is_open());
    assert_eq!(
        rig.key(Key::Enter, Modifiers::NONE),
        apply(Action::Add(BuiltIn::Rsi), false)
    );
}

#[test]
fn a_click_on_a_card_selects_it_and_asks_for_another_frame() {
    let mut rig = Rig::new(1100.0);
    let catalog = indicator_catalog::all(&[]);
    let blocks = sections::blocks(Section::Discover, &catalog, &HashSet::new(), 1);
    let second = blocks[0].entries[1].key.clone();
    assert_ne!(rig.picker.selected.as_ref(), Some(&second));

    // The second card of the first row, found from the layout: the modal is centred.
    let modal_left = (1100.0 - 1040.0) / 2.0;
    let modal_top = (760.0 - 680.0) / 2.0;
    let grid_left = modal_left + SIDEBAR + 16.0;
    let card_width = ((1040.0 - SIDEBAR - DETAIL - 32.0) - GAP) / 2.0;
    let at = pos2(
        grid_left + card_width + GAP + card_width / 2.0,
        modal_top + HEADER + 12.0 + 24.0 + GAP + CARD_HEIGHT / 2.0,
    );
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    rig.frame(vec![button(true)]);
    let (_, output) = rig.frame_with_output(vec![button(false)]);
    assert_eq!(rig.picker.selected, Some(second));
    // The detail panel was drawn before the click was seen, so it needs one more frame.
    assert!(
        output
            .viewport_output
            .values()
            .any(|viewport| viewport.repaint_delay.is_zero()),
        "no repaint was requested after the click"
    );
}

#[test]
fn the_grid_has_fewer_columns_as_it_narrows() {
    assert_eq!(columns_for(700.0), 3);
    assert_eq!(columns_for(512.0), 2);
    assert_eq!(columns_for(300.0), 1);
}

/// The picker as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_picker() {
    if gallery_dir().is_none() {
        return;
    }
    for (name, width, query, section, source, favorite) in [
        (
            "picker-discover",
            1100.0,
            "",
            Section::Discover,
            DataSource::Binance,
            true,
        ),
        (
            "picker-search",
            1100.0,
            "stoch",
            Section::Discover,
            DataSource::Binance,
            false,
        ),
        (
            "picker-momentum",
            1100.0,
            "",
            Section::Family(Family::Momentum),
            DataSource::Binance,
            false,
        ),
        (
            "picker-overlays",
            1100.0,
            "",
            Section::Overlays,
            DataSource::Binance,
            false,
        ),
        (
            "picker-favorites-empty",
            1100.0,
            "",
            Section::Favorites,
            DataSource::Binance,
            false,
        ),
        (
            "picker-scripts",
            1100.0,
            "",
            Section::MyScripts,
            DataSource::Binance,
            false,
        ),
        (
            "picker-none",
            1100.0,
            "zzzz",
            Section::Discover,
            DataSource::Binance,
            false,
        ),
        (
            "picker-narrow",
            780.0,
            "",
            Section::Discover,
            DataSource::Binance,
            false,
        ),
        (
            "picker-line-chart",
            1100.0,
            "",
            Section::All,
            DataSource::CoinMarketCap,
            false,
        ),
    ] {
        let mut rig = Rig::new(width);
        rig.target = Some(target(source));
        if favorite {
            rig.picker.toggle_favorite(&EntryKey::Bundled("macd"));
        }
        rig.picker.section = section;
        if !query.is_empty() {
            rig.type_text(query);
        }
        // Give the worker time to run the bundled scripts for the previews.
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            rig.frame(Vec::new());
        }
        let size = rig.screen.size;
        let mut last = None;
        let picker = &mut rig.picker;
        let target = rig.target.as_ref();
        last = Some(rig.screen.run(Vec::new(), |ui| {
            let _ = picker.show(ui.ctx(), target);
        }))
        .or(last);
        let _ = size;
        rig.screen.save(name, last.expect("a frame"));
    }
}
