//! Journal trades on a chart: a triangle at each entry fill, a diamond at each exit, a dashed
//! line between them in the result's colour, and a chip with the net result.
//!
//! The geometry is separate from the painting so it can be tested and so hovering uses exactly what
//! was drawn. Prices are the fills' own, never a constructed bar's, and nothing here feeds the
//! chart's autoscale: a mark that is off the price range is clipped to the plot, not chased. A
//! trade's shape and its chip's sign say the result as well as its colour.

use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use super::theme;
use degenview_journal::filters::OutcomeFilter;
use degenview_journal::format;
use degenview_journal::marks::TradeMark;
use degenview_journal::model::{Direction, Side};

/// Past this many marks in view only the glyphs are drawn: lines and chips would be a smear.
const DETAIL_LIMIT: usize = 120;
/// The most marks laid out in one frame.
const DRAW_LIMIT: usize = 600;
const ENTRY_SIZE: f32 = 8.0;
const EXIT_SIZE: f32 = 5.5;
const HIT_RADIUS: f32 = 9.0;

/// Colours for the three results, from the chart's own up and down colours.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub win: Color32,
    pub loss: Color32,
    pub flat: Color32,
    pub open: Color32,
}

impl Palette {
    pub fn new(up: Color32, down: Color32) -> Self {
        Self {
            win: up,
            loss: down,
            flat: theme::text_secondary(),
            open: theme::text_secondary(),
        }
    }

    fn of(&self, outcome: OutcomeFilter) -> Color32 {
        match outcome {
            OutcomeFilter::Win => self.win,
            OutcomeFilter::Loss => self.loss,
            OutcomeFilter::Breakeven | OutcomeFilter::Undetermined => self.flat,
            OutcomeFilter::Open => self.open,
        }
    }
}

/// One fill's place on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub at: Pos2,
    pub side: Side,
    pub is_entry: bool,
}

/// One trade as laid out for the current frame.
#[derive(Clone, Debug)]
pub struct Item {
    /// Index into the marks it was laid out from.
    pub index: usize,
    pub glyphs: Vec<Glyph>,
    /// The dashed line: from the average entry to the average exit.
    pub line: Option<(Pos2, Pos2)>,
    pub chip: Option<Rect>,
}

/// Lays out the marks that touch the plot. `x_of` and `y_of` are the chart's own time and price
/// maps. `measure` is the size of a chip's text.
pub fn layout(
    marks: &[TradeMark],
    plot: Rect,
    x_of: &dyn Fn(i64) -> f32,
    y_of: &dyn Fn(f64) -> f32,
    measure: &dyn Fn(&str) -> Vec2,
) -> Vec<Item> {
    let margin = 24.0;
    let reach = plot.expand2(vec2(margin, 0.0));
    let mut items: Vec<Item> = Vec::new();
    for (index, mark) in marks.iter().enumerate() {
        let (first, last) = mark.span();
        let (x0, x1) = (x_of(first), x_of(last));
        if !x0.is_finite() || !x1.is_finite() {
            continue;
        }
        if x1 < reach.left() || x0 > reach.right() {
            continue;
        }
        let glyphs: Vec<Glyph> = mark
            .fills
            .iter()
            .map(|fill| Glyph {
                at: pos2(x_of(fill.ts_ms), clamp_y(plot, y_of(fill.price))),
                side: fill.side,
                is_entry: fill.is_entry,
            })
            .filter(|glyph| glyph.at.x.is_finite() && glyph.at.y.is_finite())
            .collect();
        if glyphs.is_empty() {
            continue;
        }
        items.push(Item {
            index,
            glyphs,
            line: None,
            chip: None,
        });
        if items.len() >= DRAW_LIMIT {
            break;
        }
    }
    if items.len() > DETAIL_LIMIT {
        return items;
    }
    let mut taken: Vec<Rect> = Vec::new();
    for item in &mut items {
        let mark = &marks[item.index];
        let (Some(close), Some(exit)) = (mark.close_ts_ms, mark.avg_exit) else {
            continue;
        };
        let from = pos2(x_of(mark.open_ts_ms), clamp_y(plot, y_of(mark.avg_entry)));
        let to = pos2(x_of(close), clamp_y(plot, y_of(exit)));
        if from.x.is_finite() && to.x.is_finite() {
            item.line = Some((from, to));
        }
        let Some(label) = &mark.label else { continue };
        let size = measure(label) + vec2(12.0, 6.0);
        // Above the exit for a trade that ended higher than it started, below otherwise.
        let up = exit >= mark.avg_entry;
        let centre_y = if up {
            to.y - ENTRY_SIZE - size.y / 2.0 - 2.0
        } else {
            to.y + ENTRY_SIZE + size.y / 2.0 + 2.0
        };
        let rect = Rect::from_center_size(pos2(to.x, centre_y), size);
        let rect = rect.translate(vec2(
            (plot.left() + 2.0 - rect.left()).max(0.0)
                - (rect.right() - plot.right() + 2.0).max(0.0),
            (plot.top() + 2.0 - rect.top()).max(0.0)
                - (rect.bottom() - plot.bottom() - 2.0).max(0.0),
        ));
        if taken.iter().any(|other| other.expand(2.0).intersects(rect)) {
            continue;
        }
        taken.push(rect);
        item.chip = Some(rect);
    }
    items
}

fn clamp_y(plot: Rect, y: f32) -> f32 {
    y.clamp(plot.top() + 2.0, plot.bottom() - 2.0)
}

/// The mark under `pointer`, if any: a glyph, the chip, or near the dashed line.
pub fn hit(items: &[Item], pointer: Pos2) -> Option<usize> {
    // The newest on top: a later trade is drawn over an earlier one.
    items.iter().rev().find_map(|item| {
        let on_glyph = item
            .glyphs
            .iter()
            .any(|glyph| glyph.at.distance(pointer) <= HIT_RADIUS);
        let on_chip = item.chip.is_some_and(|chip| chip.contains(pointer));
        let on_line = item
            .line
            .is_some_and(|(a, b)| distance_to_segment(pointer, a, b) <= 4.0);
        (on_glyph || on_chip || on_line).then_some(item.index)
    })
}

fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length_squared = ab.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(a);
    }
    let t = (((point - a).dot(ab)) / length_squared).clamp(0.0, 1.0);
    point.distance(a + ab * t)
}

/// Paints what `layout` produced. `hovered` is the mark index under the pointer, drawn stronger.
pub fn paint(
    painter: &Painter,
    items: &[Item],
    marks: &[TradeMark],
    palette: &Palette,
    hovered: Option<usize>,
) {
    for item in items {
        let mark = &marks[item.index];
        let color = palette.of(mark.outcome);
        let strong = hovered == Some(item.index);
        if let Some((from, to)) = item.line {
            let stroke = Stroke::new(
                if strong { 1.8 } else { 1.2 },
                color.gamma_multiply(if strong { 1.0 } else { 0.75 }),
            );
            painter.extend(Shape::dashed_line(&[from, to], stroke, 4.0, 3.0));
        }
        for glyph in &item.glyphs {
            if glyph.is_entry {
                entry_glyph(painter, glyph, color, strong);
            } else {
                exit_glyph(painter, glyph.at, color, strong);
            }
        }
        if let (Some(chip), Some(label)) = (item.chip, &mark.label) {
            painter.rect_filled(chip, 5.0, color);
            painter.text(
                chip.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::monospace(10.5),
                Color32::WHITE,
            );
            if strong {
                painter.rect_stroke(
                    chip,
                    5.0,
                    Stroke::new(1.5, Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }
}

fn entry_glyph(painter: &Painter, glyph: &Glyph, color: Color32, strong: bool) {
    let size = ENTRY_SIZE * if strong { 1.25 } else { 1.0 };
    let direction = if glyph.side == Side::Buy { -1.0 } else { 1.0 };
    // A buy points up, a sell points down, whatever the trade's direction.
    let apex = glyph.at + vec2(0.0, direction * size * 0.6);
    let base_y = glyph.at.y - direction * size * 0.6;
    painter.add(Shape::convex_polygon(
        vec![
            apex,
            pos2(glyph.at.x - size * 0.7, base_y),
            pos2(glyph.at.x + size * 0.7, base_y),
        ],
        color,
        Stroke::new(1.0, theme::background()),
    ));
}

fn exit_glyph(painter: &Painter, at: Pos2, color: Color32, strong: bool) {
    let size = EXIT_SIZE * if strong { 1.25 } else { 1.0 };
    painter.add(Shape::convex_polygon(
        vec![
            pos2(at.x, at.y - size),
            pos2(at.x + size, at.y),
            pos2(at.x, at.y + size),
            pos2(at.x - size, at.y),
        ],
        color,
        Stroke::new(1.0, theme::background()),
    ));
}

/// What the tooltip says about a trade.
pub fn tooltip(mark: &TradeMark) -> Vec<String> {
    let side = match mark.direction {
        Direction::Long => "Long",
        Direction::Short => "Short",
    };
    let result = match mark.outcome {
        OutcomeFilter::Win => "win",
        OutcomeFilter::Loss => "loss",
        OutcomeFilter::Breakeven => "breakeven",
        OutcomeFilter::Undetermined => "fees unknown",
        OutcomeFilter::Open => "open",
    };
    let mut lines = vec![format!(
        "{} \u{b7} {side} \u{b7} {result}",
        mark.display_name
    )];
    let entry = format::price(rust_decimal::Decimal::try_from(mark.avg_entry).unwrap_or_default());
    match mark.avg_exit {
        Some(exit) => lines.push(format!(
            "{entry} \u{2192} {}",
            format::price(rust_decimal::Decimal::try_from(exit).unwrap_or_default())
        )),
        None => lines.push(format!("Entered at {entry}")),
    }
    lines.push(format!("Quantity {}", format::quantity(mark.quantity)));
    if let Some(label) = &mark.label {
        lines.push(match &mark.currency {
            Some(currency) => format!("Net {label} {currency}"),
            None => format!("Net {label}"),
        });
    }
    lines.push("Click to open in the Trade Journal".to_owned());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_journal::marks::MarkFill;
    use rust_decimal::Decimal;

    fn mark(
        key: &str,
        open: i64,
        close: Option<i64>,
        entry: f64,
        exit: Option<f64>,
        label: &str,
    ) -> TradeMark {
        let mut fills = vec![MarkFill {
            ts_ms: open,
            price: entry,
            quantity: 1.0,
            side: Side::Buy,
            is_entry: true,
        }];
        if let (Some(close), Some(exit)) = (close, exit) {
            fills.push(MarkFill {
                ts_ms: close,
                price: exit,
                quantity: 1.0,
                side: Side::Sell,
                is_entry: false,
            });
        }
        TradeMark {
            key: key.into(),
            account_uid: "a".into(),
            direction: Direction::Long,
            outcome: match exit {
                Some(exit) if exit > entry => OutcomeFilter::Win,
                Some(_) => OutcomeFilter::Loss,
                None => OutcomeFilter::Open,
            },
            open_ts_ms: open,
            close_ts_ms: close,
            fills,
            avg_entry: entry,
            avg_exit: exit,
            label: exit.map(|_| label.to_owned()),
            currency: Some("USD".into()),
            quantity: Decimal::ONE,
            display_name: "BTCUSDT".into(),
        }
    }

    fn plot() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0))
    }

    /// One pixel per second, one pixel per price unit upwards from 500.
    fn lay(marks: &[TradeMark]) -> Vec<Item> {
        layout(
            marks,
            plot(),
            &|ms| (ms / 1_000) as f32,
            &|price| 500.0 - price as f32,
            &|text| vec2(text.len() as f32 * 6.0, 12.0),
        )
    }

    #[test]
    fn a_trade_is_placed_at_its_fill_prices_and_times() {
        let marks = [mark(
            "a",
            100_000,
            Some(300_000),
            100.0,
            Some(150.0),
            "+50.00",
        )];
        let items = lay(&marks);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.glyphs[0].at, pos2(100.0, 400.0));
        assert_eq!(item.glyphs[1].at, pos2(300.0, 350.0));
        assert_eq!(item.line, Some((pos2(100.0, 400.0), pos2(300.0, 350.0))));
        let chip = item.chip.expect("chip");
        // A trade that ended higher keeps its chip above the exit.
        assert!(chip.bottom() < 350.0);
        assert!(plot().contains_rect(chip));
    }

    #[test]
    fn a_trade_wholly_outside_the_plot_is_skipped_and_one_crossing_it_is_kept() {
        let marks = [
            mark("left", 0, Some(1_000), 100.0, Some(110.0), "+10.00"),
            mark(
                "cross",
                900_000,
                Some(1_500_000),
                100.0,
                Some(110.0),
                "+10.00",
            ),
            mark(
                "right",
                2_000_000,
                Some(2_001_000),
                100.0,
                Some(110.0),
                "+10.00",
            ),
        ];
        let shifted = layout(
            &marks,
            plot().translate(vec2(500.0, 0.0)),
            &|ms| (ms / 1_000) as f32,
            &|price| 500.0 - price as f32,
            &|_| vec2(40.0, 12.0),
        );
        let keys: Vec<_> = shifted
            .iter()
            .map(|item| marks[item.index].key.as_str())
            .collect();
        assert_eq!(keys, ["cross"]);
    }

    #[test]
    fn prices_off_the_scale_are_clipped_to_the_plot_not_chased() {
        let marks = [mark(
            "a",
            100_000,
            Some(200_000),
            10_000.0,
            Some(-10_000.0),
            "-1.00",
        )];
        let items = lay(&marks);
        for glyph in &items[0].glyphs {
            assert!(glyph.at.y >= 2.0 && glyph.at.y <= 498.0, "{glyph:?}");
        }
    }

    #[test]
    fn chips_that_would_overlap_are_dropped_and_a_crowd_loses_its_lines() {
        let close = [
            mark("a", 100_000, Some(300_000), 100.0, Some(150.0), "+50.00"),
            mark("b", 110_000, Some(302_000), 100.0, Some(150.0), "+50.00"),
        ];
        let items = lay(&close);
        assert!(items[0].chip.is_some());
        assert!(
            items[1].chip.is_none(),
            "the second chip would sit on the first"
        );
        assert!(items[1].line.is_some());

        let crowd: Vec<_> = (0..DETAIL_LIMIT as i64 + 5)
            .map(|n| {
                mark(
                    &n.to_string(),
                    100_000 + n * 1_000,
                    Some(101_000 + n * 1_000),
                    100.0,
                    Some(110.0),
                    "+1.00",
                )
            })
            .collect();
        let items = lay(&crowd);
        assert!(
            items
                .iter()
                .all(|item| item.line.is_none() && item.chip.is_none())
        );
    }

    #[test]
    fn the_pointer_finds_a_glyph_the_chip_or_the_line() {
        let marks = [mark(
            "a",
            100_000,
            Some(300_000),
            100.0,
            Some(150.0),
            "+50.00",
        )];
        let items = lay(&marks);
        assert_eq!(hit(&items, pos2(102.0, 402.0)), Some(0), "entry glyph");
        assert_eq!(hit(&items, pos2(300.0, 351.0)), Some(0), "exit glyph");
        assert_eq!(
            hit(&items, items[0].chip.unwrap().center()),
            Some(0),
            "chip"
        );
        assert_eq!(hit(&items, pos2(200.0, 375.0)), Some(0), "on the line");
        assert_eq!(hit(&items, pos2(200.0, 100.0)), None);
    }

    #[test]
    fn painting_and_the_tooltip_cover_open_and_closed_trades() {
        let marks = [
            mark("a", 100_000, Some(300_000), 100.0, Some(150.0), "+50.00"),
            mark("b", 400_000, None, 120.0, None, ""),
        ];
        let items = lay(&marks);
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        let palette = Palette::new(Color32::GREEN, Color32::RED);
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(plot()),
                ..Default::default()
            },
            |ui| paint(&ui.painter_at(plot()), &items, &marks, &palette, Some(0)),
        );
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty());
        let lines = tooltip(&marks[0]);
        assert_eq!(lines[0], "BTCUSDT \u{b7} Long \u{b7} win");
        assert!(lines.iter().any(|line| line.contains("100 \u{2192} 150")));
        assert!(lines.iter().any(|line| line == "Net +50.00 USD"));
        let open = tooltip(&marks[1]);
        assert!(open.iter().any(|line| line.starts_with("Entered at 120")));
        assert!(!open.iter().any(|line| line.starts_with("Net")));
    }
}
