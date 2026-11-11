use super::*;
use eframe::egui::{Pos2, RawInput, Rect};

const DAY_MS: i64 = 86_400_000;

fn bars(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|i| {
            let open = 100.0 + i as f64;
            Kline {
                open_time: 1_704_067_200_000 + i as i64 * DAY_MS,
                close_time: 1_704_067_200_000 + (i as i64 + 1) * DAY_MS - 1,
                open,
                high: open + 3.0,
                low: open - 3.0,
                close: if i % 2 == 0 { open + 1.0 } else { open - 1.0 },
                volume: 1.0,
            }
        })
        .collect()
}

fn frame(ctx: &egui::Context, state: &mut StatsViewState, input: Option<Input>) -> Option<Action> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 700.0))),
        ..RawInput::default()
    };
    let mut action = None;
    let mut output = ctx.run_ui(raw, |ui| {
        action = show(ui, state, "BTCUSDT", input);
    });
    output.textures_delta.clear();
    action
}

#[test]
fn percents_are_read_leniently_and_never_negative() {
    assert_eq!(parse_percent("0.5"), Some(0.5));
    assert_eq!(parse_percent(" 1.25 % "), Some(1.25));
    assert_eq!(parse_percent(""), None);
    assert_eq!(parse_percent("abc"), None);
    assert_eq!(parse_percent("-1"), None);
    assert_eq!(parse_percent("NaN"), None);
}

#[test]
fn a_bar_is_placed_in_the_session_by_its_length() {
    assert_eq!(text::after_open(0, 3_600_000), "in the first bar");
    assert_eq!(text::after_open(2, 3_600_000), "bar 2 (2–3 h in)");
    assert_eq!(text::after_open(1, 1_800_000), "bar 1 (0.5–1 h in)");
}

#[test]
fn the_answer_is_worked_once_until_something_changes() {
    let data = bars(40);
    let input = Input {
        chart_id: 1,
        bars: &data,
        bar_ms: DAY_MS,
        hours: MarketHours::Continuous,
        replay: false,
    };
    let mut state = StatsViewState {
        outcome: Outcome::DayUp,
        ..StatsViewState::default()
    };
    let first = state.answer(&input);
    assert_eq!((*first).as_ref().unwrap().estimate.n, 40);

    // Plant a different answer under the same fingerprint: a second call must not recompute it.
    let marker = Stats {
        sessions: 999,
        ..(*first).clone().unwrap()
    };
    state.cached.as_mut().unwrap().answer = Arc::new(Ok(marker));
    assert_eq!((*state.answer(&input)).as_ref().unwrap().sessions, 999);

    // A new bar, a new question or a new condition recomputes.
    let longer = bars(41);
    let grown = Input {
        bars: &longer,
        ..input
    };
    assert_eq!((*state.answer(&grown)).as_ref().unwrap().estimate.n, 41);
    state.outcome = Outcome::PriorHighBreak;
    assert_eq!(
        (*state.answer(&grown)).as_ref().unwrap().outcome,
        Outcome::PriorHighBreak
    );
    state.filter.prior_day = Side::Up;
    assert_eq!(
        (*state.answer(&grown)).as_ref().unwrap().filter.prior_day,
        Side::Up
    );
}

#[test]
fn the_panel_draws_every_question_and_the_states_without_an_answer() {
    let data = bars(60);
    let ctx = egui::Context::default();
    let mut state = StatsViewState::default();
    for outcome in Outcome::ALL {
        state.outcome = outcome;
        for _ in 0..2 {
            let input = Input {
                chart_id: 1,
                bars: &data,
                bar_ms: DAY_MS,
                hours: MarketHours::Continuous,
                replay: false,
            };
            assert_eq!(frame(&ctx, &mut state, Some(input)), None);
        }
    }
    // The question that needs hourly bars on daily ones, a week of bars, and no bars at all.
    state.outcome = Outcome::FirstBarHighBreak;
    let input = Input {
        chart_id: 1,
        bars: &data,
        bar_ms: DAY_MS,
        hours: MarketHours::Continuous,
        replay: false,
    };
    frame(&ctx, &mut state, Some(input));
    assert_eq!(
        *state.cached.as_ref().unwrap().answer,
        Err(Unavailable::NeedsIntraday)
    );
    let weekly = Input {
        bar_ms: 7 * DAY_MS,
        ..input
    };
    frame(&ctx, &mut state, Some(weekly));
    assert_eq!(
        *state.cached.as_ref().unwrap().answer,
        Err(Unavailable::TooCoarse)
    );
    frame(&ctx, &mut state, None);
}

#[test]
fn typed_gap_limits_become_conditions() {
    let data = bars(30);
    let ctx = egui::Context::default();
    let mut state = StatsViewState {
        outcome: Outcome::DayUp,
        ..StatsViewState::default()
    };
    state.gap_min = "0.1".to_owned();
    state.gap_max = "oops".to_owned();
    let input = Input {
        chart_id: 1,
        bars: &data,
        bar_ms: DAY_MS,
        hours: MarketHours::Continuous,
        replay: false,
    };
    frame(&ctx, &mut state, Some(input));
    assert_eq!(state.filter.gap_min_pct, Some(0.1));
    assert_eq!(state.filter.gap_max_pct, None);
}

#[test]
fn numbers_and_times_read_the_way_the_panel_writes_them() {
    use crate::model::session_stats::Estimate;
    assert_eq!(text::percent(0.919), "91.9%");
    assert_eq!(text::percent(1.0), "100.0%");
    assert_eq!(text::count(1_234_567), "1,234,567");
    let estimate = Estimate { hits: 236, n: 260 };
    assert_eq!(text::fraction(&estimate), "236/260");
    assert_eq!(text::share(&estimate), "90.8% · 236/260");
    assert_eq!(text::share(&Estimate::default()), "no days");
    assert_eq!(text::date((2024, 3, 5)), "2024-03-05");
}

#[test]
fn the_copied_text_carries_the_sample_the_interval_and_the_reminder() {
    let data = bars(60);
    let stats = session_stats::compute(
        &data,
        DAY_MS,
        MarketHours::Continuous,
        Outcome::DayUp,
        &Filter::default(),
    )
    .unwrap();
    let copied = text::summary("BTCUSDT 1D", &stats, DAY_MS);
    assert!(
        copied.starts_with("BTCUSDT 1D · Day closes up on every day"),
        "{copied}"
    );
    assert!(copied.contains("95% CI"));
    assert!(copied.contains("of 60 days"));
    assert!(copied.contains("Older half"));
    assert!(copied.contains("not a prediction"));
    let impossible = Filter {
        gap_min_pct: Some(900.0),
        ..Filter::default()
    };
    let empty = session_stats::compute(
        &data,
        DAY_MS,
        MarketHours::Continuous,
        Outcome::GapFill,
        &impossible,
    )
    .unwrap();
    assert!(text::summary("BTCUSDT 1D", &empty, DAY_MS).contains("No days qualify."));
}

#[test]
fn a_comma_is_a_decimal_point_and_a_bad_field_is_marked() {
    assert_eq!(parse_percent("0,5"), Some(0.5));
    assert!(StatsViewState::gap_text_invalid("abc"));
    assert!(!StatsViewState::gap_text_invalid(""));
    assert!(!StatsViewState::gap_text_invalid("1.5"));
}

#[test]
fn clearing_the_conditions_clears_the_typed_limits_too() {
    let mut state = StatsViewState::default();
    state.filter.gap = Side::Up;
    state.gap_min = "0.1".to_owned();
    state.clear_conditions();
    assert_eq!(state.filter, Filter::default());
    assert!(state.gap_min.is_empty() && state.gap_max.is_empty());
}

/// Hourly bars of a US stock: 7 regular-session hours a weekday, in January so New York is UTC-5.
fn stock_hours() -> Vec<Kline> {
    let mut out = Vec::new();
    let mut close = 100.0_f64;
    let hour = 3_600_000_i64;
    let mut day = 1_704_153_600_000_i64; // 2024-01-02 00:00 UTC
    for d in 0..260_i64 {
        let weekday = ((day / DAY_MS) + 3).rem_euclid(7);
        if weekday < 5 {
            let gap = ((d as f64 * 1.3).sin()) * 0.01 * close;
            let mut price = close + gap;
            for h in 14..21_i64 {
                let swing = (1.0 + ((d * 7 + h) as f64 * 0.7).cos().abs()) * 0.004 * price;
                let open = price;
                let next = open + (((d * 13 + h) as f64 * 0.9).sin()) * swing;
                out.push(Kline {
                    open_time: day + h * hour,
                    close_time: day + (h + 1) * hour - 1,
                    open,
                    high: open.max(next) + swing * 0.5,
                    low: open.min(next) - swing * 0.5,
                    close: next,
                    volume: 1.0,
                });
                price = next;
            }
            close = price;
        }
        day += DAY_MS;
    }
    out
}

/// Pictures of the panel for a person to look at. Run by hand:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib session_stats_gallery -- --ignored`.
#[test]
#[ignore = "writes pictures; run by hand with DEGENVIEW_GALLERY_DIR"]
fn session_stats_gallery() {
    use crate::ui::render_tests::Screen;

    // Four years of weekdays with gaps that sometimes fill.
    let mut daily = Vec::new();
    let mut close = 100.0_f64;
    let mut time = 1_577_836_800_000_i64;
    for i in 0..1_400_i64 {
        time += DAY_MS;
        if ((time / DAY_MS) + 3).rem_euclid(7) >= 5 {
            continue;
        }
        let gap = ((i as f64 * 1.7).sin()) * 0.012 * close;
        let open = close + gap;
        let swing = (1.0 + (i as f64 * 0.9).cos().abs()) * 0.006 * close;
        let next = open + ((i as f64 * 0.37).sin() + 0.1) * swing * 0.9;
        daily.push(Kline {
            open_time: time,
            close_time: time + DAY_MS - 1,
            open,
            high: open.max(next) + swing * 0.4,
            low: open.min(next) - swing * 0.4,
            close: next,
            volume: 1.0,
        });
        close = next;
    }
    let hourly = stock_hours();

    struct Scene {
        name: &'static str,
        width: f32,
        outcome: Outcome,
        bars: &'static str,
        narrow: bool,
        few: bool,
    }
    let scenes = [
        Scene {
            name: "gap-fill",
            width: 340.0,
            outcome: Outcome::GapFill,
            bars: "daily",
            narrow: false,
            few: false,
        },
        Scene {
            name: "gap-fill-narrowed",
            width: 340.0,
            outcome: Outcome::GapFill,
            bars: "daily",
            narrow: true,
            few: false,
        },
        Scene {
            name: "gap-fill-small",
            width: 340.0,
            outcome: Outcome::GapFill,
            bars: "daily",
            narrow: true,
            few: true,
        },
        Scene {
            name: "min-width",
            width: 280.0,
            outcome: Outcome::PriorHighBreak,
            bars: "daily",
            narrow: false,
            few: false,
        },
        Scene {
            name: "wide",
            width: 520.0,
            outcome: Outcome::DayUp,
            bars: "daily",
            narrow: false,
            few: false,
        },
        Scene {
            name: "hourly-timing",
            width: 340.0,
            outcome: Outcome::GapFill,
            bars: "hourly",
            narrow: false,
            few: false,
        },
        Scene {
            name: "needs-hourly",
            width: 340.0,
            outcome: Outcome::FirstBarHighBreak,
            bars: "daily",
            narrow: false,
            few: false,
        },
    ];
    for (theme_name, app_theme) in [
        ("dark", crate::ui::theme::AppTheme::Dark),
        ("light", crate::ui::theme::AppTheme::Light),
    ] {
        for scene in &scenes {
            let mut screen = Screen::new(vec2(scene.width, 900.0));
            crate::ui::theme::apply(&screen.ctx, app_theme);
            let mut state = StatsViewState {
                outcome: scene.outcome,
                ..StatsViewState::default()
            };
            if scene.narrow {
                state.filter.weekdays = WeekdaySet::from_names(&["Tue", "Wed"]).unwrap();
                state.filter.gap = Side::Up;
                state.gap_min = "0.1".to_owned();
                state.gap_max = if scene.few {
                    "0.12".to_owned()
                } else {
                    "0.9".to_owned()
                };
            }
            let (data, bar_ms, hours) = match scene.bars {
                "hourly" => (&hourly, 3_600_000, MarketHours::UsStock),
                _ => (&daily, DAY_MS, MarketHours::UsStock),
            };
            let input = Input {
                chart_id: 1,
                bars: data,
                bar_ms,
                hours,
                replay: scene.few,
            };
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let full = ui.max_rect();
                    ui.painter()
                        .rect_filled(full, 0.0, crate::ui::theme::chrome());
                    show(ui, &mut state, "AAPL 1D", Some(input));
                }));
            }
            screen.save(
                &format!("session-stats-{}-{theme_name}", scene.name),
                last.take().expect("a frame"),
            );
        }
    }
}
