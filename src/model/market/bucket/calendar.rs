//! The bars a series can have, counted past the data it holds: where the bar a number of steps
//! before or after another one opens. On a market with closing hours the nights and weekends are
//! not bars. Holidays are not known here and count as weekdays.

use super::{DAY_MS, DAYS_BEFORE_FIRST_MONDAY, Grid, bucket_start_ms, month_index, month_start_ms};
use crate::model::market::session;
use crate::model::{TimeUnit, Timeframe};

/// The grid a series' bars are cut on and their length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Calendar {
    pub grid: Grid,
    pub timeframe: Timeframe,
}

impl Calendar {
    /// Bars from the one that opens at `from` to the moment `to`: a whole number where `to` is a
    /// bar's open, negative before `from`.
    pub fn steps(self, from: i64, to: i64) -> f64 {
        let phase = self.phase(from);
        self.position(to - phase) - self.position(from - phase)
    }

    /// The moment `steps` bars after (before, if negative) the bar that opens at `from`.
    pub fn after(self, from: i64, steps: f64) -> i64 {
        let phase = self.phase(from);
        self.moment(self.position(from - phase) + steps) + phase
    }

    /// How far a series' bars open from the grid's own buckets: a provider may stamp a daily bar
    /// at the hour its session opens.
    fn phase(self, open: i64) -> i64 {
        open - self.grid.bucket_start_ms(self.timeframe, open)
    }

    fn stock(self) -> bool {
        self.grid == Grid::UsEquities
    }

    /// Bars a trading day holds, for an intraday length.
    fn bars_per_day(self) -> i64 {
        let span = if self.stock() {
            session::CLOSE_MS - session::OPEN_MS
        } else {
            DAY_MS
        };
        let length = self.timeframe.nominal_ms();
        (span + length - 1) / length
    }

    /// `t` as a number of bars from a fixed origin, whole at a bar's open.
    fn position(self, t: i64) -> f64 {
        let length = self.timeframe.nominal_ms();
        match self.timeframe.unit() {
            TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour => {
                let per_day = self.bars_per_day();
                let (days, within) = if self.stock() {
                    let day = session::local_day(t);
                    let within = if session::is_weekday(day) {
                        ((t - session::open_ms(day)) as f64 / length as f64)
                            .clamp(0.0, per_day as f64)
                    } else {
                        0.0
                    };
                    (weekdays_before(day), within)
                } else {
                    let day = t.div_euclid(DAY_MS);
                    (day, (t - day * DAY_MS) as f64 / length as f64)
                };
                (days * per_day) as f64 + within
            }
            TimeUnit::Day if self.stock() && self.timeframe.count() == 1 => {
                let day = t.div_euclid(DAY_MS);
                let within = if session::is_weekday(day) {
                    (t - day * DAY_MS) as f64 / DAY_MS as f64
                } else {
                    0.0
                };
                weekdays_before(day) as f64 + within
            }
            // Months are counted from January and start again each year.
            TimeUnit::Month => {
                let count = i64::from(self.timeframe.count());
                let start = bucket_start_ms(self.timeframe, t);
                let next = self.grid.next_bucket_start_ms(self.timeframe, start);
                let month = month_index(start.div_euclid(DAY_MS));
                let per_year = (12 + count - 1) / count;
                let index = month.div_euclid(12) * per_year + month.rem_euclid(12) / count;
                index as f64 + (t - start) as f64 / (next - start).max(1) as f64
            }
            TimeUnit::Day => t as f64 / length as f64,
            TimeUnit::Week => (t + DAYS_BEFORE_FIRST_MONDAY * DAY_MS) as f64 / length as f64,
        }
    }

    /// The inverse of `position`.
    fn moment(self, position: f64) -> i64 {
        let length = self.timeframe.nominal_ms();
        match self.timeframe.unit() {
            TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour => {
                let per_day = self.bars_per_day() as f64;
                let days = (position / per_day).floor();
                let within = ((position - days * per_day) * length as f64).round() as i64;
                let days = days as i64;
                if self.stock() {
                    session::open_ms(nth_weekday(days)) + within
                } else {
                    days * DAY_MS + within
                }
            }
            TimeUnit::Day if self.stock() && self.timeframe.count() == 1 => {
                let index = position.floor();
                nth_weekday(index as i64) * DAY_MS
                    + ((position - index) * DAY_MS as f64).round() as i64
            }
            TimeUnit::Month => {
                let count = i64::from(self.timeframe.count());
                let per_year = (12 + count - 1) / count;
                let index = position.floor() as i64;
                let month = index.div_euclid(per_year) * 12 + index.rem_euclid(per_year) * count;
                let start = month_start_ms(month);
                let next = self.grid.next_bucket_start_ms(self.timeframe, start);
                start + ((position - index as f64) * (next - start) as f64).round() as i64
            }
            TimeUnit::Day => (position * length as f64).round() as i64,
            TimeUnit::Week => {
                (position * length as f64).round() as i64 - DAYS_BEFORE_FIRST_MONDAY * DAY_MS
            }
        }
    }
}

/// Weekdays from a fixed Monday up to `day` (days since the epoch), `day` not counted.
fn weekdays_before(day: i64) -> i64 {
    let since_monday = day + DAYS_BEFORE_FIRST_MONDAY;
    since_monday.div_euclid(7) * 5 + since_monday.rem_euclid(7).min(5)
}

/// The weekday that has `index` weekdays before it.
fn nth_weekday(index: i64) -> i64 {
    index.div_euclid(5) * 7 + index.rem_euclid(5) - DAYS_BEFORE_FIRST_MONDAY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::days_from_civil;

    const HOUR: i64 = 3_600_000;

    fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
        days_from_civil(year, month, day) * DAY_MS + hour * HOUR + minute * 60_000
    }

    fn calendar(grid: Grid, timeframe: Timeframe) -> Calendar {
        Calendar { grid, timeframe }
    }

    #[test]
    fn a_market_that_never_closes_steps_by_its_length() {
        let hourly = calendar(Grid::Utc, Timeframe::HOUR_1);
        let from = utc(2026, 11, 27, 23, 0);
        assert_eq!(hourly.after(from, 3.0), from + 3 * HOUR);
        assert_eq!(hourly.after(from, -30.0), from - 30 * HOUR);
        assert_eq!(hourly.steps(from, from + 3 * HOUR), 3.0);
        assert_eq!(hourly.steps(from, from + HOUR / 2), 0.5);
    }

    #[test]
    fn a_stocks_next_bar_after_the_close_is_the_next_session() {
        let half_hours = calendar(Grid::UsEquities, Timeframe::MINUTE_30);
        // Friday 2026-11-27, the 15:30 New York bar (20:30 UTC); Monday opens 14:30 UTC.
        let last = utc(2026, 11, 27, 20, 30);
        let monday = utc(2026, 11, 30, 14, 30);
        assert_eq!(half_hours.after(last, 1.0), monday);
        assert_eq!(half_hours.after(monday, -1.0), last);
        assert_eq!(half_hours.steps(last, monday), 1.0);
        // Thirteen half hours a session.
        assert_eq!(half_hours.after(monday, 13.0), utc(2026, 12, 1, 14, 30));
        // A moment in the night is where the next session starts.
        assert_eq!(half_hours.steps(last, utc(2026, 11, 29, 3, 0)), 1.0);
    }

    #[test]
    fn a_stocks_hours_follow_daylight_saving_time() {
        let hourly = calendar(Grid::UsEquities, Timeframe::HOUR_1);
        // The week of 2026-10-26 opens at 13:30 UTC, the week after (New York back on standard
        // time since 11-01) at 14:30.
        let friday_last = utc(2026, 10, 30, 19, 30);
        assert_eq!(hourly.after(friday_last, 1.0), utc(2026, 11, 2, 14, 30));
    }

    #[test]
    fn a_stocks_days_skip_the_weekend_and_keep_their_hour() {
        let daily = calendar(Grid::UsEquities, Timeframe::DAY_1);
        let friday = utc(2026, 11, 27, 5, 0);
        assert_eq!(daily.after(friday, 1.0), utc(2026, 11, 30, 5, 0));
        assert_eq!(daily.after(friday, 6.0), utc(2026, 12, 7, 5, 0));
        assert_eq!(daily.steps(friday, utc(2026, 12, 7, 5, 0)), 6.0);
    }

    #[test]
    fn weeks_and_months_follow_the_calendar() {
        let weekly = calendar(Grid::Utc, Timeframe::WEEK_1);
        let monday = utc(2026, 11, 23, 0, 0);
        assert_eq!(weekly.after(monday, 2.0), utc(2026, 12, 7, 0, 0));
        let monthly = calendar(Grid::Utc, Timeframe::MONTH_1);
        let november = utc(2026, 11, 1, 0, 0);
        assert_eq!(monthly.after(november, 3.0), utc(2027, 2, 1, 0, 0));
        assert_eq!(monthly.steps(november, utc(2027, 2, 1, 0, 0)), 3.0);
        let quarterly = calendar(Grid::Utc, Timeframe::MONTH_3);
        let october = utc(2026, 10, 1, 0, 0);
        assert_eq!(quarterly.after(october, 1.0), utc(2027, 1, 1, 0, 0));
        assert_eq!(quarterly.after(october, -3.0), utc(2026, 1, 1, 0, 0));
    }

    #[test]
    fn every_step_reads_back() {
        for (grid, timeframe) in [
            (Grid::Utc, Timeframe::HOUR_4),
            (Grid::UsEquities, Timeframe::MINUTE_5),
            (Grid::UsEquities, Timeframe::DAY_1),
            (Grid::UsEquities, Timeframe::WEEK_1),
            (Grid::Utc, Timeframe::MONTH_1),
        ] {
            let calendar = calendar(grid, timeframe);
            let from = grid.bucket_start_ms(timeframe, utc(2026, 11, 24, 15, 0));
            for steps in -400..400 {
                let at = calendar.after(from, f64::from(steps));
                let back = calendar.steps(from, at);
                assert!(
                    (back - f64::from(steps)).abs() < 1e-6,
                    "{grid:?} {timeframe:?} {steps}: {back}"
                );
            }
        }
    }
}
