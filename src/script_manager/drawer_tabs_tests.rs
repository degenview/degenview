//! The drawer's tabs at the sizes a person can leave it at: none of them draws past the room it is
//! given (the body scrolls instead), at a quarter of the width and at a hundredth of the height.
//! With `DEGENVIEW_GALLERY_DIR` set, each frame is also written as a picture to look at.

use std::time::Instant;

use eframe::egui::{Ui, vec2};

use super::drawer;
use super::profile_ui::{self, TabView};
use super::profiler::Status;
use crate::pine::alert_frequency::AlertFrequency;
use crate::pine::logs::{LogBatch, LogEvent, LogHistory, LogLevel};
use crate::pine::program::VisualOutput;
use crate::pine::program::{InputControl, InputDefinition, InputSchema, InputValue};
use crate::pine::state::AlertEvent;
use crate::pine::strategy::{StrategyReport, Trade};
use crate::pine::token::{
    Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourcePosition, SourceRange,
};
use crate::ui::pine_logs_view::{self, Feed, LogsState, ScriptEntry};
use crate::ui::render_tests::{Screen, gallery_dir};

const SIZES: [(f32, f32); 5] = [
    (320.0, 90.0),
    (480.0, 140.0),
    (900.0, 200.0),
    (1400.0, 300.0),
    (900.0, 640.0),
];

/// Draws `body` over several frames in a window of `size`, asserting it takes no more than the
/// window, and writes the last frame when a gallery folder is set.
fn fits(tab: &str, state: &str, size: (f32, f32), mut body: impl FnMut(&mut Ui)) {
    let mut screen = Screen::new(vec2(size.0, size.1));
    let mut last = None;
    for _ in 0..4 {
        last = Some(screen.run(Vec::new(), |ui| {
            let room = ui.max_rect();
            ui.painter()
                .rect_filled(room, 0.0, crate::ui::theme::background());
            body(ui);
            let used = ui.min_rect();
            assert!(
                used.height() <= room.height() + 1.0 && used.width() <= room.width() + 1.0,
                "{tab} ({state}) at {size:?} used {:?} of {:?}",
                used.size(),
                room.size()
            );
        }));
    }
    if gallery_dir().is_some() {
        screen.save(
            &format!("drawer-{tab}-{state}-{}x{}", size.0, size.1),
            last.expect("a frame"),
        );
    }
}

fn diagnostic(code: &str, severity: DiagnosticSeverity, line: usize, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity,
        category: DiagnosticCategory::Semantic,
        message: message.into(),
        range: SourceRange::new(
            SourcePosition::new(line, 4, line * 20),
            SourcePosition::new(line, 9, line * 20 + 5),
        ),
    }
}

fn trade(id: usize, long: bool, profit: f64) -> Trade {
    Trade {
        id,
        entry_id: "Long".into(),
        exit_id: "Exit".into(),
        is_long: long,
        quantity: 1.0,
        entry_bar: id * 3,
        entry_time: 0,
        entry_price: 100.0 + id as f64,
        exit_bar: id * 3 + 2,
        exit_time: 0,
        exit_price: 101.0 + id as f64,
        commission: 0.0,
        profit,
        ..Trade::default()
    }
}

fn alert(id: usize, live: bool) -> AlertEvent {
    AlertEvent {
        id,
        site: 0,
        bar: id,
        time: 1_700_000_000_000 + id as i64 * 60_000,
        message: format!("Crossed the signal line on bar {id}"),
        frequency: AlertFrequency::OncePerBar,
        is_realtime: live,
        is_confirmed: true,
    }
}

fn strategy(trades: usize) -> StrategyReport {
    StrategyReport {
        settings: Default::default(),
        trades: (1..=trades)
            .map(|id| trade(id, id % 2 == 0, if id % 3 == 0 { -40.0 } else { 65.0 }))
            .collect(),
        open_trades: Vec::new(),
        equity: (0..80)
            .map(|i| 1_000_000.0 + (i as f64 * 0.4).sin() * 20_000.0 + i as f64 * 900.0)
            .collect(),
        open_profit: 0.0,
        chart: crate::model::ChartKind::Standard,
    }
}

#[test]
fn inputs_fit_the_drawer() {
    use crate::pine::token::ValueType;
    let definition = |id: &str, kind, default| InputDefinition {
        id: id.into(),
        kind,
        control: InputControl::Standard,
        default_value: default,
        title: Some(format!("The {id} input")),
        tooltip: None,
        group: None,
        inline: None,
        confirm: false,
        min_value: None,
        max_value: None,
        step: None,
        options: None,
        option_titles: None,
        active: None,
    };
    let six = InputSchema {
        inputs: (0..6)
            .map(|index| {
                definition(
                    &format!("input{index}"),
                    ValueType::Int,
                    InputValue::Int(14),
                )
            })
            .collect(),
    };
    let cases = [("none", InputSchema::default()), ("six", six)];
    for (state, schema) in &cases {
        for size in SIZES {
            let values = super::inputs::InputValues::new();
            fits("inputs", state, size, |ui| {
                super::inputs::show(ui, schema, &values);
            });
        }
    }
}

#[test]
fn problems_fit_the_drawer() {
    use DiagnosticSeverity::{Error, Warning};
    let long =
        "The argument 'length' of ta.sma() must be a series int, but a series float was given here";
    let cases: [(&str, Vec<Diagnostic>); 4] = [
        ("none", Vec::new()),
        ("errors", vec![diagnostic("PINE1001", Error, 3, long); 6]),
        (
            "warnings",
            vec![diagnostic(
                "PINE7001",
                Warning,
                9,
                "plot() is called in a loop",
            )],
        ),
        (
            "mixed",
            vec![
                diagnostic("PINE1001", Error, 3, long),
                diagnostic("PINE2002", Warning, 8, "Unused variable 'x'"),
            ],
        ),
    ];
    for (state, list) in &cases {
        for size in SIZES {
            let mut copied = None::<Instant>;
            fits("problems", state, size, |ui| {
                drawer::problems(ui, list, &mut copied);
            });
        }
    }
}

#[test]
fn problems_are_counted_in_words() {
    use DiagnosticSeverity::{Error, Warning};
    let one = |severity| diagnostic("X", severity, 1, "m");
    assert_eq!(drawer::problems_summary(&[]), "No problems");
    assert_eq!(drawer::problems_summary(&[one(Error)]), "1 error");
    assert_eq!(
        drawer::problems_summary(&[one(Warning), one(Warning)]),
        "2 warnings"
    );
    assert_eq!(
        drawer::problems_summary(&[one(Error), one(Error), one(Warning)]),
        "2 errors \u{b7} 1 warning"
    );
}

#[test]
fn report_fits_the_drawer() {
    let cases: [(&str, Option<VisualOutput>); 5] = [
        ("none", None),
        (
            "alerts",
            Some(VisualOutput {
                alerts: (0..7).map(|i| alert(i, i % 3 == 0)).collect(),
                ..Default::default()
            }),
        ),
        (
            "strategy",
            Some(VisualOutput {
                strategy: Some(strategy(24)),
                ..Default::default()
            }),
        ),
        (
            "strategy-no-trades",
            Some(VisualOutput {
                strategy: Some(strategy(0)),
                ..Default::default()
            }),
        ),
        (
            "both",
            Some(VisualOutput {
                strategy: Some(strategy(130)),
                alerts: (0..120).map(|i| alert(i, i % 4 == 0)).collect(),
                ..Default::default()
            }),
        ),
    ];
    for (state, visual) in &cases {
        for size in SIZES {
            fits("report", state, size, |ui| {
                drawer::report(ui, visual.as_ref());
            });
        }
    }
}

#[test]
fn report_says_what_it_holds() {
    let report = strategy(24);
    assert_eq!(drawer::report_summary(None, 0), "No report");
    assert_eq!(drawer::report_summary(None, 3), "Indicator \u{b7} 3 alerts");
    assert_eq!(
        drawer::report_summary(Some(&report), 0),
        "Strategy \u{b7} 24 trades"
    );
    assert_eq!(
        drawer::report_summary(Some(&report), 1),
        "Strategy \u{b7} 24 trades \u{b7} 1 alert"
    );
}

fn log_event(sequence: u64, level: LogLevel, message: &str) -> LogEvent {
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
        range: crate::pine::SourceRange::ZERO,
        request: None,
    }
}

#[test]
fn logs_fit_the_drawer() {
    let mut full = LogHistory::default();
    let events: Vec<LogEvent> = (0..40)
        .map(|i| log_event(i, LogLevel::Info, "close 101.25, signal crossed"))
        .collect();
    full.apply(LogBatch {
        reset: true,
        from: 0,
        to: events.len() as u64,
        events,
        dropped: 0,
    });
    let empty = LogHistory::default();
    let scripts = vec![ScriptEntry {
        id: 1,
        label: "Trend strength".to_owned(),
    }];
    for (state, history, logging, search) in [
        ("intro", &empty, true, false),
        ("off", &empty, false, false),
        ("rows", &full, true, false),
        ("search", &full, true, true),
    ] {
        for size in SIZES {
            let mut view = LogsState::default();
            view.embedded = true;
            if search {
                view.filter.query = "signal".to_owned();
                view.set_range(1_700_000_000_000, 1_700_000_600_000);
            }
            view.select_valid(&scripts);
            fits("logs", state, size, |ui| {
                let feed = Feed {
                    epoch: 1,
                    history,
                    logging,
                };
                pine_logs_view::show(ui, &mut view, &scripts, Some(feed));
            });
        }
    }
}

#[test]
fn profile_fits_the_drawer() {
    for status in [
        Status::Off,
        Status::Idle,
        Status::Waiting("The chart has no bars yet"),
        Status::Running {
            done: 120,
            total: 500,
            cancelling: false,
        },
        Status::Failed("The script did not compile".into()),
    ] {
        let state = format!("{status:?}");
        let state: String = state
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .take(12)
            .collect();
        for size in SIZES {
            fits("profile", &state, size, |ui| {
                let view = TabView {
                    status: &status,
                    profile: None,
                    lines_match: true,
                };
                profile_ui::tab(ui, &view);
            });
        }
    }
}
