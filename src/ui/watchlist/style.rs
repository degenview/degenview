//! Sizes and colours the watchlist panel shares, so its parts line up by construction.

use crate::model::watchlist::{WatchlistColumn, WatchlistFlag};
use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// Where left-aligned content starts, measured from the panel's left edge: asset logos,
/// section titles, the "Symbol" column title and the panel header share this one value.
/// It is narrower than the right edge so a flag's bookmark, which touches the border,
/// sits behind the logo.
pub const LEADING_INSET: f32 = 8.0;
/// Where right-aligned content ends, measured from the panel's right edge: column
/// values, column titles, chevrons and the header buttons.
pub const TRAILING_INSET: f32 = 12.0;
/// A flag is drawn from x = 0, touching the panel's left edge, and ends short of the logo.
pub const FLAG_SIZE: Vec2 = Vec2::new(6.0, 14.0);
pub const ROW_HEIGHT_WITH_SUBTITLE: f32 = 40.0;
pub const ROW_HEIGHT_COMPACT: f32 = 30.0;
pub const LOGO_SIZE: f32 = 20.0;
/// Spacing between the value columns.
pub const COLUMN_GAP: f32 = 6.0;

pub fn column_width(column: WatchlistColumn) -> f32 {
    match column {
        WatchlistColumn::Last => 76.0,
        WatchlistColumn::Change => 70.0,
        WatchlistColumn::ChangePercent => 74.0,
        WatchlistColumn::Volume => 60.0,
        WatchlistColumn::Exchange => 82.0,
        WatchlistColumn::Status => 76.0,
    }
}

/// Width of the symbol block, the part that gives way when the panel narrows, that the
/// panel needs to keep a name legible.
#[allow(dead_code)] // the split's minimum width reads these
pub const MIN_NAME_WIDTH: f32 = 78.0;

/// The narrowest the panel can be and still show every column plus room for a name.
#[allow(dead_code)] // the split's minimum width reads these
pub fn minimum_panel_width(columns: &[WatchlistColumn]) -> f32 {
    let values: f32 = columns
        .iter()
        .map(|column| column_width(*column) + COLUMN_GAP)
        .sum();
    LEADING_INSET + LOGO_SIZE + 6.0 + MIN_NAME_WIDTH + values + TRAILING_INSET
}

pub fn flag_color(flag: WatchlistFlag) -> Color32 {
    match flag {
        WatchlistFlag::Red => Color32::from_rgb(255, 69, 58),
        WatchlistFlag::Orange => Color32::from_rgb(255, 159, 10),
        WatchlistFlag::Yellow => Color32::from_rgb(255, 214, 10),
        WatchlistFlag::Green => Color32::from_rgb(48, 209, 88),
        WatchlistFlag::Blue => Color32::from_rgb(10, 132, 255),
        WatchlistFlag::Purple => Color32::from_rgb(191, 90, 242),
    }
}

/// The bookmark turned on its side: a flat end with softly rounded corners against the
/// panel's edge and a V cut into the tail, which points toward the logo. The V makes the
/// outline concave, and egui fills only convex polygons, so it comes as two halves
/// (upper, lower) that meet along the centre line.
pub fn flag_halves(left_top: Pos2, size: Vec2, notch_depth: f32) -> [Vec<Pos2>; 2] {
    let (width, height) = (size.x, size.y);
    let notch = notch_depth.min(width / 2.0);
    let radius = 1.5_f32.min(width.min(height) / 2.0);
    let at = |x: f32, y: f32| left_top + Vec2::new(x, y);
    let middle = height / 2.0;
    [
        vec![
            at(radius, 0.0),
            at(width, 0.0),
            at(width - notch, middle),
            at(0.0, middle),
            at(0.0, radius),
        ],
        vec![
            at(0.0, middle),
            at(width - notch, middle),
            at(width, height),
            at(radius, height),
            at(0.0, height - radius),
        ],
    ]
}

/// Paints a flag centred vertically on `y`, touching the left edge of `panel_left`.
pub fn paint_flag(painter: &Painter, panel_left: f32, y: f32, flag: WatchlistFlag) {
    let top_left = Pos2::new(panel_left, y - FLAG_SIZE.y / 2.0);
    for half in flag_halves(top_left, FLAG_SIZE, 2.5) {
        painter.add(Shape::convex_polygon(half, flag_color(flag), Stroke::NONE));
    }
}

/// A small flag mark for menus, in the flag's colour, within the 12-point square `rect`.
pub fn paint_swatch(painter: &Painter, rect: Rect, flag: WatchlistFlag) {
    let top_left = Pos2::new(rect.left() + 2.0, rect.top() - 1.0);
    for half in flag_halves(top_left, Vec2::new(8.0, 14.0), 3.0) {
        painter.add(Shape::convex_polygon(half, flag_color(flag), Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_minimum_grows_with_columns() {
        let two = minimum_panel_width(&[WatchlistColumn::Last, WatchlistColumn::ChangePercent]);
        let three = minimum_panel_width(&[
            WatchlistColumn::Last,
            WatchlistColumn::ChangePercent,
            WatchlistColumn::Volume,
        ]);
        assert!(three > two);
        assert_eq!(
            three - two,
            column_width(WatchlistColumn::Volume) + COLUMN_GAP
        );
    }

    #[test]
    fn flag_is_narrower_than_the_leading_inset_and_touches_the_edge() {
        const { assert!(FLAG_SIZE.x < LEADING_INSET) };
        let halves = flag_halves(Pos2::new(0.0, 0.0), FLAG_SIZE, 2.5);
        let points = halves.iter().flatten();
        let min_x = points.clone().map(|p| p.x).fold(f32::MAX, f32::min);
        let max_x = points.map(|p| p.x).fold(f32::MIN, f32::max);
        assert_eq!((min_x, max_x), (0.0, FLAG_SIZE.x));
    }

    #[test]
    fn every_flag_has_a_distinct_colour() {
        let colours: std::collections::HashSet<_> = WatchlistFlag::ALL
            .iter()
            .map(|flag| flag_color(*flag))
            .collect();
        assert_eq!(colours.len(), 6);
    }
}
