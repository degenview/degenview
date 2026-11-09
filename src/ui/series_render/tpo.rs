//! Paints the Time price opportunity chart type: for each profile (a day, a week, a session), the
//! letters of the blocks that visited every price row, with the point of control, value area, initial
//! balance, poor highs and lows, single prints and the volume profile beside it.
//!
//! The profiles are built elsewhere (`model::market_profile::tpo`) from 5-minute candles and handed in
//! as a finished [`TpoSet`]; this turns them into shapes. A profile is drawn from the x of its start
//! across the width its period has on screen, and how it is drawn depends on the room a letter has
//! ([`Detail`]): letters where they fit, coloured blocks where they do not, a plain bar per row
//! where even blocks would be a smear. A bar of the chart that no profile covers (the history is
//! older than the candles reach) stays a plain candle, and the status line says what the data is.
//!
//! [`Geo`] is the coordinate system both the painter and the pointer (hover, the right-click menu)
//! use, so a letter is where it is drawn.

use super::{SeriesFrame, color};
use crate::model::Kline;
use crate::model::chart_type::{TpoLevel, TpoSettings, VolumePlacement};
use crate::model::market_profile::footprint::SetStatus;
use crate::model::market_profile::sessions::letter;
use crate::model::market_profile::tpo::{TpoProfile, TpoSet};
use crate::ui::chart_card::PriceScale;
use crate::ui::theme;
use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Stroke};

/// How much of a profile is drawn, by the room a letter has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Detail {
    Letters,
    Blocks,
    /// One bar per row, as long as its count.
    Bars,
}

const LETTER_ROW_HEIGHT: f32 = 9.0;
const LETTER_WIDTH: f32 = 6.0;
const BLOCK_ROW_HEIGHT: f32 = 2.0;
const BLOCK_WIDTH: f32 = 2.0;

/// The detail for rows `row_height` tall and columns `column_width` wide (points).
pub fn detail(row_height: f32, column_width: f32) -> Detail {
    if row_height >= LETTER_ROW_HEIGHT && column_width >= LETTER_WIDTH {
        Detail::Letters
    } else if row_height >= BLOCK_ROW_HEIGHT && column_width >= BLOCK_WIDTH {
        Detail::Blocks
    } else {
        Detail::Bars
    }
}

/// What the painter and the pointer share: the bars in view, where they are and the price scale.
#[derive(Clone, Copy)]
pub struct Geo<'a> {
    pub plot: Rect,
    pub bars: &'a [Kline],
    pub x_for: &'a dyn Fn(usize) -> f32,
    pub slot_width: f32,
    pub scale: &'a PriceScale,
}

impl<'a> Geo<'a> {
    pub fn of(frame: &'a SeriesFrame<'a>) -> Self {
        Self {
            plot: frame.plot,
            bars: frame.bars,
            x_for: frame.x_for,
            slot_width: frame.slot_width,
            scale: frame.scale,
        }
    }

    fn bar_ms(&self) -> f32 {
        match self.bars {
            [first, second, ..] => (second.open_time - first.open_time).max(1) as f32,
            _ => 3_600_000.0,
        }
    }

    /// The x of an instant: a bar's left edge at its open, in between across it, and past the ends by
    /// the same scale (a profile of today reaches beyond the last bar).
    pub fn x_at(&self, time_ms: i64) -> f32 {
        let count = self.bars.partition_point(|bar| bar.open_time <= time_ms);
        let half = self.slot_width / 2.0;
        if count == 0 {
            let first = self.bars[0].open_time;
            return (self.x_for)(0)
                - half
                - (first - time_ms) as f32 / self.bar_ms() * self.slot_width;
        }
        let index = count - 1;
        let open = self.bars[index].open_time;
        let next = self
            .bars
            .get(index + 1)
            .map_or(open + self.bar_ms() as i64, |bar| bar.open_time);
        let fraction = (time_ms - open) as f32 / (next - open).max(1) as f32;
        (self.x_for)(index) - half + fraction * self.slot_width
    }

    pub fn y(&self, price: f64) -> f32 {
        self.scale.y(price)
    }
}

/// Where the parts of one profile sit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The profile's span on screen.
    pub x0: f32,
    pub x1: f32,
    /// Where the letters start, the width of one column, and how many columns there are.
    pub letters_x0: f32,
    pub column: f32,
    pub columns: u32,
    /// The volume profile: where its bars start, how long the longest is, and which way they grow.
    pub volume: Option<VolumeBars>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeBars {
    pub base: f32,
    pub length: f32,
    pub leftwards: bool,
}

impl Layout {
    /// Where the letters end.
    pub fn letters_x1(&self) -> f32 {
        self.letters_x0 + self.column * self.columns as f32
    }
}

/// The ideal width of a letter column for rows `row_height` tall.
fn ideal_column(row_height: f32) -> f32 {
    ((row_height * 0.82).clamp(7.0, 13.0) * 0.62 + 1.0).clamp(6.0, 14.0)
}

pub fn layout(
    geo: &Geo,
    profile: &TpoProfile,
    settings: &TpoSettings,
    row_height: f32,
    grid_rows: u32,
) -> Layout {
    let _ = grid_rows;
    let x0 = geo.x_at(profile.start_ms);
    let x1 = geo.x_at(profile.end_ms);
    let span = (x1 - x0).max(0.0);
    let columns = if settings.expand_blocks {
        profile.blocks.max(1)
    } else {
        profile.width().max(1)
    };
    let volume_on = settings.volume.show && profile.volume_poc.is_some();
    // The volume profile takes a quarter of the span, at most; the letters get the rest.
    let volume_span = if volume_on {
        (span * 0.25).clamp(0.0, 90.0)
    } else {
        0.0
    };
    let (letters_x0, available) = match (volume_on, settings.volume.placement) {
        (true, VolumePlacement::Left) => (x0 + volume_span + 2.0, span - volume_span - 2.0),
        (true, VolumePlacement::Right) => (x0, span - volume_span - 2.0),
        _ => (x0, span),
    };
    let ideal = ideal_column(row_height);
    let column = if ideal * columns as f32 <= available {
        ideal
    } else {
        (available / columns as f32).max(0.0)
    };
    let letters_end = letters_x0 + column * columns as f32;
    let volume = volume_on.then(|| match settings.volume.placement {
        VolumePlacement::Left => VolumeBars {
            base: x0 + volume_span,
            length: volume_span,
            leftwards: true,
        },
        VolumePlacement::Right => VolumeBars {
            base: letters_end + 3.0,
            length: (x1 - letters_end - 3.0).clamp(0.0, volume_span),
            leftwards: false,
        },
    });
    Layout {
        x0,
        x1,
        letters_x0,
        column,
        columns,
        volume,
    }
}

// MARK: Colours

const DEFAULT_GRADIENT: [[u8; 4]; 4] = [
    [38, 166, 154, 255],
    [255, 213, 79, 255],
    [255, 152, 0, 255],
    [239, 83, 80, 255],
];

pub fn default_gradient() -> [[u8; 4]; 4] {
    DEFAULT_GRADIENT
}

fn lerp(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    [
        mix(a[0], b[0]),
        mix(a[1], b[1]),
        mix(a[2], b[2]),
        mix(a[3], b[3]),
    ]
}

/// The colour of a block's letter: the first two gradient colours span A to Z, the last two a to z.
pub fn block_color(settings: &TpoSettings, block: u32) -> [u8; 4] {
    let pick = |n: usize| settings.gradient[n].unwrap_or(DEFAULT_GRADIENT[n]);
    let within = (block % 26) as f32 / 25.0;
    if block % 52 < 26 {
        lerp(pick(0), pick(1), within)
    } else {
        lerp(pick(2), pick(3), within)
    }
}

fn tint(base: [u8; 4], alpha: f32) -> Color32 {
    let [r, g, b, a] = base;
    Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        (f32::from(a) * alpha).round().clamp(0.0, 255.0) as u8,
    )
}

fn level_color(level: &TpoLevel, fallback: [u8; 4]) -> Color32 {
    color(level.color.unwrap_or(fallback))
}

// MARK: Painting

pub fn paint(frame: &SeriesFrame) {
    let settings = frame.style.tpo;
    let feed = frame.profiles;
    let set = feed.and_then(|feed| feed.tpo.as_deref());
    let geo = Geo::of(frame);
    let painter = frame.painter;
    let mut covered = vec![false; frame.bars.len()];
    if let Some(set) = set {
        for profile in &set.profiles {
            // Bars inside a profile are drawn by it.
            for (index, bar) in frame.bars.iter().enumerate() {
                if profile.start_ms <= bar.open_time && bar.open_time < profile.end_ms {
                    covered[index] = true;
                }
            }
        }
    }
    for (index, is_covered) in covered.iter().enumerate() {
        if !is_covered {
            super::footprint::plain_candle(frame, index);
        }
    }
    if let Some(set) = set
        && let Some(grid) = set.grid
    {
        for profile in &set.profiles {
            let first = grid.bottom(profile.first_row());
            let row_height = (geo.y(first) - geo.y(grid.top(profile.first_row()))).abs();
            let lay = layout(&geo, profile, &settings, row_height, 0);
            if lay.x1 < frame.plot.left() - 1.0 || lay.x0 > frame.plot.right() + 1.0 {
                continue;
            }
            paint_profile(frame, &geo, &settings, set, profile, &lay, row_height);
        }
    }
    let _ = painter;
    paint_status(frame, set);
}

fn paint_profile(
    frame: &SeriesFrame,
    geo: &Geo,
    settings: &TpoSettings,
    set: &TpoSet,
    profile: &TpoProfile,
    lay: &Layout,
    row_height: f32,
) {
    let Some(grid) = set.grid else { return };
    let painter = frame.painter;
    let plot = frame.plot;
    let level = detail(row_height, lay.column);
    let (va_low, va_high) = profile.value_area;
    let outside = settings.opacity_outside_value_area as f32 / 100.0;
    let font = FontId::monospace((row_height * 0.82).clamp(7.0, 13.0));
    let ink = theme::text_primary();
    // The volume profile first, so letters sit over nothing of it.
    if let Some(volume) = lay.volume {
        paint_volume(frame, geo, settings, set, profile, &volume, row_height);
    }
    for (index, row) in profile.rows.iter().enumerate() {
        let (top, bottom) = (geo.y(grid.top(row.row)), geo.y(grid.bottom(row.row)));
        if bottom < plot.top() || top > plot.bottom() || row.blocks.is_empty() {
            continue;
        }
        let alpha = if (va_low..=va_high).contains(&index) {
            1.0
        } else {
            outside
        };
        match level {
            Detail::Bars => {
                let width = lay.column.max(0.0) * row.count() as f32;
                let rect = Rect::from_min_max(
                    Pos2::new(lay.letters_x0, top),
                    Pos2::new(lay.letters_x0 + width.max(1.0), bottom),
                );
                let base = block_color(settings, row.blocks[0]);
                painter.rect_filled(rect, 0.0, tint(base, 0.8 * alpha));
            }
            Detail::Blocks | Detail::Letters => {
                for (position, &block) in row.blocks.iter().enumerate() {
                    let column = if settings.expand_blocks {
                        block as usize
                    } else {
                        position
                    };
                    let x = lay.letters_x0 + lay.column * column as f32;
                    let rect =
                        Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + lay.column, bottom));
                    let base = block_color(settings, block);
                    let want_letters = settings.show_letters && level == Detail::Letters;
                    if settings.show_blocks || !want_letters {
                        painter.rect_filled(
                            rect.shrink2(eframe::egui::vec2(0.25, 0.25)),
                            0.0,
                            tint(base, 0.55 * alpha),
                        );
                    }
                    if want_letters {
                        let text_color = if settings.show_blocks {
                            ink
                        } else {
                            tint(base, alpha)
                        };
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            letter(block).to_string(),
                            font.clone(),
                            if settings.show_blocks {
                                text_color.gamma_multiply(alpha.max(0.5))
                            } else {
                                text_color
                            },
                        );
                    }
                }
            }
        }
    }
    paint_levels(frame, geo, settings, set, profile, lay);
}

fn paint_volume(
    frame: &SeriesFrame,
    geo: &Geo,
    settings: &TpoSettings,
    set: &TpoSet,
    profile: &TpoProfile,
    bars: &VolumeBars,
    row_height: f32,
) {
    let (Some(grid), Some(poc), Some(area)) =
        (set.grid, profile.volume_poc, profile.volume_value_area)
    else {
        return;
    };
    let peak = profile.peak_volume();
    if peak <= 0.0 || bars.length < 1.0 {
        return;
    }
    let base = settings.volume.color.unwrap_or([140, 150, 160, 255]);
    let area_color = settings
        .volume
        .value_area_color
        .unwrap_or([200, 170, 90, 255]);
    let painter = frame.painter;
    let font = FontId::monospace((row_height * 0.7).clamp(7.0, 10.0));
    for (index, row) in profile.rows.iter().enumerate() {
        if row.volume <= 0.0 {
            continue;
        }
        let (top, bottom) = (geo.y(grid.top(row.row)), geo.y(grid.bottom(row.row)));
        let length = bars.length * (row.volume / peak) as f32;
        let (from, to) = if bars.leftwards {
            (bars.base - length, bars.base)
        } else {
            (bars.base, bars.base + length)
        };
        let inside = (area.0..=area.1).contains(&index);
        let rect = Rect::from_min_max(Pos2::new(from, top + 0.5), Pos2::new(to, bottom - 0.5));
        painter.rect_filled(
            rect,
            0.0,
            tint(
                if inside { area_color } else { base },
                if index == poc { 0.95 } else { 0.6 },
            ),
        );
        if settings.volume.values && row_height >= 9.0 {
            painter.text(
                Pos2::new(
                    if bars.leftwards { from - 2.0 } else { to + 2.0 },
                    (top + bottom) / 2.0,
                ),
                if bars.leftwards {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                super::footprint::format_volume(row.volume),
                font.clone(),
                theme::text_secondary(),
            );
        }
    }
    let line = |row: usize, edge_top: bool, label: &str| {
        let y = if edge_top {
            geo.y(grid.top(profile.rows[row].row))
        } else {
            geo.y(grid.bottom(profile.rows[row].row))
        };
        let (a, b) = if bars.leftwards {
            (bars.base - bars.length, bars.base)
        } else {
            (bars.base, bars.base + bars.length)
        };
        painter.hline(a..=b, y, Stroke::new(1.0, tint(area_color, 0.8)));
        let _ = label;
    };
    if settings.volume.show_poc {
        let y = geo.y(grid.middle(profile.rows[poc].row));
        let (a, b) = if bars.leftwards {
            (bars.base - bars.length, bars.base)
        } else {
            (bars.base, bars.base + bars.length)
        };
        painter.hline(
            a..=b,
            y,
            Stroke::new(1.5, theme::text_primary().gamma_multiply(0.8)),
        );
    }
    if settings.volume.show_vah {
        line(area.1, true, "VAH");
    }
    if settings.volume.show_val {
        line(area.0, false, "VAL");
    }
}

/// The x a level is carried to: the first bar after the profile whose range reaches `price`, else the
/// edge of the plot. Only for a finished profile (a forming one has nothing after it).
pub(super) fn extended_to_bar(geo: &Geo, end_ms: i64, price: f64) -> f32 {
    geo.bars
        .iter()
        .filter(|bar| bar.open_time >= end_ms)
        .find(|bar| bar.low <= price && price <= bar.high)
        .map_or(geo.plot.right(), |bar| geo.x_at(bar.open_time))
}

fn extended_to(geo: &Geo, profile: &TpoProfile, price: f64) -> f32 {
    geo.bars
        .iter()
        .filter(|bar| bar.open_time >= profile.end_ms)
        .find(|bar| bar.low <= price && price <= bar.high)
        .map_or(geo.plot.right(), |bar| geo.x_at(bar.open_time))
}

fn paint_levels(
    frame: &SeriesFrame,
    geo: &Geo,
    settings: &TpoSettings,
    set: &TpoSet,
    profile: &TpoProfile,
    lay: &Layout,
) {
    let Some(grid) = set.grid else { return };
    let painter = frame.painter;
    let plot = frame.plot;
    let font = FontId::proportional(9.5);
    let right = lay.x1.max(lay.letters_x1());
    let level = |level: &TpoLevel,
                 price: f64,
                 fallback: [u8; 4],
                 label: &str,
                 dashed: bool,
                 thick: f32,
                 extend: bool| {
        if !level.show {
            return;
        }
        let y = geo.y(price);
        if y < plot.top() || y > plot.bottom() {
            return;
        }
        let end = if extend && level.extend && !profile.forming {
            extended_to(geo, profile, price).max(right)
        } else {
            right
        };
        let stroke = Stroke::new(thick, level_color(level, fallback));
        if dashed {
            painter.extend(eframe::egui::Shape::dashed_line(
                &[Pos2::new(lay.x0, y), Pos2::new(end, y)],
                stroke,
                4.0,
                3.0,
            ));
        } else {
            painter.hline(lay.x0..=end, y, stroke);
        }
        if end - lay.x0 > 40.0 {
            painter.text(
                Pos2::new(end - 2.0, y - 1.0),
                Align2::RIGHT_BOTTOM,
                label,
                font.clone(),
                level_color(level, fallback).gamma_multiply(0.9),
            );
        }
    };
    let (va_low, va_high) = profile.value_area;
    level(
        &settings.poc,
        grid.middle(profile.poc_row()),
        [240, 185, 11, 255],
        "POC",
        false,
        1.5,
        true,
    );
    level(
        &settings.vah,
        grid.top(profile.rows[va_high].row),
        [160, 160, 160, 255],
        "VAH",
        true,
        1.0,
        false,
    );
    level(
        &settings.val,
        grid.bottom(profile.rows[va_low].row),
        [160, 160, 160, 255],
        "VAL",
        true,
        1.0,
        false,
    );
    level(
        &settings.midpoint,
        (profile.high + profile.low) / 2.0,
        [190, 190, 190, 255],
        "Mid",
        true,
        1.0,
        false,
    );
    level(
        &settings.open,
        profile.open,
        [120, 200, 120, 255],
        "Open",
        false,
        1.0,
        false,
    );
    level(
        &settings.close,
        profile.close,
        [220, 120, 120, 255],
        "Close",
        false,
        1.0,
        false,
    );
    if profile.poor_high {
        let top = profile.rows.len() - 1;
        level(
            &settings.poor_high,
            grid.top(profile.rows[top].row),
            [255, 112, 67, 255],
            "Poor high",
            false,
            1.5,
            true,
        );
    }
    if profile.poor_low {
        level(
            &settings.poor_low,
            grid.bottom(profile.rows[0].row),
            [255, 112, 67, 255],
            "Poor low",
            false,
            1.5,
            true,
        );
    }
    if settings.single_prints.show {
        for run in profile.single_print_runs() {
            let Some(first) = profile.first_established(run) else {
                continue;
            };
            let row = &profile.rows[first.index];
            let (top, bottom) = (geo.y(grid.top(row.row)), geo.y(grid.bottom(row.row)));
            let color = level_color(&settings.single_prints, [255, 235, 59, 255]);
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(lay.x0, top), Pos2::new(right, bottom)),
                0.0,
                Stroke::new(1.0, color),
                eframe::egui::StrokeKind::Inside,
            );
            if settings.single_prints.extend && !profile.forming {
                let end = extended_to(geo, profile, grid.middle(row.row));
                if end > right {
                    painter.hline(
                        right..=end,
                        (top + bottom) / 2.0,
                        Stroke::new(1.0, tint([255, 235, 59, 255], 0.7)),
                    );
                }
            }
        }
    }
    if settings.initial_balance.show
        && let Some((low, high)) = profile.initial_balance
    {
        let x = lay.x0 - 3.0;
        let color = level_color(&settings.initial_balance, [200, 200, 200, 255]);
        let (top, bottom) = (geo.y(high), geo.y(low));
        painter.vline(x, top..=bottom, Stroke::new(1.5, color));
        painter.hline(x..=x + 4.0, top, Stroke::new(1.0, color));
        painter.hline(x..=x + 4.0, bottom, Stroke::new(1.0, color));
    }
}

/// Where the data came from, or why there is none: bottom left, small.
fn paint_status(frame: &SeriesFrame, set: Option<&TpoSet>) {
    let Some(message) = status_text(frame, set) else {
        return;
    };
    frame.painter.text(
        Pos2::new(frame.plot.left() + 8.0, frame.plot.bottom() - 8.0),
        Align2::LEFT_BOTTOM,
        message,
        FontId::proportional(10.5),
        theme::text_secondary(),
    );
}

pub fn status_text(frame: &SeriesFrame, set: Option<&TpoSet>) -> Option<String> {
    if let Some(SetStatus::Unsupported(reason)) = set.map(|set| set.status) {
        return Some(format!(
            "Time price opportunity unavailable: {}",
            reason.explanation()
        ));
    }
    let Some(set) = set else {
        return Some(match frame.profiles {
            Some(feed) if feed.error.is_some() => {
                format!(
                    "Time price opportunity: {}",
                    feed.error.as_deref().unwrap_or("")
                )
            }
            _ => "Loading order-flow data…".to_owned(),
        });
    };
    if set.refused > 0 {
        let reason = set.first_refusal.map_or("", |reason| reason.explanation());
        return Some(format!("{} profiles not shown. {reason}", set.refused));
    }
    if set.profiles.is_empty() {
        return Some("No 5-minute candles for these bars yet".to_owned());
    }
    Some(format!(
        "{} profile{} from 5-minute candles, {}-minute blocks",
        set.profiles.len(),
        if set.profiles.len() == 1 { "" } else { "s" },
        set.block_ms / 60_000
    ))
}

// MARK: Pointer

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit<'a> {
    pub profile: &'a TpoProfile,
    pub row: i64,
    /// The block of the letter under the pointer, when it is over one.
    pub block: Option<u32>,
    pub in_volume: bool,
}

pub fn hit<'a>(
    geo: &Geo,
    set: &'a TpoSet,
    settings: &TpoSettings,
    pointer: Pos2,
) -> Option<Hit<'a>> {
    if !geo.plot.contains(pointer) || geo.bars.is_empty() {
        return None;
    }
    let grid = set.grid?;
    let row = grid.row_of(geo.scale.price(pointer.y))?;
    for profile in &set.profiles {
        let first = profile.first_row();
        let row_height = (geo.y(grid.bottom(first)) - geo.y(grid.top(first))).abs();
        let lay = layout(geo, profile, settings, row_height, 0);
        if pointer.x < lay.x0 || pointer.x > lay.x1 {
            continue;
        }
        let in_volume = lay.volume.is_some_and(|v| {
            let (a, b) = if v.leftwards {
                (v.base - v.length, v.base)
            } else {
                (v.base, v.base + v.length)
            };
            pointer.x >= a && pointer.x <= b
        });
        profile.row_at(row)?;
        let column = ((pointer.x - lay.letters_x0) / lay.column.max(0.001)).floor();
        let block = (!in_volume && column >= 0.0 && (column as u32) < lay.columns)
            .then(|| {
                let row_data = profile.row_at(row)?;
                if settings.expand_blocks {
                    row_data
                        .blocks
                        .contains(&(column as u32))
                        .then_some(column as u32)
                } else {
                    row_data.blocks.get(column as usize).copied()
                }
            })
            .flatten();
        return Some(Hit {
            profile,
            row,
            block,
            in_volume,
        });
    }
    None
}

/// Draws `bars` with `set` as a TPO into a picture named `name` in the gallery folder (the live tests'
/// way of showing what real data looks like). Does nothing without `DEGENVIEW_GALLERY_DIR`.
#[cfg(test)]
pub(crate) fn gallery_picture(name: &str, bars: &[Kline], set: &std::sync::Arc<TpoSet>) {
    use crate::model::PriceFormat;
    use crate::model::chart_look::Palette;
    use crate::model::chart_type::{ChartStyle, ChartType};
    use crate::model::market_profile::feed::ProfileFeed;
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() || bars.is_empty() {
        return;
    }
    let feed = ProfileFeed {
        tpo: Some(set.clone()),
        ..ProfileFeed::default()
    };
    let (width, height) = (1500.0_f32, 900.0_f32);
    let slot = width / bars.len() as f32;
    let (low, high) = bars.iter().fold((f64::MAX, f64::MIN), |(l, h), bar| {
        (l.min(bar.low), h.max(bar.high))
    });
    let pad = (high - low) * 0.05;
    let style = ChartStyle {
        chart_type: ChartType::TimePriceOpportunity,
        ..ChartStyle::default()
    };
    let mut screen = Screen::new(eframe::egui::vec2(width, height));
    let mut last = None;
    for _ in 0..2 {
        last = Some(screen.run(Vec::new(), |ui| {
            let plot = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(width, height));
            ui.painter().rect_filled(plot, 0.0, theme::chrome());
            let scale = PriceScale::linear(low - pad, high + pad, 0.0, height);
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([38, 166, 154], [239, 83, 80]),
                style: &style,
                chart_type: ChartType::TimePriceOpportunity,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: Some(&feed),
            });
        }));
    }
    screen.save(name, last.expect("a frame"));
}

/// The context-menu target of the letter under `pointer`.
pub fn target(
    geo: &Geo,
    set: &TpoSet,
    settings: &TpoSettings,
    pointer: Pos2,
) -> Option<crate::model::chart_context::Target> {
    let hit = hit(geo, set, settings, pointer)?;
    let block = hit.block?;
    Some(crate::model::chart_context::Target::TpoBlock {
        letter: letter(block),
        split_at_ms: hit.profile.start_ms + i64::from(block) * set.block_ms,
        first_block: block == 0,
        merge_at_ms: set
            .previous_span(hit.profile.start_ms)
            .map(|_| hit.profile.start_ms),
    })
}

/// "09:30" for a time of day in UTC.
fn clock(time_ms: i64) -> String {
    let minutes = time_ms.div_euclid(60_000).rem_euclid(1_440);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The tooltip for what is under `pointer`: the row's price range and count, the letter's block and
/// time, or the volume of the row, then the profile's period and levels.
pub fn hover(
    frame: &SeriesFrame,
    pointer: Pos2,
) -> Option<crate::ui::pine_chart_render::HoverTarget> {
    let set = frame.profiles?.tpo.as_deref()?;
    let geo = Geo::of(frame);
    let settings = frame.style.tpo;
    let hit = hit(&geo, set, &settings, pointer)?;
    let grid = set.grid?;
    let profile = hit.profile;
    let format = |price: f64| frame.price_format.format(price);
    let row = profile.row_at(hit.row)?;
    let (top, bottom) = (geo.y(grid.top(hit.row)), geo.y(grid.bottom(hit.row)));
    let mut lines = vec![format!(
        "{} – {}",
        format(grid.bottom(hit.row)),
        format(grid.top(hit.row))
    )];
    if hit.in_volume {
        lines.push(format!(
            "Volume {}",
            super::footprint::format_volume(row.volume)
        ));
    } else {
        let letters: String = row.blocks.iter().map(|b| letter(*b)).collect();
        lines.push(format!("TPO count {} ({letters})", row.count()));
        if let Some(block) = hit.block {
            let start = profile.start_ms + i64::from(block) * set.block_ms;
            lines.push(format!(
                "Block {}: {} – {} UTC",
                letter(block),
                clock(start),
                clock(start + set.block_ms)
            ));
        }
        lines.push(format!(
            "Volume {}",
            super::footprint::format_volume(row.volume)
        ));
    }
    let index = (hit.row - profile.first_row()) as usize;
    if index == profile.poc {
        lines.push("Point of control".to_owned());
    } else if (profile.value_area.0..=profile.value_area.1).contains(&index) {
        lines.push("In the value area".to_owned());
    }
    let (va_low, va_high) = profile.value_area;
    lines.push(format!(
        "Profile: POC {}, VAH {}, VAL {}",
        format(grid.middle(profile.poc_row())),
        format(grid.top(profile.rows[va_high].row)),
        format(grid.bottom(profile.rows[va_low].row))
    ));
    lines.push(format!(
        "{} blocks, {} TPOs{}",
        profile.blocks,
        profile.total,
        if profile.forming {
            ", still forming"
        } else {
            ""
        }
    ));
    Some(crate::ui::pine_chart_render::HoverTarget {
        rect: Rect::from_min_max(
            Pos2::new(pointer.x - 3.0, top.min(bottom)),
            Pos2::new(pointer.x + 3.0, top.max(bottom)),
        ),
        text: lines.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_follows_the_room_a_letter_has() {
        assert_eq!(detail(12.0, 8.0), Detail::Letters);
        assert_eq!(detail(8.0, 8.0), Detail::Blocks, "too short for a letter");
        assert_eq!(detail(12.0, 5.0), Detail::Blocks, "too narrow for a letter");
        assert_eq!(detail(1.0, 8.0), Detail::Bars);
        assert_eq!(detail(12.0, 1.0), Detail::Bars);
    }

    #[test]
    fn a_blocks_colour_follows_the_gradient_halves() {
        let settings = TpoSettings::default();
        assert_eq!(block_color(&settings, 0), DEFAULT_GRADIENT[0]);
        assert_eq!(block_color(&settings, 25), DEFAULT_GRADIENT[1]);
        assert_eq!(block_color(&settings, 26), DEFAULT_GRADIENT[2]);
        assert_eq!(block_color(&settings, 51), DEFAULT_GRADIENT[3]);
        assert_eq!(
            block_color(&settings, 52),
            DEFAULT_GRADIENT[0],
            "the sequence repeats"
        );
        let middle = block_color(&settings, 12);
        assert!(middle != DEFAULT_GRADIENT[0] && middle != DEFAULT_GRADIENT[1]);
        let custom = TpoSettings {
            gradient: [Some([1, 2, 3, 255]), Some([1, 2, 3, 255]), None, None],
            ..TpoSettings::default()
        };
        assert_eq!(block_color(&custom, 7), [1, 2, 3, 255]);
    }

    // MARK: Drawing

    use crate::model::chart_look::Palette;
    use crate::model::chart_type::{ChartStyle, ChartType};
    use crate::model::market_profile::feed::ProfileFeed;
    use crate::model::{ChartAppearance, PriceFormat};
    use crate::ui::chart_style_preview::{sample_candles, sample_tpo};
    use eframe::egui;

    fn bars() -> Vec<Kline> {
        sample_candles()[..24].to_vec()
    }

    fn style() -> ChartStyle {
        ChartStyle {
            chart_type: ChartType::TimePriceOpportunity,
            ..ChartStyle::default()
        }
    }

    fn feed(style: &ChartStyle) -> ProfileFeed {
        sample_tpo(&ChartAppearance {
            style: *style,
            ..ChartAppearance::default()
        })
    }

    /// The scale and the x of bar `n`, for a plot `width` by `height` showing the 24 sample bars.
    fn setup(width: f32, height: f32) -> (Rect, PriceScale, f32) {
        let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(width, height));
        (
            plot,
            PriceScale::linear(95.0, 125.0, 0.0, height),
            width / 24.0,
        )
    }

    fn paint_with(
        feed: Option<&ProfileFeed>,
        style: &ChartStyle,
        width: f32,
        height: f32,
    ) -> (usize, Vec<String>) {
        let bars = bars();
        let (plot, scale, slot) = setup(width, height);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars: &bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style,
                chart_type: ChartType::TimePriceOpportunity,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: feed,
            });
        });
        output.textures_delta.clear();
        let shapes = std::mem::take(&mut output.shapes);
        let words = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        (shapes.len(), words)
    }

    #[test]
    fn a_profile_with_room_writes_letters_and_one_without_draws_blocks_or_bars() {
        let style = style();
        let feed = feed(&style);
        let (_, words) = paint_with(Some(&feed), &style, 2400.0, 3000.0);
        let letters = words
            .iter()
            .filter(|w| w.len() == 1 && w.chars().all(|c| c.is_ascii_alphabetic()))
            .count();
        assert!(letters > 50, "letters are written: {letters}");
        let (narrow_shapes, narrow_words) = paint_with(Some(&feed), &style, 300.0, 200.0);
        let narrow_letters = narrow_words
            .iter()
            .filter(|w| w.len() == 1 && w.chars().all(|c| c.is_ascii_alphabetic()))
            .count();
        assert_eq!(narrow_letters, 0, "no room for letters");
        assert!(narrow_shapes > 0);
    }

    #[test]
    fn no_feed_draws_candles_and_says_it_is_loading_and_a_failed_load_says_why() {
        let (shapes, words) = paint_with(None, &style(), 600.0, 300.0);
        assert!(shapes > 0);
        assert!(words.iter().any(|w| w.contains("Loading")), "{words:?}");
        let failed = ProfileFeed {
            error: Some("Network error: offline".into()),
            ..ProfileFeed::default()
        };
        let (_, words) = paint_with(Some(&failed), &style(), 600.0, 300.0);
        assert!(words.iter().any(|w| w.contains("offline")), "{words:?}");
    }

    #[test]
    fn the_status_line_says_what_the_profiles_are_built_from() {
        let style = style();
        let feed = feed(&style);
        let (_, words) = paint_with(Some(&feed), &style, 600.0, 300.0);
        assert!(
            words
                .iter()
                .any(|w| w.contains("5-minute candles") && w.contains("30-minute blocks")),
            "{words:?}"
        );
    }

    #[test]
    fn every_switch_paints_without_panicking() {
        let base = style();
        let feed = feed(&base);
        for expand in [false, true] {
            for letters in [false, true] {
                for blocks in [false, true] {
                    for volume in [false, true] {
                        let mut style = base;
                        style.tpo.expand_blocks = expand;
                        style.tpo.show_letters = letters;
                        style.tpo.show_blocks = blocks;
                        style.tpo.volume.show = volume;
                        style.tpo.volume.values = volume;
                        style.tpo.volume.placement = if expand {
                            VolumePlacement::Left
                        } else {
                            VolumePlacement::Right
                        };
                        style.tpo.midpoint.show = true;
                        style.tpo.open.show = true;
                        style.tpo.close.show = true;
                        let (shapes, _) = paint_with(Some(&feed), &style, 1200.0, 900.0);
                        assert!(shapes > 0, "{expand} {letters} {blocks} {volume}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_x_of_a_time_is_linear_across_bars_and_runs_on_past_the_ends() {
        let bars = bars();
        let (plot, scale, slot) = setup(2400.0, 600.0);
        let x_for = |index: usize| slot * (index as f32 + 0.5);
        let geo = Geo {
            plot,
            bars: &bars,
            x_for: &x_for,
            slot_width: slot,
            scale: &scale,
        };
        assert!((geo.x_at(bars[0].open_time) - 0.0).abs() < 1e-3);
        assert!((geo.x_at(bars[1].open_time) - slot).abs() < 1e-3);
        assert!((geo.x_at(bars[1].open_time + 1_800_000) - 1.5 * slot).abs() < 1e-3);
        assert!(geo.x_at(bars[0].open_time - 3_600_000) < 0.0);
        assert!((geo.x_at(bars[23].open_time + 3_600_000) - 24.0 * slot).abs() < 1e-3);
    }

    #[test]
    fn the_hover_and_the_menu_find_the_letter_the_painter_drew() {
        let style = style();
        let feed = feed(&style);
        let set = feed.tpo.as_deref().unwrap();
        let bars = bars();
        let (plot, scale, slot) = setup(2400.0, 3000.0);
        let x_for = |index: usize| slot * (index as f32 + 0.5);
        let geo = Geo {
            plot,
            bars: &bars,
            x_for: &x_for,
            slot_width: slot,
            scale: &scale,
        };
        let profile = &set.profiles[0];
        let grid = set.grid.unwrap();
        let row_height =
            (geo.y(grid.bottom(profile.first_row())) - geo.y(grid.top(profile.first_row()))).abs();
        let lay = layout(&geo, profile, &style.tpo, row_height, 0);
        // The first letter of the POC row.
        let poc = &profile.rows[profile.poc];
        let pointer = Pos2::new(
            lay.letters_x0 + lay.column * 0.5,
            geo.y(grid.middle(poc.row)),
        );
        let found = hit(&geo, set, &style.tpo, pointer).expect("over a letter");
        assert_eq!(found.row, poc.row);
        assert_eq!(found.block, Some(poc.blocks[0]));
        // The second letter is the second block of that row.
        let second = Pos2::new(lay.letters_x0 + lay.column * 1.5, pointer.y);
        assert_eq!(
            hit(&geo, set, &style.tpo, second).unwrap().block,
            Some(poc.blocks[1])
        );
        // Past the letters of the row: the row, but no letter.
        let past = Pos2::new(
            lay.letters_x0 + lay.column * (poc.count() as f32 + 0.5),
            pointer.y,
        );
        let past_hit = hit(&geo, set, &style.tpo, past);
        assert!(past_hit.is_none_or(|hit| hit.block.is_none()));
        // The menu target names the letter and where it would split.
        let target = target(&geo, set, &style.tpo, pointer).expect("a target");
        match target {
            crate::model::chart_context::Target::TpoBlock {
                letter: l,
                split_at_ms,
                first_block,
                merge_at_ms,
            } => {
                assert_eq!(l, letter(poc.blocks[0]));
                assert_eq!(
                    split_at_ms,
                    profile.start_ms + i64::from(poc.blocks[0]) * set.block_ms
                );
                assert_eq!(first_block, poc.blocks[0] == 0);
                assert_eq!(merge_at_ms, None, "a lone profile has nothing before it");
            }
            other => panic!("{other:?}"),
        }
        // Outside the plot or off every profile: nothing.
        assert!(hit(&geo, set, &style.tpo, Pos2::new(-5.0, 10.0)).is_none());
    }

    /// Pictures of the profile at the sizes and settings worth a look.
    /// `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib gallery_of_the_tpo -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_tpo() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let bars = bars();
        type Tweak = fn(&mut ChartStyle);
        let cases: [(&str, f32, f32, Tweak); 5] = [
            ("tpo-wide", 900.0, 640.0, |_| {}),
            ("tpo-narrow", 300.0, 300.0, |_| {}),
            ("tpo-volume-left-levels", 900.0, 640.0, |s| {
                s.tpo.volume.placement = VolumePlacement::Left;
                s.tpo.volume.values = true;
                s.tpo.midpoint.show = true;
                s.tpo.open.show = true;
                s.tpo.close.show = true;
            }),
            ("tpo-expanded-no-blocks", 900.0, 640.0, |s| {
                s.tpo.expand_blocks = true;
                s.tpo.show_blocks = false;
            }),
            ("tpo-hourly-blocks", 900.0, 640.0, |s| {
                s.tpo.block_minutes = 60;
            }),
        ];
        for (name, width, height, tweak) in cases {
            let mut style = style();
            tweak(&mut style);
            let feed = feed(&style);
            let mut screen = Screen::new(egui::vec2(width, height));
            let mut last = None;
            for _ in 0..2 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let (plot, scale, slot) = setup(width, height);
                    ui.painter().rect_filled(plot, 0.0, theme::chrome());
                    let x_for = |index: usize| slot * (index as f32 + 0.5);
                    paint(&SeriesFrame {
                        painter: ui.painter(),
                        plot,
                        bars: &bars,
                        synthetic: None,
                        box_size: None,
                        previous_close: None,
                        slot_width: slot,
                        x_for: &x_for,
                        scale: &scale,
                        palette: Palette::new([38, 166, 154], [239, 83, 80]),
                        style: &style,
                        chart_type: ChartType::TimePriceOpportunity,
                        recolor: None,
                        price_format: PriceFormat::AUTO,
                        profiles: Some(&feed),
                    });
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}
