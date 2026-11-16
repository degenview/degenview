use super::*;

const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

fn window_of(timeframe: &str) -> LineWindow {
    window(Timeframe::parse(timeframe).unwrap())
}

#[test]
fn an_hourly_line_reaches_back_a_month_a_point_an_hour() {
    assert_eq!(
        window_of("1h"),
        LineWindow {
            span_ms: 30 * DAY_MS,
            step_ms: HOUR_MS
        }
    );
}

#[test]
fn a_daily_line_has_sixty_points() {
    assert_eq!(
        window_of("1D"),
        LineWindow {
            span_ms: 60 * DAY_MS,
            step_ms: DAY_MS
        }
    );
}

#[test]
fn a_point_is_never_a_week_and_more_apart_and_the_line_stays_within_a_year() {
    for timeframe in ["1W", "1M", "3M", "1Y"] {
        let line = window_of(timeframe);
        assert_eq!(line.step_ms, 7 * DAY_MS, "{timeframe}");
        assert_eq!(line.span_ms, 364 * DAY_MS, "{timeframe}");
    }
}

#[test]
fn a_short_line_reaches_back_hours_not_a_month() {
    let line = window_of("1m");
    assert_eq!(line.step_ms, 60_000);
    assert_eq!(line.span_ms, 1_000 * 60_000);
    assert_eq!(line.step_minutes(), 1);
    // Between those, a point per timeframe over a thousand of them at most.
    assert_eq!(window_of("15m").span_ms, 1_000 * 15 * 60_000);
}

#[test]
fn no_line_has_more_than_a_thousand_points_or_fewer_than_two() {
    for timeframe in Timeframe::PRESETS {
        let line = window(timeframe);
        let points = line.span_ms / line.step_ms;
        assert!((2..=MAX_POINTS).contains(&points), "{timeframe}: {points}");
    }
}
