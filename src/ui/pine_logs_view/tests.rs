use super::*;
use crate::pine::SourceRange;
use crate::pine::logs::{LogBatch, LogHistory};
use eframe::egui::{Event, RawInput};

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 560.0;

fn event(sequence: u64, level: LogLevel, message: &str) -> LogEvent {
    LogEvent {
        sequence,
        level,
        message: message.to_owned(),
        time_ms: 1_700_000_000_000 + sequence as i64 * 60_000,
        bar_time_ms: 0,
        bar_index: Some(sequence as i64),
        realtime: false,
        confirmed: true,
        library: None,
        range: SourceRange::ZERO,
        request: None,
    }
}

fn history(events: Vec<LogEvent>) -> LogHistory {
    let mut history = LogHistory::default();
    history.apply(LogBatch {
        reset: true,
        from: 0,
        to: events.len() as u64,
        events,
        dropped: 0,
    });
    history
}

fn scripts() -> Vec<ScriptEntry> {
    vec![
        ScriptEntry {
            id: 1,
            label: "Alpha".to_owned(),
        },
        ScriptEntry {
            id: 2,
            label: "Alpha #2".to_owned(),
        },
    ]
}

struct Rig {
    ctx: egui::Context,
    state: LogsState,
    history: LogHistory,
    logging: bool,
    scripts: Vec<ScriptEntry>,
    actions: Vec<LogsAction>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn new(history: LogHistory) -> Self {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let mut rig = Self {
            ctx,
            state: LogsState::default(),
            history,
            logging: true,
            scripts: scripts(),
            actions: Vec::new(),
            shapes: Vec::new(),
        };
        for _ in 0..3 {
            rig.frame(Vec::new());
        }
        rig.actions.clear();
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, HEIGHT))),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| {
            self.state.select_valid(&self.scripts);
            let feed = Feed {
                epoch: 1,
                history: &self.history,
                logging: self.logging,
            };
            let actions = egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    show(ui, &mut self.state, &self.scripts, Some(feed))
                })
                .inner;
            self.actions.extend(actions);
        });
        output.textures_delta.clear();
        self.shapes = output.shapes;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    /// Every galley drawn this frame.
    fn galleys(&self) -> Vec<Arc<egui::Galley>> {
        self.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(Arc::clone(&text.galley)),
                _ => None,
            })
            .collect()
    }

    fn text(&self) -> String {
        self.galleys()
            .iter()
            .map(|galley| galley.job.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[test]
fn a_chart_with_no_scripts_says_so() {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let mut state = LogsState::default();
    let mut shown = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, HEIGHT))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    show(ui, &mut state, &[], None);
                });
        });
        output.textures_delta.clear();
        shown.clear();
        for clipped in output.shapes {
            if let egui::Shape::Text(text) = clipped.shape {
                shown.push_str(&text.galley.job.text);
                shown.push('\n');
            }
        }
    }
    assert!(shown.contains("No scripts on this chart"), "{shown}");
}

#[test]
fn an_empty_history_shows_how_to_work_with_logs() {
    let rig = Rig::new(history(Vec::new()));
    let text = rig.text();
    assert!(text.contains("Pine logs"), "{text}");
    assert!(text.contains("How to work with logs"), "{text}");
    assert!(text.contains("Add the indicator to the chart."), "{text}");
    assert!(text.contains("log functions"), "{text}");
    assert!(text.contains("Enable logging."), "{text}");
}

impl Rig {
    /// The vertical middle of everything drawn under the toolbar, and of the room it has.
    fn content_and_room_middles(&self) -> (f32, f32) {
        let below = 106.0;
        let mut top = f32::MAX;
        let mut bottom = f32::MIN;
        for clipped in &self.shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            if bounds.is_positive() && bounds.min.y >= below && bounds.max.y < HEIGHT {
                top = top.min(bounds.min.y);
                bottom = bottom.max(bounds.max.y);
            }
        }
        ((top + bottom) / 2.0, (below + HEIGHT) / 2.0)
    }
}

/// The gaps above and below the toolbar's script picker: from `above` (the top of the pane, or
/// the divider under the title) to the picker, and from the picker to the divider under it.
fn toolbar_gaps(rig: &Rig, above: f32) -> (f32, f32) {
    let rects: Vec<Rect> = rig
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => Some(rect.rect),
            _ => None,
        })
        .collect();
    let picker = rects
        .iter()
        .find(|rect| (rect.height() - 26.0).abs() < 0.1 && rect.min.y >= above)
        .expect("the script picker");
    let divider = rects
        .iter()
        .filter(|rect| {
            rect.height() <= 1.1 && rect.width() >= WIDTH - 1.0 && rect.min.y > picker.max.y
        })
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the divider under the toolbar");
    (picker.min.y - above, divider.min.y - picker.max.y)
}

#[test]
fn the_toolbar_has_the_same_room_above_and_below_its_content() {
    // Inside a tab: the toolbar is the first thing in the pane.
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.state.embedded = true;
    rig.settle();
    let (above, below) = toolbar_gaps(&rig, 0.0);
    assert!((above - below).abs() < 1.0, "{above} above, {below} below");

    // Docked: under the title and its divider.
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.settle();
    let divider_under_title = rig
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect)
                if rect.rect.height() <= 1.1 && rect.rect.width() >= WIDTH - 1.0 =>
            {
                Some(rect.rect.max.y)
            }
            _ => None,
        })
        .fold(f32::MAX, f32::min);
    let (above, below) = toolbar_gaps(&rig, divider_under_title);
    assert!((above - below).abs() < 1.0, "{above} above, {below} below");
}

#[test]
fn the_empty_states_sit_in_the_middle_of_the_pane() {
    for logging in [true, false] {
        let mut rig = Rig::new(history(Vec::new()));
        rig.logging = logging;
        rig.settle();
        rig.settle();
        let (content, room) = rig.content_and_room_middles();
        assert!(
            (content - room).abs() < 12.0,
            "logging {logging}: content centred on {content}, room on {room}"
        );
    }
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.state.filter.query = "zzz".to_owned();
    rig.settle();
    rig.settle();
    let (content, room) = rig.content_and_room_middles();
    assert!((content - room).abs() < 12.0, "{content} vs {room}");
}

#[test]
fn an_empty_history_with_logging_off_offers_to_turn_it_on() {
    let mut rig = Rig::new(history(Vec::new()));
    rig.logging = false;
    rig.settle();
    let text = rig.text();
    assert!(text.contains("Logging is disabled"), "{text}");
    assert!(!text.contains("How to work with logs"), "{text}");
}

#[test]
fn messages_are_listed_with_their_time_and_text_in_the_level_colors() {
    let rig = Rig::new(history(vec![
        event(0, LogLevel::Info, "plain info"),
        event(1, LogLevel::Warning, "careful"),
        event(2, LogLevel::Error, "broken"),
    ]));
    let text = rig.text();
    assert!(text.contains("2023-11-14T22:13:20.000Z"), "{text}");
    assert!(text.contains("plain info"), "{text}");
    assert!(text.contains("careful") && text.contains("broken"));
    assert!(!text.contains("How to work with logs"));
    let colors: Vec<Color32> = rig
        .galleys()
        .iter()
        .filter(|galley| {
            ["plain info", "careful", "broken"]
                .iter()
                .any(|message| galley.job.text.contains(message))
        })
        .map(|galley| galley.job.sections.last().unwrap().format.color)
        .collect();
    assert_eq!(colors, [level_color(LogLevel::Info), kit::ORANGE, kit::RED]);
}

#[test]
fn a_multiline_message_keeps_its_lines() {
    let rig = Rig::new(history(vec![event(
        0,
        LogLevel::Info,
        "first\nsecond\nthird",
    )]));
    let galley = rig
        .galleys()
        .into_iter()
        .find(|galley| galley.job.text.contains("second"))
        .expect("the row");
    assert!(galley.rows.len() >= 3, "{} rows", galley.rows.len());
}

#[test]
fn a_long_unbroken_token_wraps_inside_the_pane() {
    let rig = Rig::new(history(vec![event(0, LogLevel::Info, &"x".repeat(900))]));
    let galley = rig
        .galleys()
        .into_iter()
        .find(|galley| galley.job.text.contains("xxxx"))
        .expect("the row");
    assert!(galley.size().x <= WIDTH, "{} wide", galley.size().x);
    assert!(galley.rows.len() > 2);
}

#[test]
fn html_in_a_message_is_drawn_as_text() {
    let rig = Rig::new(history(vec![event(
        0,
        LogLevel::Info,
        "<b>bold</b> **md** `code`",
    )]));
    assert!(rig.text().contains("<b>bold</b> **md** `code`"));
}

#[test]
fn ten_thousand_messages_lay_out_only_the_rows_on_screen() {
    let events = (0..10_000)
        .map(|n| event(n, LogLevel::Info, &format!("message number {n}")))
        .collect();
    let mut rig = Rig::new(history(events));
    rig.settle();
    let rows = rig
        .galleys()
        .iter()
        .filter(|galley| galley.job.text.contains("message number"))
        .count();
    assert!(rows > 5 && rows < 80, "{rows} rows laid out");
    // The newest message is the one at the bottom.
    let text = rig.text();
    assert!(
        text.contains("message number 9999"),
        "{}",
        &text[text.len().saturating_sub(600)..]
    );
}

#[test]
fn a_search_lights_its_matches_and_leaves_the_rest_out() {
    let mut rig = Rig::new(history(vec![
        event(0, LogLevel::Info, "bar one"),
        event(1, LogLevel::Info, "something else"),
        event(2, LogLevel::Warning, "Bar two"),
    ]));
    rig.state.filter.query = "bar".to_owned();
    rig.settle();
    let text = rig.text();
    assert!(
        text.contains("bar one") && text.contains("Bar two"),
        "{text}"
    );
    assert!(!text.contains("something else"), "{text}");
    let lit: Vec<String> = rig
        .galleys()
        .iter()
        .flat_map(|galley| {
            galley
                .job
                .sections
                .iter()
                .filter(|section| section.format.background != Color32::TRANSPARENT)
                .map(|section| {
                    galley.job.text[section.byte_range.start.0..section.byte_range.end.0].to_owned()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(lit, ["bar", "Bar"]);
}

#[test]
fn filters_that_hide_everything_say_so_and_clear_filters_restores_the_list() {
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.state.filter.query = "nothing like this".to_owned();
    rig.settle();
    let text = rig.text();
    assert!(text.contains("No matching messages"), "{text}");
    assert!(!text.contains("How to work with logs"));
    rig.state.clear_filters();
    rig.settle();
    assert!(rig.text().contains("hello"));
}

#[test]
fn a_regex_that_does_not_compile_is_flagged_and_lists_everything() {
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.state.filter.options.regex = true;
    rig.state.filter.query = "(".to_owned();
    rig.settle();
    let text = rig.text();
    assert!(text.contains("hello"), "{text}");
    assert!(rig.state.filtered.search().is_invalid());
}

fn on_bar(sequence: u64, message: &str, bar_time_ms: i64) -> LogEvent {
    LogEvent {
        bar_time_ms,
        ..event(sequence, LogLevel::Info, message)
    }
}

#[test]
fn the_bar_range_filters_by_the_bar_a_message_was_written_on() {
    let mut rig = Rig::new(history(vec![
        on_bar(0, "early", 1_700_000_000_000),
        on_bar(1, "inside", 1_700_000_300_000),
        on_bar(2, "late", 1_700_000_900_000),
    ]));
    // The wall-clock time (`time_ms`) of all three is within minutes; the bars are what differ.
    rig.state.set_range(1_700_000_600_000, 1_700_000_300_000);
    rig.settle();
    let text = rig.text();
    assert!(
        text.contains("inside") && !text.contains("early") && !text.contains("late"),
        "{text}"
    );
    rig.state.clear_filters();
    assert_eq!(rig.state.filter.range, None);
}

#[test]
fn the_range_pill_says_what_it_waits_for_and_what_it_keeps() {
    let mut state = LogsState::default();
    assert_eq!(range_label(&state), "All bars");
    state.picking = true;
    assert_eq!(range_label(&state), "Drag on the chart");
    state.picking = false;
    let at = 1_700_000_000_000;
    state.set_range(at, at);
    assert!(
        range_label(&state).starts_with("Bar "),
        "{}",
        range_label(&state)
    );
    state.set_range(at, at + 3_600_000);
    let one_day = range_label(&state);
    assert_eq!(one_day.matches(", ").count(), 1, "{one_day}");
    assert!(one_day.contains(" – "), "{one_day}");
    state.set_range(at, at + 2 * 86_400_000);
    assert_eq!(range_label(&state).matches(", ").count(), 2);
}

/// The pill alone, 26 high at the top left, and what a click at `at` on it asks for.
fn click_pill(state: &mut LogsState, width: f32, narrow: bool, at: egui::Pos2) -> Vec<LogsAction> {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let mut actions = Vec::new();
    for events in [
        Vec::new(),
        vec![Event::PointerMoved(at)],
        vec![Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }],
        vec![Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    ] {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, HEIGHT))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| range_pill(ui, state, width, narrow, &mut actions));
        });
        output.textures_delta.clear();
    }
    actions
}

#[test]
fn clicking_the_range_pill_asks_for_the_drag_and_clicking_it_again_cancels() {
    let mut state = LogsState::default();
    let at = pos2(13.0, 13.0);
    assert_eq!(
        click_pill(&mut state, 26.0, true, at),
        [LogsAction::PickRange]
    );
    state.picking = true;
    assert_eq!(
        click_pill(&mut state, 26.0, true, at),
        [LogsAction::CancelPick]
    );
}

#[test]
fn the_cross_on_a_set_range_removes_it_without_asking_for_a_pick() {
    let mut state = LogsState::default();
    state.set_range(1_000, 2_000);
    // A 220-wide pill keeps its cross 14 from the right edge.
    let actions = click_pill(&mut state, 220.0, false, pos2(220.0 - 14.0, 13.0));
    assert!(actions.is_empty(), "{actions:?}");
    assert_eq!(state.filter.range, None);
}

#[test]
fn the_selector_falls_back_to_a_script_that_exists() {
    let mut state = LogsState {
        selected: Some(99),
        ..Default::default()
    };
    state.select_valid(&scripts());
    assert_eq!(state.selected, Some(1));
    state.selected = Some(2);
    state.select_valid(&scripts());
    assert_eq!(state.selected, Some(2));
    state.select_valid(&[]);
    assert_eq!(state.selected, None);
}

#[test]
fn prefs_carry_the_levels_and_search_options() {
    let mut state = LogsState::default();
    state.filter.levels.info = false;
    state.filter.options.regex = true;
    assert!(!state.prefs().levels.info && state.prefs().search.regex);
    state.filter.levels.toggle(LogLevel::Info);
    assert!(state.prefs().levels.info);
}

#[test]
fn new_messages_appear_without_rebuilding_the_list() {
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "one")]));
    rig.history.apply(LogBatch {
        reset: false,
        from: 1,
        to: 2,
        events: vec![event(1, LogLevel::Info, "two")],
        dropped: 0,
    });
    rig.settle();
    let text = rig.text();
    assert!(text.contains("one") && text.contains("two"), "{text}");
    assert!(!rig.state.filtered.rebuilt());
}

impl Rig {
    /// The top left of the row whose text holds `needle`.
    fn row_top(&self, needle: &str) -> egui::Pos2 {
        self.shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                    Some(text.pos - vec2(ROW_PAD_X, ROW_PAD_Y))
                }
                _ => None,
            })
            .expect("the row is drawn")
    }

    fn press_at(&mut self, at: egui::Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
    }
}

#[test]
fn hovering_a_row_offers_source_code_and_scroll_to_bar() {
    let mut rig = Rig::new(history(vec![
        event(0, LogLevel::Info, "first"),
        event(1, LogLevel::Info, "second"),
    ]));
    rig.settle();
    let top = rig.row_top("second");
    // Not hovered: the actions are not there to click.
    assert!(!rig.text().contains("Source code"));
    rig.frame(vec![Event::PointerMoved(pos2(60.0, top.y + 20.0))]);
    rig.frame(Vec::new());
    // Source code is the left of the two buttons, Scroll to bar the right.
    rig.press_at(pos2(WIDTH - 54.0 + 11.0, top.y + 2.0 + 11.0));
    assert_eq!(rig.actions, [LogsAction::Source(1)]);
    rig.actions.clear();
    rig.frame(vec![Event::PointerMoved(pos2(60.0, top.y + 20.0))]);
    rig.press_at(pos2(WIDTH - 30.0 + 11.0, top.y + 2.0 + 11.0));
    assert_eq!(rig.actions, [LogsAction::ScrollToBar(1)]);
}

impl Rig {
    fn right_click_at(&mut self, at: egui::Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle();
    }

    /// Where the text `needle` is drawn, and the popup area around it (the menu).
    fn text_and_menu(&self, needle: &str) -> Option<(egui::Pos2, Rect)> {
        let at = self
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })?;
        let menu = self.ctx.memory(|memory| {
            memory
                .layer_ids()
                .filter(|layer| layer.order != egui::Order::Middle)
                .filter_map(|layer| memory.area_rect(layer.id))
                .filter(|rect| rect.contains(at))
                .min_by(|a, b| a.width().total_cmp(&b.width()))
        })?;
        Some((at, menu))
    }
}

#[test]
fn the_row_menu_is_narrow_and_can_clear_the_logs() {
    let mut rig = Rig::new(history(vec![
        event(0, LogLevel::Info, "first"),
        event(1, LogLevel::Info, "second"),
    ]));
    rig.settle();
    let top = rig.row_top("second");
    rig.right_click_at(pos2(60.0, top.y + 20.0));
    assert!(rig.text().contains("Clear logs"), "{}", rig.text());
    let (clear, _) = rig
        .text_and_menu("Clear logs")
        .expect("the row menu is open with Clear logs");
    let (_, menu) = rig.text_and_menu("Source code").expect("the menu");
    assert!(
        menu.width() <= MENU_WIDTH + 24.0,
        "the menu is {} wide",
        menu.width()
    );
    for entry in ["Scroll to bar", "Copy message", "Copy with time"] {
        assert!(rig.text().contains(entry), "{entry}");
    }
    rig.actions.clear();
    rig.press_at(clear);
    assert_eq!(rig.actions, [LogsAction::Clear]);
}

#[test]
fn a_filter_that_keeps_the_row_count_still_relays_the_rows_out() {
    let mut events = Vec::new();
    for n in 0..100 {
        events.push(event(n, LogLevel::Info, "one line"));
    }
    for n in 100..200 {
        events.push(event(n, LogLevel::Error, "three\nlines\nhere"));
    }
    let mut rig = Rig::new(history(events));
    rig.state.filter.levels.error = false;
    rig.settle();
    let single = rig.state.heights.total();
    // Same number of rows, other messages: the heights must follow the messages.
    rig.state.filter.levels.error = true;
    rig.state.filter.levels.info = false;
    rig.settle();
    let triple = rig.state.heights.total();
    assert!(triple > single * 1.5, "{single} then {triple}");
}

// MARK: Gallery

/// The same messages, each written on the bar its time names.
fn mixed_on_bars() -> Vec<LogEvent> {
    mixed_messages()
        .into_iter()
        .map(|event| LogEvent {
            bar_time_ms: event.time_ms,
            ..event
        })
        .collect()
}

fn mixed_messages() -> Vec<LogEvent> {
    let mut events = vec![event(0, LogLevel::Info, "Script initialized")];
    for bar in 1..6u64 {
        events.push(event(
            bar * 3,
            LogLevel::Info,
            &format!("Bullish bar {bar}: close=101.{bar}2345678"),
        ));
        events.push(event(
            bar * 3 + 1,
            LogLevel::Warning,
            &format!("Bearish bar: {bar}"),
        ));
    }
    events.push(event(40, LogLevel::Error, "Unexpected missing close"));
    events.push(event(
        41,
        LogLevel::Info,
        "Levels:\n  support    98.50\n  resistance 104.25\n  pivot      101.375",
    ));
    events.push(event(
        42,
        LogLevel::Info,
        &format!("token {}", "x".repeat(120)),
    ));
    events
}

type Setup = fn(&mut LogsState, &mut LogHistory, &mut bool);

/// The pane as pictures, in both themes. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_pane() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::{self, AppTheme};
    if gallery_dir().is_none() {
        return;
    }
    let scenes: Vec<(&str, f32, Setup)> = vec![
        ("empty", 320.0, |_, _, _| {}),
        ("embedded", 520.0, |state, history, _| {
            state.embedded = true;
            *history = history_of(mixed_messages());
        }),
        ("empty-narrow", 240.0, |_, _, _| {}),
        ("empty-wide", 520.0, |_, _, _| {}),
        ("populated", 320.0, |_, history, _| {
            *history = history_of(mixed_messages())
        }),
        ("narrow", 240.0, |_, history, _| {
            *history = history_of(mixed_messages())
        }),
        ("wide", 480.0, |_, history, _| {
            *history = history_of(mixed_messages())
        }),
        ("search", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.filter.query = "bar".to_owned();
        }),
        ("search-regex-invalid", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.filter.options.regex = true;
            state.filter.query = "(bar".to_owned();
        }),
        ("range", 520.0, |state, history, _| {
            *history = history_of(mixed_on_bars());
            state.set_range(
                1_700_000_000_000 + 20 * 60_000,
                1_700_000_000_000 + 50 * 60_000,
            );
        }),
        ("range-picking", 520.0, |state, history, _| {
            *history = history_of(mixed_on_bars());
            state.picking = true;
        }),
        ("errors-only", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.filter.levels.info = false;
            state.filter.levels.warning = false;
        }),
        ("logging-off", 320.0, |_, _, logging| *logging = false),
        ("no-results", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.filter.query = "zzz".to_owned();
        }),
    ];
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (name, width, setup) in &scenes {
            let mut screen = Screen::new(vec2(*width, 560.0));
            theme::apply(&screen.ctx, app_theme);
            let mut state = LogsState::default();
            let mut history = LogHistory::default();
            let mut logging = true;
            setup(&mut state, &mut history, &mut logging);
            let scripts = scripts();
            let mut last = None;
            for _ in 0..4 {
                theme::sync(&screen.ctx);
                last = Some(screen.run(Vec::new(), |ui| {
                    state.select_valid(&scripts);
                    let feed = Feed {
                        epoch: 1,
                        history: &history,
                        logging,
                    };
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(theme::chrome()))
                        .show(ui, |ui| {
                            show(ui, &mut state, &scripts, Some(feed));
                        });
                }));
            }
            screen.save(
                &format!("pine-logs-{name}-{theme_name}"),
                last.expect("a frame"),
            );
        }
        // A hovered row, with its two actions.
        let mut screen = Screen::new(vec2(320.0, 560.0));
        theme::apply(&screen.ctx, app_theme);
        let mut state = LogsState::default();
        let history = history_of(mixed_messages());
        let scripts = scripts();
        let mut hover = Vec::new();
        let mut last = None;
        for _ in 0..5 {
            theme::sync(&screen.ctx);
            let output = screen.run(std::mem::take(&mut hover), |ui| {
                state.select_valid(&scripts);
                let feed = Feed {
                    epoch: 1,
                    history: &history,
                    logging: true,
                };
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme::chrome()))
                    .show(ui, |ui| {
                        show(ui, &mut state, &scripts, Some(feed));
                    });
            });
            let row = output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains("Unexpected") => {
                        Some(text.pos)
                    }
                    _ => None,
                });
            if let Some(pos) = row {
                hover = vec![Event::PointerMoved(pos2(60.0, pos.y + 6.0))];
            }
            last = Some(output);
        }
        screen.save(
            &format!("pine-logs-hover-{theme_name}"),
            last.expect("a frame"),
        );
    }
}

fn history_of(events: Vec<LogEvent>) -> LogHistory {
    history(events)
}

fn script_names_drawn(rig: &mut Rig) -> bool {
    rig.settle();
    rig.text().contains("Alpha")
}

#[test]
fn the_script_is_chosen_in_the_docked_title_and_only_when_there_is_a_choice() {
    // A tab shows one script, whatever it is fed: nothing to choose, nothing drawn.
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.state.embedded = true;
    assert!(!script_names_drawn(&mut rig));
    rig.scripts.truncate(1);
    assert!(!script_names_drawn(&mut rig));

    // The docked pane offers the choice between two, and nothing for one.
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    assert!(script_names_drawn(&mut rig));
    rig.scripts.truncate(1);
    assert!(!script_names_drawn(&mut rig));
}

#[test]
fn the_levels_menu_is_as_narrow_as_its_rows() {
    let mut rig = Rig::new(history(vec![event(0, LogLevel::Info, "hello")]));
    rig.state.embedded = true;
    rig.settle();
    // The filter button is the rightmost icon of the toolbar.
    let at = pos2(WIDTH - MARGIN - BUTTON / 2.0, 23.0);
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![button(true)]);
    rig.frame(vec![button(false)]);
    rig.settle();
    let (_, menu) = rig
        .text_and_menu("Warning")
        .expect("the levels menu is open");
    assert!(menu.width() < 140.0, "the menu is {} wide", menu.width());
}
