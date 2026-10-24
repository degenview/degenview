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
    rig.state.search_open = true;
    rig.state.filter.options.regex = true;
    rig.state.filter.query = "(".to_owned();
    rig.settle();
    let text = rig.text();
    assert!(text.contains("hello"), "{text}");
    assert!(text.contains("Not a regular expression"), "{text}");
}

#[test]
fn the_start_date_shows_as_a_tag_and_filters_by_absolute_time() {
    let mut rig = Rig::new(history(vec![
        event(0, LogLevel::Info, "early"),
        event(1, LogLevel::Info, "late"),
    ]));
    rig.state.filter.start_ms = Some(1_700_000_030_000);
    rig.settle();
    let text = rig.text();
    assert!(text.contains("From 2023-11-14 22:13"), "{text}");
    assert!(text.contains("late") && !text.contains("early"), "{text}");
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
            state.search_open = true;
            state.filter.query = "bar".to_owned();
        }),
        ("search-regex-invalid", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.search_open = true;
            state.filter.options.regex = true;
            state.filter.query = "(bar".to_owned();
        }),
        ("date", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.filter.start_ms = Some(1_700_000_000_000 + 20 * 60_000);
        }),
        ("date-bar", 320.0, |state, history, _| {
            *history = history_of(mixed_messages());
            state.date_open = true;
            state.date_text = "2023-11-14 22:30".to_owned();
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
    }
}

fn history_of(events: Vec<LogEvent>) -> LogHistory {
    history(events)
}
