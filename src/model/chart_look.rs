//! What each bar of a standard chart type looks like, decided from the data and the style.
//!
//! The renderer only paints what this returns, so the rules that give a bar its fill and colour
//! (hollow candles' four states, colouring by the previous close, the width a volume candle gets,
//! where a step turns, where a baseline line crosses) are testable without a window.

use super::Kline;
use super::chart_type::{BarSettings, CandleSettings, Rgba};

/// The chart's up and down colours (`ChartAppearance::bullish`/`bearish`, opaque).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub up: Rgba,
    pub down: Rgba,
}

impl Palette {
    pub fn new(bullish: [u8; 3], bearish: [u8; 3]) -> Self {
        Self {
            up: [bullish[0], bullish[1], bullish[2], 255],
            down: [bearish[0], bearish[1], bearish[2], 255],
        }
    }

    fn pick(self, up: bool) -> Rgba {
        if up { self.up } else { self.down }
    }
}

/// How a candle's body is painted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fill {
    Solid(Rgba),
    /// Outline only: the chart shows through.
    Hollow,
    /// No body at all.
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandleLook {
    pub body: Fill,
    pub border: Option<Rgba>,
    pub wick: Option<Rgba>,
}

/// Whether the bar counts as rising: the close against the previous close when that is the chosen
/// reference and there is one, else against its own open.
pub fn is_rising(bar: &Kline, previous_close: Option<f64>, by_previous_close: bool) -> bool {
    match previous_close {
        Some(previous) if by_previous_close => bar.close >= previous,
        _ => bar.close >= bar.open,
    }
}

/// A candle of the plain, volume, Heikin Ashi, Renko, Line Break and Range-as-candles types.
pub fn candle_look(
    settings: &CandleSettings,
    palette: Palette,
    bar: &Kline,
    previous_close: Option<f64>,
    recolor: Option<Rgba>,
) -> CandleLook {
    let up = is_rising(bar, previous_close, settings.color_by_previous_close);
    let body = settings
        .up_body
        .filter(|_| up)
        .or(settings.down_body.filter(|_| !up))
        .unwrap_or_else(|| palette.pick(up));
    // A script's `barcolor()` recolours the body and the wick, whatever the user chose for them.
    // The border keeps the chart's own colour: TradingView draws a recoloured candle with its
    // usual outline (seen on a script that dims bodies toward the background), so the candle
    // stays visible. This is inferred from that, not documented.
    let border = if up {
        settings.up_border
    } else {
        settings.down_border
    }
    .unwrap_or(body);
    let body = recolor.unwrap_or(body);
    let wick = recolor
        .or(if up {
            settings.up_wick
        } else {
            settings.down_wick
        })
        .unwrap_or(body);
    CandleLook {
        body: if settings.body {
            Fill::Solid(body)
        } else {
            Fill::None
        },
        border: settings.border.then_some(border),
        wick: settings.wick.then_some(wick),
    }
}

/// A hollow candle has four states. The colour follows the close against the previous close
/// (the open for the first bar); the body is hollow when the close is at or above the open and
/// filled when it is below.
///
/// | close vs open | close vs previous close | looks like   |
/// |---------------|-------------------------|--------------|
/// | above         | above                   | green hollow |
/// | below         | above                   | green filled |
/// | above         | below                   | red hollow   |
/// | below         | below                   | red filled   |
///
/// A close equal to the open counts as above it, and equal to the previous close as above that,
/// the same way `Kline::is_up` reads a doji.
pub fn hollow_look(
    settings: &CandleSettings,
    palette: Palette,
    bar: &Kline,
    previous_close: Option<f64>,
    recolor: Option<Rgba>,
) -> CandleLook {
    let green = is_rising(bar, previous_close, true);
    let hollow = bar.close >= bar.open;
    let color = settings
        .up_body
        .filter(|_| green)
        .or(settings.down_body.filter(|_| !green))
        .unwrap_or_else(|| palette.pick(green));
    let color = recolor.unwrap_or(color);
    let border = recolor
        .or(if green {
            settings.up_border
        } else {
            settings.down_border
        })
        .unwrap_or(color);
    let wick = recolor
        .or(if green {
            settings.up_wick
        } else {
            settings.down_wick
        })
        .unwrap_or(color);
    CandleLook {
        body: if !settings.body {
            Fill::None
        } else if hollow {
            Fill::Hollow
        } else {
            Fill::Solid(color)
        },
        // The outline is what carries a hollow body, so it stays even when borders are switched
        // off in favour of the body colour.
        border: (settings.border || hollow).then_some(border),
        wick: settings.wick.then_some(wick),
    }
}

/// The colour of an OHLC or high-low bar.
pub fn bar_color(
    settings: &BarSettings,
    palette: Palette,
    bar: &Kline,
    previous_close: Option<f64>,
    recolor: Option<Rgba>,
) -> Rgba {
    let up = is_rising(bar, previous_close, settings.color_by_previous_close);
    recolor
        .or(if up { settings.up } else { settings.down })
        .unwrap_or_else(|| palette.pick(up))
}

/// The width of a volume candle's body as a fraction of its slot's candle width: the volume
/// against the largest in view, never thinner than `MIN_VOLUME_WIDTH` so a quiet bar can still be
/// seen. Without volume (a missing or all-zero window) every candle is full width, so nothing is
/// invented from data the source does not have.
///
/// TradingView does not document its scaling, so the linear scale against the visible maximum is
/// this app's own choice.
pub fn volume_width_fraction(volume: f64, window_max: f64) -> f64 {
    if !(window_max.is_finite() && window_max > 0.0) || !volume.is_finite() {
        return 1.0;
    }
    (volume / window_max).clamp(MIN_VOLUME_WIDTH, 1.0)
}

pub const MIN_VOLUME_WIDTH: f64 = 0.12;

/// The high and low a candle is drawn to: the wicks cannot stop short of the body, so values a
/// script gives that contradict each other are widened to include the open and close.
pub fn drawn_extremes(open: f64, high: f64, low: f64, close: f64) -> (f64, f64) {
    let top = high.max(low).max(open).max(close);
    let bottom = high.min(low).min(open).min(close);
    (top, bottom)
}

/// A point of a polyline in plot coordinates.
pub type Point = (f32, f32);

/// The corners of a step line through `points`: it holds each value until the next bar, then
/// jumps (horizontal first, vertical at the next bar).
pub fn step_points(points: &[Point]) -> Vec<Point> {
    let mut stepped = Vec::with_capacity(points.len() * 2);
    for (index, point) in points.iter().enumerate() {
        if let Some(previous) = index.checked_sub(1).map(|index| points[index]) {
            stepped.push((point.0, previous.1));
        }
        stepped.push(*point);
    }
    stepped
}

/// A run of a polyline on one side of the baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct SideRun {
    /// True when the run is above the baseline (smaller `y`: screen coordinates grow downward).
    pub above: bool,
    pub points: Vec<Point>,
}

/// Cuts `points` where they cross the horizontal line `level_y`, so each run lies on one side and
/// the neighbouring runs meet at the crossing.
pub fn split_at_level(points: &[Point], level_y: f32) -> Vec<SideRun> {
    let mut runs: Vec<SideRun> = Vec::new();
    for (index, &point) in points.iter().enumerate() {
        let above = point.1 <= level_y;
        if let Some(previous) = index.checked_sub(1).map(|index| points[index]) {
            let was_above = previous.1 <= level_y;
            if was_above != above {
                let t = (level_y - previous.1) / (point.1 - previous.1);
                let crossing = (previous.0 + (point.0 - previous.0) * t, level_y);
                if let Some(run) = runs.last_mut() {
                    run.points.push(crossing);
                }
                runs.push(SideRun {
                    above,
                    points: vec![crossing],
                });
            }
        }
        match runs.last_mut() {
            Some(run) => run.points.push(point),
            None => runs.push(SideRun {
                above,
                points: vec![point],
            }),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    const PALETTE: Palette = Palette {
        up: [0, 255, 0, 255],
        down: [255, 0, 0, 255],
    };
    const GREEN: Rgba = [0, 255, 0, 255];
    const RED: Rgba = [255, 0, 0, 255];

    fn bar(open: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 1,
            open,
            high: open.max(close) + 1.0,
            low: open.min(close) - 1.0,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn a_candle_is_coloured_by_its_open_unless_the_previous_close_is_chosen() {
        let settings = CandleSettings::default();
        let falling_but_above_previous = bar(12.0, 11.0);
        let look = candle_look(
            &settings,
            PALETTE,
            &falling_but_above_previous,
            Some(10.0),
            None,
        );
        assert_eq!(look.body, Fill::Solid(RED));
        let by_previous = CandleSettings {
            color_by_previous_close: true,
            ..settings
        };
        let look = candle_look(
            &by_previous,
            PALETTE,
            &falling_but_above_previous,
            Some(10.0),
            None,
        );
        assert_eq!(look.body, Fill::Solid(GREEN));
        // The first bar has no previous close: it falls back to its own open.
        let look = candle_look(
            &by_previous,
            PALETTE,
            &falling_but_above_previous,
            None,
            None,
        );
        assert_eq!(look.body, Fill::Solid(RED));
    }

    #[test]
    fn borders_and_wicks_follow_the_body_until_given_their_own_colour() {
        let settings = CandleSettings {
            up_border: Some([1, 2, 3, 255]),
            wick: false,
            ..CandleSettings::default()
        };
        let look = candle_look(&settings, PALETTE, &bar(10.0, 11.0), None, None);
        assert_eq!(look.border, Some([1, 2, 3, 255]));
        assert_eq!(look.wick, None);
        let look = candle_look(&settings, PALETTE, &bar(11.0, 10.0), None, None);
        assert_eq!(look.border, Some(RED));
    }

    #[test]
    fn switching_the_body_off_leaves_the_outline() {
        let settings = CandleSettings {
            body: false,
            ..CandleSettings::default()
        };
        let look = candle_look(&settings, PALETTE, &bar(10.0, 11.0), None, None);
        assert_eq!(look.body, Fill::None);
        assert_eq!(look.border, Some(GREEN));
    }

    #[test]
    fn a_script_recolour_wins_over_body_and_wick_but_not_the_border() {
        let settings = CandleSettings {
            up_border: Some([1, 2, 3, 255]),
            ..CandleSettings::default()
        };
        let purple = [128, 0, 128, 200];
        let look = candle_look(&settings, PALETTE, &bar(10.0, 11.0), None, Some(purple));
        assert_eq!(look.body, Fill::Solid(purple));
        assert_eq!(look.border, Some([1, 2, 3, 255]));
        assert_eq!(look.wick, Some(purple));
        // With no border colour of its own the outline is the chart's colour for the bar.
        let look = candle_look(&settings, PALETTE, &bar(11.0, 10.0), None, Some(purple));
        assert_eq!(look.border, Some(RED));
        // A hollow candle's outline is its body, so it follows the script.
        let look = hollow_look(
            &settings,
            PALETTE,
            &bar(10.0, 11.0),
            Some(9.0),
            Some(purple),
        );
        assert_eq!(look.body, Fill::Hollow);
        assert_eq!(look.border, Some(purple));
    }

    #[test]
    fn hollow_candles_have_four_states() {
        let settings = CandleSettings::default();
        // Close above open, above the previous close: green hollow.
        let look = hollow_look(&settings, PALETTE, &bar(10.0, 12.0), Some(9.0), None);
        assert_eq!((look.body, look.border), (Fill::Hollow, Some(GREEN)));
        // Close below open but above the previous close: green filled.
        let look = hollow_look(&settings, PALETTE, &bar(12.0, 11.0), Some(10.0), None);
        assert_eq!((look.body, look.border), (Fill::Solid(GREEN), Some(GREEN)));
        // Close above open but below the previous close: red hollow.
        let look = hollow_look(&settings, PALETTE, &bar(9.0, 10.0), Some(11.0), None);
        assert_eq!((look.body, look.border), (Fill::Hollow, Some(RED)));
        // Close below open and below the previous close: red filled.
        let look = hollow_look(&settings, PALETTE, &bar(12.0, 10.0), Some(11.0), None);
        assert_eq!((look.body, look.border), (Fill::Solid(RED), Some(RED)));
    }

    #[test]
    fn a_hollow_doji_is_hollow_and_a_hollow_body_keeps_its_outline() {
        let settings = CandleSettings {
            border: false,
            ..CandleSettings::default()
        };
        let look = hollow_look(&settings, PALETTE, &bar(10.0, 10.0), Some(10.0), None);
        assert_eq!(look.body, Fill::Hollow);
        assert_eq!(look.border, Some(GREEN));
        // Filled with borders off: no separate outline.
        let look = hollow_look(&settings, PALETTE, &bar(12.0, 10.0), Some(11.0), None);
        assert_eq!(look.border, None);
    }

    #[test]
    fn bars_take_up_down_colours_and_recolours() {
        let settings = BarSettings {
            down: Some([9, 9, 9, 255]),
            ..BarSettings::default()
        };
        assert_eq!(
            bar_color(&settings, PALETTE, &bar(10.0, 11.0), None, None),
            GREEN
        );
        assert_eq!(
            bar_color(&settings, PALETTE, &bar(11.0, 10.0), None, None),
            [9, 9, 9, 255]
        );
        assert_eq!(
            bar_color(&settings, PALETTE, &bar(11.0, 10.0), None, Some(RED)),
            RED
        );
    }

    #[test]
    fn volume_widths_scale_against_the_window_and_never_vanish() {
        assert_eq!(volume_width_fraction(50.0, 100.0), 0.5);
        assert_eq!(volume_width_fraction(100.0, 100.0), 1.0);
        assert_eq!(volume_width_fraction(0.0, 100.0), MIN_VOLUME_WIDTH);
        assert_eq!(volume_width_fraction(500.0, 100.0), 1.0);
        // No volume anywhere, or not a number: full width rather than invented thinness.
        assert_eq!(volume_width_fraction(0.0, 0.0), 1.0);
        assert_eq!(volume_width_fraction(f64::NAN, 100.0), 1.0);
        assert_eq!(volume_width_fraction(5.0, f64::NAN), 1.0);
    }

    #[test]
    fn contradictory_extremes_are_widened_to_the_body() {
        assert_eq!(drawn_extremes(10.0, 9.0, 11.0, 12.0), (12.0, 9.0));
        assert_eq!(drawn_extremes(10.0, 12.0, 8.0, 11.0), (12.0, 8.0));
    }

    #[test]
    fn a_step_holds_the_value_until_the_next_bar() {
        let stepped = step_points(&[(0.0, 5.0), (10.0, 3.0), (20.0, 4.0)]);
        assert_eq!(
            stepped,
            vec![
                (0.0, 5.0),
                (10.0, 5.0),
                (10.0, 3.0),
                (20.0, 3.0),
                (20.0, 4.0)
            ]
        );
        assert!(step_points(&[]).is_empty());
        assert_eq!(step_points(&[(1.0, 1.0)]), vec![(1.0, 1.0)]);
    }

    #[test]
    fn a_line_is_cut_where_it_crosses_the_baseline() {
        let runs = split_at_level(&[(0.0, 2.0), (10.0, 12.0), (20.0, 14.0), (30.0, 4.0)], 7.0);
        assert_eq!(runs.len(), 3);
        assert!(runs[0].above && !runs[1].above && runs[2].above);
        assert_eq!(runs[0].points, vec![(0.0, 2.0), (5.0, 7.0)]);
        assert_eq!(runs[1].points.first(), Some(&(5.0, 7.0)));
        assert_eq!(runs[1].points.last(), Some(&(27.0, 7.0)));
        assert_eq!(runs[2].points, vec![(27.0, 7.0), (30.0, 4.0)]);
        // Entirely on one side: a single run, untouched.
        let runs = split_at_level(&[(0.0, 1.0), (1.0, 2.0)], 7.0);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].points.len(), 2);
    }
}
