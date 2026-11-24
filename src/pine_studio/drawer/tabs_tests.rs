//! The drawer's tabs at the sizes a person can leave it at: none of them draws past the room it is
//! given (the body scrolls instead), at a quarter of the width and at a hundredth of the height.
//! With `DEGENVIEW_GALLERY_DIR` set, each frame is also written as a picture to look at.

use std::time::Instant;

use eframe::egui::{Ui, vec2};

use crate::pine::builtins::alert::frequency::AlertFrequency;
use crate::pine::compiler::program::VisualOutput;
use crate::pine::compiler::program::{InputControl, InputDefinition, InputSchema, InputValue};
use crate::pine::logs::{LogBatch, LogEvent, LogHistory, LogLevel};
use crate::pine::runtime::state::AlertEvent;
use crate::pine::strategy::{StrategyReport, Trade};
use crate::pine::syntax::token::{
    Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourcePosition, SourceRange,
};
use crate::pine_studio::drawer;
use crate::pine_studio::profiler::Status;
use crate::pine_studio::profiler::ui::{self, TabView};
use crate::ui::pine::logs_view::{self, Feed, LogsState, ScriptEntry};
use crate::ui::render_tests::{Screen, gallery_dir};

const SIZES: [(f32, f32); 5] = [
    (320.0, 90.0),
    (480.0, 140.0),
    (900.0, 200.0),
    (1400.0, 300.0),
    (900.0, 640.0),
];

/// What the tabs draw that is data, not interface text: the script's own lines and alerts, the
/// compiler's messages and codes, log lines, a symbol, and the words the tests hand a tab as a
/// status or an input's title.
const DATA: &[&str] = &[
    "BTCUSDT",
    "total",
    "sq(",
    "plot(",
    "loop for i",
    "w = weight",
    "PINE",
    "log.info(",
    "input.int(",
    "strategy.entry(",
    "Crossed the signal line",
    "signal",
    "2023-11-14",
    "The argument 'length'",
    "Unused variable",
    "Undefined variable",
    "is called in a loop",
    "The input",
    "The script did not compile",
    "The chart has no bars yet",
];

/// Draws `body` over several frames in a window of `size`, asserting it takes no more than the
/// window, and writes the last frame when a gallery folder is set. It is drawn twice: in English
/// and in the pseudo-locale, whose longer text must fit as well.
fn fits(tab: &str, state: &str, size: (f32, f32), mut body: impl FnMut(&mut Ui)) {
    use crate::i18n::{Locale, with_locale};
    for (locale, suffix) in [(Locale::ENGLISH, ""), (Locale::PSEUDO, "-pseudo")] {
        with_locale(locale, || {
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
                        "{tab} ({state}{suffix}) at {size:?} used {:?} of {:?}",
                        used.size(),
                        room.size()
                    );
                }));
            }
            if locale == Locale::PSEUDO {
                let loose = crate::ui::trade::pseudo_text::untranslated(
                    last.as_ref().expect("a frame"),
                    DATA,
                );
                assert!(
                    loose.is_empty(),
                    "{tab} ({state}) has text that skipped tr!: {loose:?}"
                );
            }
            if gallery_dir().is_some() {
                screen.save(
                    &format!("drawer-{tab}-{state}{suffix}-{}x{}", size.0, size.1),
                    last.expect("a frame"),
                );
            }
        });
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
    strategy_of((1..=trades).map(|id| if id % 3 == 0 { -40.0 } else { 65.0 }))
}

/// A strategy that closed a trade for each of `profits`, three bars apart, on 10,000 of capital,
/// its equity the closed profits plus a little open-trade drift.
fn strategy_of(profits: impl Iterator<Item = f64>) -> StrategyReport {
    let trades: Vec<Trade> = profits
        .enumerate()
        .map(|(index, profit)| trade(index + 1, index % 2 == 1, profit))
        .collect();
    let capital = 10_000.0;
    let bars = trades.len() * 3 + 6;
    let equity = (0..bars)
        .map(|bar| {
            let closed: f64 = trades
                .iter()
                .filter(|trade| trade.exit_bar <= bar)
                .map(|trade| trade.profit)
                .sum();
            capital + closed + (bar as f64 * 0.9).sin() * 12.0
        })
        .collect();
    StrategyReport {
        settings: crate::pine::strategy::StrategySettings {
            initial_capital: capital,
            ..Default::default()
        },
        trades,
        open_trades: Vec::new(),
        equity,
        open_profit: 0.0,
        chart: crate::model::ChartKind::Standard,
    }
}

/// The painted figures reach the accessibility tree: a screen reader finds each one by name.
#[test]
fn the_report_figures_are_named_for_a_screen_reader() {
    use crate::pine::strategy::monte_carlo::MonteCarlo;
    use crate::ui::accessibility::AccessNodes;
    use crate::ui::pine::report::PineReportFormat;
    use eframe::egui::{Context, Pos2, RawInput, Rect};

    let report = strategy(24);
    let simulated = MonteCarlo::run(&report).unwrap();
    let wanted = [
        format!(
            "Net profit {}, {}",
            PineReportFormat::signed_money(report.net_profit()),
            PineReportFormat::percent(report.return_fraction(), true)
        ),
        format!(
            "Win rate: {}, {} of 24",
            PineReportFormat::percent(report.win_rate().unwrap(), false),
            report.win_count()
        ),
        "Closed trades: 24".to_owned(),
        format!(
            "Chance of a loss: {}",
            PineReportFormat::percent(simulated.loss_probability, false)
        ),
    ];

    let ctx = Context::default();
    let mut nodes = AccessNodes::start(&ctx);
    let visual = VisualOutput {
        strategy: Some(report),
        ..Default::default()
    };
    for frame in 0..3 {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 1400.0))),
            time: Some(f64::from(frame) * 0.1),
            ..Default::default()
        };
        nodes.absorb(ctx.run_ui(raw, |ui| drawer::report(ui, Some(&visual))));
    }
    for label in &wanted {
        assert!(nodes.find(label).is_some(), "no node named {label:?}");
    }
}

#[test]
fn inputs_fit_the_drawer() {
    use crate::pine::syntax::token::ValueType;
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
            let values = crate::pine_studio::inputs::InputValues::new();
            fits("inputs", state, size, |ui| {
                crate::pine_studio::inputs::show(ui, schema, &values);
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

/// The toolbar's button, clicked for real, puts every problem on the clipboard in one command (a
/// second copy in the same frame would leave only the last).
#[test]
fn the_copy_button_copies_every_problem_at_once() {
    use DiagnosticSeverity::{Error, Warning};
    use eframe::egui::{Event, Modifiers, PointerButton, pos2};
    let list = vec![
        diagnostic("E1", Error, 3, "first error"),
        diagnostic("E2", Error, 5, "second error"),
        diagnostic("E3", Error, 7, "third error"),
        diagnostic("W1", Warning, 9, "a warning"),
    ];
    let mut screen = Screen::new(vec2(900.0, 300.0));
    let mut copied = None::<Instant>;
    let mut copies: Vec<String> = Vec::new();
    let frame = |screen: &mut Screen, events: Vec<Event>, copied: &mut Option<Instant>| {
        let output = screen.run(events, |ui| {
            drawer::problems(ui, &list, copied);
        });
        output
            .platform_output
            .commands
            .into_iter()
            .filter_map(|command| match command {
                eframe::egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    frame(&mut screen, Vec::new(), &mut copied);
    // The button is somewhere along the toolbar: click along it until one lands.
    'scan: for x in (700..890).step_by(6) {
        let at = pos2(x as f32, 23.0);
        frame(&mut screen, vec![Event::PointerMoved(at)], &mut copied);
        for pressed in [true, false] {
            copies.extend(frame(
                &mut screen,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }],
                &mut copied,
            ));
        }
        if !copies.is_empty() {
            break 'scan;
        }
    }
    assert_eq!(copies.len(), 1, "one copy: {copies:?}");
    let lines: Vec<&str> = copies[0].lines().collect();
    assert_eq!(lines.len(), 4, "every problem is in it: {:?}", copies[0]);
    assert!(lines[0].contains("first error") && lines[3].contains("a warning"));
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
    let cases: [(&str, Option<VisualOutput>); 6] = [
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
            "few-trades",
            Some(VisualOutput {
                strategy: Some(strategy(4)),
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

/// The Report tab in both themes, narrow and wide, with a strategy that wins, one that loses, one
/// with too few trades to simulate and one with none, and the pointer on a chart.
#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_report() {
    use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
    use eframe::egui::{Event, pos2};
    let _restore = RestoreDefaultTheme;
    let losing = strategy_of((1..=36).map(|id| if id % 3 == 0 { 70.0 } else { -45.0 }));
    // More trades than the simulation takes; the equity stays short so the picture is quick.
    let mut too_many = strategy(4);
    too_many.trades = (1..=20_001).map(|id| trade(id, true, 1.0)).collect();
    let cases = [
        ("winning", strategy(24)),
        ("losing", losing),
        ("few", strategy(4)),
        ("too-many", too_many),
        ("empty", strategy(0)),
    ];
    let sizes = [("narrow", 380.0, 1_000.0), ("wide", 1_000.0, 900.0)];
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (state, report) in &cases {
            let visual = VisualOutput {
                strategy: Some(report.clone()),
                ..Default::default()
            };
            for (size_name, width, height) in sizes {
                let hovers: &[(&str, Option<(f32, f32)>)] = match (*state, size_name) {
                    ("winning", "narrow") => &[("", None), ("-equity", Some((200.0, 395.0)))],
                    ("winning", "wide") => &[("", None), ("-fan", Some((300.0, 640.0)))],
                    _ => &[("", None)],
                };
                for (suffix, hover) in hovers {
                    let mut screen = Screen::new(vec2(width, height));
                    theme::apply(&screen.ctx, app_theme);
                    let mut last = None;
                    for _ in 0..4 {
                        let events = hover
                            .map(|(x, y)| vec![Event::PointerMoved(pos2(x, y))])
                            .unwrap_or_default();
                        last = Some(screen.run(events, |ui| {
                            ui.painter()
                                .rect_filled(ui.max_rect(), 0.0, theme::background());
                            drawer::report(ui, Some(&visual));
                        }));
                    }
                    screen.save(
                        &format!("report-{state}-{size_name}-{theme_name}{suffix}"),
                        last.expect("a frame"),
                    );
                }
            }
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
                logs_view::show(ui, &mut view, &scripts, Some(feed));
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
                ui::tab(ui, &view);
            });
        }
    }
}

/// A finished profile of a small script, to draw.
fn finished_profile() -> crate::pine_studio::profiler::Profile {
    use crate::pine_studio::preview::RunIdentity;
    use crate::pine_studio::profiler::{Profile, ProfileKey};
    let body =
        "sq(x) =>\n    x * x\ntotal = 0.0\nfor i = 1 to 20\n    total += sq(i)\nplot(total)\n";
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let report = crate::pine::profile::support::report(body, 40, 1_000);
    Profile::new(
        ProfileKey::new("demo", &source, RunIdentity::for_test()),
        "BTCUSDT \u{b7} 1H".to_owned(),
        report,
        source.into(),
        1_234_000,
    )
}

/// A profile is made in one language and read in another when the language changes: its words are
/// written again, not kept.
#[test]
fn a_profile_writes_its_words_again_when_the_language_changes() {
    use crate::i18n::{Locale, with_locale};
    let profile = finished_profile();
    let label = |profile: &crate::pine_studio::profiler::Profile| {
        profile
            .marks()
            .summary
            .first()
            .map(|row| row.label)
            .expect("a summary row")
    };
    assert!(
        !label(&profile).starts_with('['),
        "English: {}",
        label(&profile)
    );
    with_locale(Locale::PSEUDO, || {
        assert!(
            label(&profile).starts_with('['),
            "pseudo: {}",
            label(&profile)
        );
    });
    assert!(!label(&profile).starts_with('['), "and back");
}

#[test]
fn a_finished_profile_fits_the_drawer_current_or_stale() {
    let profile = finished_profile();
    for (name, status, lines_match) in [
        ("ready", Status::Ready, true),
        ("stale-data", Status::Stale, true),
        ("stale-text", Status::Stale, false),
    ] {
        for size in SIZES {
            fits("profile", name, size, |ui| {
                let view = TabView {
                    status: &status,
                    profile: Some(&profile),
                    lines_match,
                };
                ui::tab(ui, &view);
            });
        }
    }
}

#[test]
fn a_stale_profile_of_the_same_text_is_navigable_and_not_current() {
    use crate::pine_studio::profiler::ui::Freshness;
    let fresh = |status: &Status, lines| Freshness::of(status, lines);
    assert_eq!(
        fresh(&Status::Ready, true),
        Freshness {
            navigable: true,
            current: true
        }
    );
    assert_eq!(
        fresh(&Status::Partial, true),
        Freshness {
            navigable: true,
            current: true
        },
        "a partial profile is of what is on screen"
    );
    assert_eq!(
        fresh(&Status::Stale, true),
        Freshness {
            navigable: true,
            current: false
        },
        "its rows can be followed, but its figures are not current"
    );
    assert_eq!(
        fresh(&Status::Stale, false),
        Freshness {
            navigable: false,
            current: false
        }
    );
}
