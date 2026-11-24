use super::grid::{Grid, Step, title};
use super::state::{State, Typed};
use super::*;
use crate::model::{Section, TimeUnit, TimeframeParseError};
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2};
use std::collections::HashMap;

fn custom(count: u16, unit: TimeUnit) -> Timeframe {
    Timeframe::new(count, unit).expect("in range")
}

fn labels(grid: &Grid, section: Section) -> Vec<String> {
    grid.sections()
        .iter()
        .find(|(shown, _)| *shown == section)
        .map(|(_, timeframes)| {
            timeframes
                .iter()
                .map(|timeframe| timeframe.label())
                .collect()
        })
        .expect("every section is listed")
}

#[test]
fn the_presets_fill_three_sections() {
    let grid = Grid::new(&[]);
    assert_eq!(labels(&grid, Section::Minutes), ["1m", "5m", "15m", "30m"]);
    assert_eq!(labels(&grid, Section::Hours), ["1h", "4h", "8h", "12h"]);
    assert_eq!(labels(&grid, Section::DaysAndUp), ["1D", "1W", "1M", "1Y"]);
    assert_eq!(title(Section::DaysAndUp), "Days and up");
}

#[test]
fn a_pinned_timeframe_joins_its_section_by_duration() {
    let grid = Grid::new(&[
        custom(22, TimeUnit::Minute),
        custom(3, TimeUnit::Hour),
        custom(2, TimeUnit::Day),
        custom(2, TimeUnit::Minute),
    ]);
    assert_eq!(
        labels(&grid, Section::Minutes),
        ["1m", "2m", "5m", "15m", "22m", "30m"]
    );
    assert_eq!(
        labels(&grid, Section::Hours),
        ["1h", "3h", "4h", "8h", "12h"]
    );
    assert_eq!(
        labels(&grid, Section::DaysAndUp),
        ["1D", "2D", "1W", "1M", "1Y"]
    );
}

#[test]
fn a_preset_or_a_repeat_among_the_pinned_is_listed_once() {
    let grid = Grid::new(&[
        Timeframe::HOUR_1,
        custom(22, TimeUnit::Minute),
        custom(22, TimeUnit::Minute),
    ]);
    assert_eq!(grid.all().count(), Timeframe::PRESETS.len() + 1);
}

#[test]
fn up_and_down_keep_the_column_and_clamp_to_the_row() {
    let grid = Grid::new(&[]);
    // Minutes: 1m 5m 15m 30m. Hours: 1h 4h 8h 12h. Days and up: 1D 1W 1M 1Y.
    // 1D 1W 1M 1Y.
    assert_eq!(
        grid.step(Timeframe::MINUTE_5, Step::Down),
        Some(Timeframe::HOUR_4)
    );
    assert_eq!(
        grid.step(Timeframe::HOUR_12, Step::Down),
        Some(Timeframe::YEAR_1)
    );
    assert_eq!(grid.step(Timeframe::YEAR_1, Step::Down), None);
    assert_eq!(
        grid.step(Timeframe::YEAR_1, Step::Up),
        Some(Timeframe::HOUR_12)
    );
    let pinned = Grid::new(&[custom(22, TimeUnit::Minute)]);
    // Minutes: 1m 5m 15m 22m / 30m.
    assert_eq!(
        pinned.step(Timeframe::MINUTE_15, Step::Down),
        Some(Timeframe::MINUTE_30),
        "the wrapped row is shorter than the column"
    );
    assert_eq!(
        pinned.step(Timeframe::MINUTE_30, Step::Down),
        Some(Timeframe::HOUR_1)
    );
    assert_eq!(grid.step(Timeframe::MINUTE_1, Step::Up), None);
}

#[test]
fn left_and_right_walk_the_chips_in_reading_order() {
    let grid = Grid::new(&[]);
    assert_eq!(
        grid.step(Timeframe::MINUTE_30, Step::Right),
        Some(Timeframe::HOUR_1)
    );
    assert_eq!(
        grid.step(Timeframe::HOUR_1, Step::Left),
        Some(Timeframe::MINUTE_30)
    );
    assert_eq!(grid.step(Timeframe::MINUTE_1, Step::Left), None);
    assert_eq!(grid.step(Timeframe::YEAR_1, Step::Right), None);
}

#[test]
fn from_a_typed_timeframe_down_goes_to_the_first_chip() {
    let grid = Grid::new(&[]);
    let typed = custom(22, TimeUnit::Minute);
    assert_eq!(grid.step(typed, Step::Down), Some(Timeframe::MINUTE_1));
    assert_eq!(grid.step(typed, Step::Up), None);
}

#[test]
fn the_field_is_read_as_a_timeframe() {
    assert_eq!(Typed::read(""), Typed::Empty);
    assert_eq!(Typed::read("   "), Typed::Empty);
    assert_eq!(
        Typed::read("22m"),
        Typed::Valid(custom(22, TimeUnit::Minute))
    );
    assert_eq!(Typed::read("60"), Typed::Valid(Timeframe::HOUR_1));
    assert_eq!(
        Typed::read("30s"),
        Typed::Valid(custom(30, TimeUnit::Second))
    );
    assert_eq!(
        Typed::read("60s"),
        Typed::Invalid(TimeframeParseError::OutOfRange {
            unit: TimeUnit::Second,
            max: 59
        })
    );
    assert!(matches!(Typed::read("soon"), Typed::Invalid(_)));
}

#[test]
fn the_highlight_follows_the_arrows_then_the_text_then_the_current_timeframe() {
    let current = Timeframe::HOUR_1;
    let mut state = State::default();
    assert_eq!(state.highlighted(current), Some(current));
    state.query = "22m".to_owned();
    assert_eq!(
        state.highlighted(current),
        Some(custom(22, TimeUnit::Minute))
    );
    state.moved = Some(Timeframe::DAY_1);
    assert_eq!(state.highlighted(current), Some(Timeframe::DAY_1));
    state.moved = None;
    state.query = "60s".to_owned();
    assert_eq!(state.highlighted(current), None);
}

/// A button with the picker open under it, driven by raw events. What a screen reader would call
/// each widget tells where it is.
struct Harness {
    ctx: egui::Context,
    time: f64,
    button: Rect,
    labels: HashMap<String, Rect>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut harness = Self {
            ctx,
            time: 0.0,
            button: Rect::NOTHING,
            labels: HashMap::new(),
        };
        harness.frame(Vec::new(), &Self::input(&[], &[]));
        harness
    }

    fn input<'a>(pinned: &'a [Timeframe], sources: &'a [DataSource]) -> TimeframePickerInput<'a> {
        TimeframePickerInput {
            current: Timeframe::HOUR_1,
            pinned,
            sources,
        }
    }

    fn frame(&mut self, events: Vec<Event>, input: &TimeframePickerInput) -> Option<TimeframePick> {
        self.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                pos2(0.0, 0.0),
                egui::vec2(900.0, 700.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut picked = None;
        let mut button = self.button;
        let mut output = self.ctx.run_ui(raw, |ui| {
            let response = ui.button("Timeframe");
            button = response.rect;
            picked = show(ui, &response, input);
        });
        if let Some(update) = &output.platform_output.accesskit_update {
            for (_, node) in &update.nodes {
                if let (Some(label), Some(bounds)) = (node.label(), node.bounds()) {
                    self.labels.insert(
                        label.to_owned(),
                        Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                    );
                }
            }
        }
        output.textures_delta.clear();
        self.button = button;
        picked
    }

    fn pointer(
        &mut self,
        at: Pos2,
        button: PointerButton,
        input: &TimeframePickerInput,
    ) -> Option<TimeframePick> {
        let event = |pressed| Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut picked = self.frame(vec![Event::PointerMoved(at)], input);
        picked = picked.or(self.frame(vec![event(true)], input));
        picked.or(self.frame(vec![event(false)], input))
    }

    fn click(&mut self, at: Pos2, input: &TimeframePickerInput) -> Option<TimeframePick> {
        self.pointer(at, PointerButton::Primary, input)
    }

    /// Opens the popover and lets it settle, so its field has the keyboard.
    fn open(&mut self, input: &TimeframePickerInput) {
        self.click(self.button.center(), input);
        for _ in 0..3 {
            self.frame(Vec::new(), input);
        }
    }

    fn is_open(&self) -> bool {
        egui::Popup::is_any_open(&self.ctx)
    }

    fn chip(&self, label: &str) -> Pos2 {
        self.labels
            .get(label)
            .unwrap_or_else(|| panic!("no chip {label:?} in {:?}", self.labels.keys()))
            .center()
    }

    fn type_text(&mut self, text: &str, input: &TimeframePickerInput) {
        self.frame(vec![Event::Text(text.to_owned())], input);
        self.frame(Vec::new(), input);
    }

    fn press(
        &mut self,
        key: Key,
        modifiers: Modifiers,
        input: &TimeframePickerInput,
    ) -> Option<TimeframePick> {
        let event = |pressed| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        };
        let picked = self.frame(vec![event(true)], input);
        picked.or(self.frame(vec![event(false)], input))
    }
}

#[test]
fn the_button_opens_the_picker_and_escape_closes_it() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    assert!(!harness.is_open());
    harness.open(&input);
    assert!(harness.is_open());
    assert_eq!(harness.press(Key::Escape, Modifiers::NONE, &input), None);
    harness.frame(Vec::new(), &input);
    assert!(!harness.is_open());
}

#[test]
fn return_charts_the_current_timeframe_while_the_field_is_empty() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    assert_eq!(
        harness.press(Key::Enter, Modifiers::NONE, &input),
        Some(TimeframePick::Chart(Timeframe::HOUR_1))
    );
}

#[test]
fn a_typed_timeframe_is_charted_with_return_and_pinned_with_shift_return() {
    let input = Harness::input(&[], &[]);
    let typed = custom(22, TimeUnit::Minute);
    let mut harness = Harness::new();
    harness.open(&input);
    harness.type_text("22m", &input);
    assert_eq!(
        harness.press(Key::Enter, Modifiers::NONE, &input),
        Some(TimeframePick::Chart(typed))
    );

    let mut harness = Harness::new();
    harness.open(&input);
    harness.type_text("22m", &input);
    assert_eq!(
        harness.press(Key::Enter, Modifiers::SHIFT, &input),
        Some(TimeframePick::ChartAndPin(typed))
    );
}

#[test]
fn return_does_nothing_for_text_that_is_no_timeframe() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    harness.type_text("60s", &input);
    assert_eq!(harness.press(Key::Enter, Modifiers::NONE, &input), None);
    assert!(harness.is_open(), "the picker stays for another try");
}

#[test]
fn the_arrows_move_the_highlight_and_return_charts_it() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    // From the current 1h (column 0 of the hours row): Down lands on 1D, Right on 1W.
    assert_eq!(harness.press(Key::ArrowDown, Modifiers::NONE, &input), None);
    assert_eq!(
        harness.press(Key::ArrowRight, Modifiers::NONE, &input),
        None
    );
    assert_eq!(
        harness.press(Key::Enter, Modifiers::NONE, &input),
        Some(TimeframePick::Chart(Timeframe::WEEK_1))
    );
}

#[test]
fn sideways_arrows_belong_to_the_field_once_it_has_text() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    harness.type_text("15", &input);
    harness.press(Key::ArrowRight, Modifiers::NONE, &input);
    assert_eq!(
        harness.press(Key::Enter, Modifiers::NONE, &input),
        Some(TimeframePick::Chart(Timeframe::MINUTE_15)),
        "the highlight stayed on what is typed"
    );
}

#[test]
fn typing_takes_the_highlight_back_from_the_arrows() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    harness.press(Key::ArrowDown, Modifiers::NONE, &input);
    harness.type_text("15", &input);
    assert_eq!(
        harness.press(Key::Enter, Modifiers::NONE, &input),
        Some(TimeframePick::Chart(Timeframe::MINUTE_15))
    );
}

#[test]
fn a_click_on_a_chip_charts_it_and_closes_the_picker() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    let at = harness.chip("15m");
    assert_eq!(
        harness.click(at, &input),
        Some(TimeframePick::Chart(Timeframe::MINUTE_15))
    );
    harness.frame(Vec::new(), &input);
    assert!(!harness.is_open());
}

#[test]
fn the_chips_are_one_width_and_four_to_a_row() {
    let pinned = [custom(22, TimeUnit::Minute)];
    let input = Harness::input(&pinned, &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    let rect = |label: &str| harness.labels[label];
    let first_row = ["1m", "5m", "15m", "22m"].map(rect);
    for pair in first_row.windows(2) {
        assert_eq!(pair[0].width(), pair[1].width());
        assert_eq!(pair[0].top(), pair[1].top());
        assert!(pair[0].right() < pair[1].left());
    }
    // A pinned 22m makes five minutes: the fifth wraps under the first.
    assert_eq!(rect("30m").left(), rect("1m").left());
    assert!(rect("30m").top() > rect("1m").top());
}

#[test]
fn a_pinned_chip_has_an_unpin_menu() {
    let pinned = [custom(22, TimeUnit::Minute)];
    let input = Harness::input(&pinned, &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    let at = harness.chip("22m");
    assert_eq!(harness.pointer(at, PointerButton::Secondary, &input), None);
    harness.frame(Vec::new(), &input);
    let unpin = harness.chip("Unpin");
    assert_eq!(
        harness.click(unpin, &input),
        Some(TimeframePick::Unpin(pinned[0]))
    );
    harness.frame(Vec::new(), &input);
    assert!(harness.is_open(), "unpinning leaves the picker open");
}

#[test]
fn a_preset_chip_has_no_unpin_menu() {
    let input = Harness::input(&[], &[]);
    let mut harness = Harness::new();
    harness.open(&input);
    let at = harness.chip("5m");
    harness.pointer(at, PointerButton::Secondary, &input);
    harness.frame(Vec::new(), &input);
    assert!(!harness.labels.contains_key("Unpin"));
}
