//! A market's last week drawn small: a line with a soft fade under it, green when the week ended
//! above where it began. Painter code and one pure mapping, so the shape can be tested.

use crate::model::chart::price_format::PriceFormat;
use crate::ui::theme;
use eframe::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// What the page knows about a market's last week. A market the page has not heard about yet has
/// no `Trend` at all (it is still loading).
#[derive(Clone, Debug, PartialEq)]
pub enum Trend {
    /// Daily closes, oldest first.
    Ready(Vec<f32>),
    /// The market has no daily history to show.
    Unavailable,
}

/// Share of the height kept clear above and below the line, so a stroke never touches the edge.
const MARGIN: f32 = 0.12;
/// How strong the fade is at the line; it reaches nothing at the bottom.
const FILL_ALPHA: f32 = 0.26;

/// Where each close sits in `rect`: the lowest on the bottom margin, the highest on the top one.
/// A flat week runs through the middle.
pub fn points(closes: &[f32], rect: Rect) -> Vec<Pos2> {
    let (low, high) = closes
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), &close| {
            (low.min(close), high.max(close))
        });
    let span = high - low;
    let inner = rect.height() * (1.0 - 2.0 * MARGIN);
    let last = closes.len().saturating_sub(1).max(1) as f32;
    closes
        .iter()
        .enumerate()
        .map(|(index, &close)| {
            let level = if span > f32::EPSILON {
                (close - low) / span
            } else {
                0.5
            };
            pos2(
                rect.left() + rect.width() * index as f32 / last,
                rect.bottom() - rect.height() * MARGIN - inner * level,
            )
        })
        .collect()
}

/// A week's move as the page writes it: the share, and the amount when a price difference means
/// something (a prediction market quotes probabilities, so only the share does).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeText {
    pub percent: String,
    pub amount: Option<String>,
}

/// How far the week ended from where it began, written out; `None` without a usable first close.
pub fn change_text(closes: &[f32], with_amount: bool) -> Option<ChangeText> {
    let (first, last) = (f64::from(*closes.first()?), f64::from(*closes.last()?));
    let amount = last - first;
    let percent = amount / first * 100.0;
    if closes.len() < 2 || !percent.is_finite() || first == 0.0 {
        return None;
    }
    let flat = percent.abs() < 0.005;
    let sign = if amount < 0.0 { "-" } else { "+" };
    Some(ChangeText {
        percent: if flat {
            "0.00%".to_owned()
        } else {
            format!("{sign}{:.2}%", percent.abs())
        },
        amount: with_amount.then(|| {
            if flat {
                PriceFormat::AUTO.format(0.0)
            } else {
                format!("{sign}{}", PriceFormat::AUTO.format(amount.abs()))
            }
        }),
    })
}

/// Whether the series ended where it began or higher.
pub fn rising(closes: &[f32]) -> bool {
    match (closes.first(), closes.last()) {
        (Some(first), Some(last)) => last >= first,
        _ => true,
    }
}

/// Draws `trend` in `rect`. Still loading: a faint flat line, so the tile does not jump when the
/// week arrives. No history: nothing.
pub fn paint(painter: &Painter, rect: Rect, trend: Option<&Trend>) {
    match trend {
        Some(Trend::Ready(closes)) if closes.len() >= 2 => {
            let color = if rising(closes) {
                theme::UP
            } else {
                theme::DOWN
            };
            let line = points(closes, rect);
            fade_fill(painter, &line, rect.bottom(), color, FILL_ALPHA, |_| 1.0);
            painter.add(Shape::line(line, Stroke::new(1.5, color)));
        }
        Some(_) => {}
        None => {
            let y = rect.center().y;
            painter.hline(rect.x_range(), y, Stroke::new(1.0, theme::ink(22)));
        }
    }
}

/// Fills the area between `line` and `bottom` with `color`, strongest at the line (`top_alpha`
/// times `weight(x)`) and gone at the bottom. A vertex-coloured mesh, so there is no gradient
/// texture and the fade costs nothing per frame.
pub fn fade_fill(
    painter: &Painter,
    line: &[Pos2],
    bottom: f32,
    color: Color32,
    top_alpha: f32,
    weight: impl Fn(f32) -> f32,
) {
    if line.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    for point in line {
        let alpha = (top_alpha * weight(point.x)).clamp(0.0, 1.0);
        mesh.colored_vertex(*point, color.gamma_multiply(alpha));
        mesh.colored_vertex(pos2(point.x, bottom.max(point.y)), Color32::TRANSPARENT);
    }
    for segment in 0..line.len() as u32 - 1 {
        let at = segment * 2;
        mesh.add_triangle(at, at + 1, at + 2);
        mesh.add_triangle(at + 1, at + 3, at + 2);
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 50.0))
    }

    #[test]
    fn the_lowest_close_sits_low_and_the_highest_high() {
        let points = points(&[3.0, 1.0, 2.0, 5.0], rect());
        assert_eq!(points.len(), 4);
        assert_eq!(points[0].x, 10.0);
        assert_eq!(points[3].x, 110.0);
        let lowest = points[1].y;
        let highest = points[3].y;
        assert!(lowest > highest, "a larger close is drawn higher up");
        assert!(lowest <= rect().bottom() && highest >= rect().top());
    }

    #[test]
    fn a_flat_week_runs_through_the_middle() {
        let points = points(&[4.0, 4.0, 4.0], rect());
        assert!(points.iter().all(|point| point.y == rect().center().y));
    }

    #[test]
    fn the_change_reads_as_a_signed_amount_and_share() {
        let up = change_text(&[100.0, 90.0, 112.5], true).expect("a change");
        assert_eq!(up.percent, "+12.50%");
        assert_eq!(up.amount.as_deref(), Some("+12.50"));
        let down = change_text(&[2000.0, 1900.0], true).expect("a change");
        assert_eq!(down.percent, "-5.00%");
        assert_eq!(down.amount.as_deref(), Some("-100.00"));
        let flat = change_text(&[5.0, 5.0], true).expect("a change");
        assert_eq!(flat.percent, "0.00%");
        assert_eq!(
            change_text(&[0.4, 0.5], false).expect("a change").amount,
            None
        );
    }

    #[test]
    fn no_change_without_a_week_or_a_first_close() {
        assert_eq!(change_text(&[], true), None);
        assert_eq!(change_text(&[3.0], true), None);
        assert_eq!(change_text(&[0.0, 1.0], true), None);
    }

    #[test]
    fn rising_compares_the_ends() {
        assert!(rising(&[1.0, 0.5, 2.0]));
        assert!(rising(&[2.0, 2.0]));
        assert!(!rising(&[2.0, 3.0, 1.0]));
    }
}
