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
    assert_eq!(after_open(0, 3_600_000), "in the first bar");
    assert_eq!(after_open(2, 3_600_000), "bar 2 (2–3 h in)");
    assert_eq!(after_open(1, 1_800_000), "bar 1 (0.5–1 h in)");
}

#[test]
fn the_answer_is_worked_once_until_something_changes() {
    let data = bars(40);
    let input = Input {
        chart_id: 1,
        bars: &data,
        bar_ms: DAY_MS,
        hours: MarketHours::Continuous,
    };
    let mut state = StatsViewState {
        outcome: Outcome::DayUp,
        ..StatsViewState::default()
    };
    let first = state.answer(&input).clone();
    assert_eq!(first.as_ref().unwrap().estimate.n, 40);

    // Plant a different answer under the same fingerprint: a second call must not recompute it.
    let marker = Stats {
        sessions: 999,
        ..first.clone().unwrap()
    };
    state.cached.as_mut().unwrap().answer = Ok(marker);
    assert_eq!(state.answer(&input).as_ref().unwrap().sessions, 999);

    // A new bar, a new question or a new condition recomputes.
    let longer = bars(41);
    let grown = Input {
        bars: &longer,
        ..input
    };
    assert_eq!(state.answer(&grown).as_ref().unwrap().estimate.n, 41);
    state.outcome = Outcome::PriorHighBreak;
    assert_eq!(
        state.answer(&grown).as_ref().unwrap().outcome,
        Outcome::PriorHighBreak
    );
    state.filter.prior_day = Side::Up;
    assert_eq!(
        state.answer(&grown).as_ref().unwrap().filter.prior_day,
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
    };
    frame(&ctx, &mut state, Some(input));
    assert_eq!(
        state.cached.as_ref().unwrap().answer,
        Err(Unavailable::NeedsIntraday)
    );
    let weekly = Input {
        bar_ms: 7 * DAY_MS,
        ..input
    };
    frame(&ctx, &mut state, Some(weekly));
    assert_eq!(
        state.cached.as_ref().unwrap().answer,
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
    };
    frame(&ctx, &mut state, Some(input));
    assert_eq!(state.filter.gap_min_pct, Some(0.1));
    assert_eq!(state.filter.gap_max_pct, None);
}

/// A picture of the panel for a person to look at. Run by hand:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib session_stats_gallery -- --ignored`.
#[test]
#[ignore = "writes pictures; run by hand with DEGENVIEW_GALLERY_DIR"]
fn session_stats_gallery() {
    use crate::ui::render_tests::Screen;

    // Four years of weekdays with gaps that sometimes fill, so every part of the answer shows.
    let mut data = Vec::new();
    let mut close = 100.0_f64;
    let mut time = 1_577_836_800_000_i64;
    for i in 0..1_000_i64 {
        time += DAY_MS;
        let weekday = ((time / DAY_MS) + 3).rem_euclid(7);
        if weekday >= 5 {
            continue;
        }
        let gap = ((i as f64 * 1.7).sin()) * 0.012 * close;
        let open = close + gap;
        let swing = (1.0 + (i as f64 * 0.9).cos().abs()) * 0.009 * close;
        let high = open + swing;
        let low = open - swing;
        let next = open + ((i as f64 * 0.37).sin()) * swing * 0.8;
        data.push(Kline {
            open_time: time,
            close_time: time + DAY_MS - 1,
            open,
            high,
            low,
            close: next,
            volume: 1.0,
        });
        close = next;
    }
    for (theme_name, app_theme) in [
        ("dark", crate::ui::theme::AppTheme::Dark),
        ("light", crate::ui::theme::AppTheme::Light),
    ] {
        let mut screen = Screen::new(vec2(340.0, 640.0));
        crate::ui::theme::apply(&screen.ctx, app_theme);
        let mut state = StatsViewState::default();
        let input = Input {
            chart_id: 1,
            bars: &data,
            bar_ms: DAY_MS,
            hours: MarketHours::UsStock,
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
            &format!("session-stats-{theme_name}"),
            last.take().expect("a frame"),
        );

        state.filter.weekdays = WeekdaySet::from_names(&["Tue", "Wed"]).unwrap();
        state.filter.gap = Side::Up;
        state.gap_min = "0.1".to_owned();
        state.gap_max = "0.6".to_owned();
        for _ in 0..4 {
            last = Some(screen.run(Vec::new(), |ui| {
                let full = ui.max_rect();
                ui.painter()
                    .rect_filled(full, 0.0, crate::ui::theme::chrome());
                show(ui, &mut state, "AAPL 1D", Some(input));
            }));
        }
        screen.save(
            &format!("session-stats-narrowed-{theme_name}"),
            last.take().expect("a frame"),
        );
    }
}
