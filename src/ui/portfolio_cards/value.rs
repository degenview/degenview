//! The Portfolio Value card: today's total as one big number, what it did in the last day, and
//! a month of history faint behind it when the card is tall enough to carry it.

use super::{area_under, chip, fitted, loading_pill, reveal, tone_color};
use crate::model::portfolio::card::CardData;
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{Color32, Id, Pos2, Rect, Stroke, Ui, Vec2};

/// Below this height the sparkline would crowd the number.
const SPARKLINE_MIN_HEIGHT: f32 = 170.0;
/// Room kept around the text.
const PAD: f32 = 14.0;

/// Where the text and the history go in a card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Layout {
    pub text: Rect,
    pub spark: Option<Rect>,
}

/// The number sits at the left. A tall card carries the month under it; a wide, short one beside
/// it; one too short for either has the number alone, in the middle.
pub(super) fn layout(area: Rect) -> Layout {
    let inner = area.shrink2(Vec2::new(PAD, PAD - 2.0));
    if area.height() < SPARKLINE_MIN_HEIGHT {
        return Layout {
            text: inner,
            spark: None,
        };
    }
    if area.width() >= 440.0 && area.height() < 260.0 {
        let split = inner.left() + inner.width() * 0.46;
        return Layout {
            text: Rect::from_min_max(inner.min, Pos2::new(split - 12.0, inner.bottom())),
            spark: Some(Rect::from_min_max(
                Pos2::new(split, inner.top() + 10.0),
                Pos2::new(inner.right(), inner.bottom() - 6.0),
            )),
        };
    }
    let share = if area.height() > 380.0 { 0.48 } else { 0.4 };
    let band_top = area.bottom() - area.height() * share;
    Layout {
        text: Rect::from_min_max(inner.min, Pos2::new(inner.right(), band_top - 6.0)),
        spark: Some(Rect::from_min_max(
            Pos2::new(area.left() + 6.0, band_top),
            Pos2::new(area.right() - 6.0, area.bottom() - 6.0),
        )),
    }
}

pub(super) fn paint(ui: &Ui, area: Rect, id: Id, data: &CardData, _pointer: Option<Pos2>) {
    if data.loading {
        loading_pill(ui, area, "Loading market data…");
        return;
    }
    let layout = layout(area);
    if let Some(band) = layout.spark {
        paint_sparkline(ui, band, id, data);
    }
    let text = layout.text;
    let width = text.width().max(40.0);
    let galley = fitted(
        ui,
        &data.formatter.money(data.total_value),
        40.0,
        18.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        width,
    );

    // The number, the change chip and the caption are one block, centred in the text area when
    // the card has no history to fill the rest of it.
    let caption = if data.unpriced > 0 {
        format!(
            "{} {} could not be priced",
            data.unpriced,
            if data.unpriced == 1 {
                "asset"
            } else {
                "assets"
            }
        )
    } else {
        format!(
            "{} · {} {}",
            data.scope_name,
            data.holdings.len(),
            if data.holdings.len() == 1 {
                "asset"
            } else {
                "assets"
            }
        )
    };
    let caption_color = if data.unpriced > 0 {
        kit::ORANGE
    } else {
        theme::text_secondary()
    };
    let caption = kit::galley(ui, &caption, 11.5, kit::REGULAR, caption_color, width);
    let chip_row = if data.day_change.is_some() {
        20.0 + 10.0
    } else {
        0.0
    };
    let block = galley.size().y + 10.0 + chip_row + caption.size().y;
    let mut y = if layout.spark.is_some() {
        text.top() + 6.0
    } else {
        text.center().y - block / 2.0
    };
    ui.painter().galley(
        Pos2::new(text.left(), y),
        galley.clone(),
        Color32::PLACEHOLDER,
    );
    y += galley.size().y + 10.0;
    if let Some(change) = data.day_change {
        let color = tone_color(data.formatter.tone(Some(change.amount)));
        let arrow = if change.amount >= 0.0 { "↗" } else { "↘" };
        let label = match change.percentage {
            Some(fraction) => format!(
                "{arrow} {} · {}  24h",
                data.formatter.signed_money(change.amount),
                data.formatter.signed_percent(fraction, 2)
            ),
            None => format!(
                "{arrow} {}  24h",
                data.formatter.signed_money(change.amount)
            ),
        };
        chip(ui, Pos2::new(text.left(), y + 10.0), &label, color, width);
        y += 30.0;
    }
    ui.painter().galley(
        Pos2::new(text.left() + 1.0, y),
        caption,
        Color32::PLACEHOLDER,
    );
}

fn paint_sparkline(ui: &Ui, band: Rect, id: Id, data: &CardData) {
    let values: Vec<f64> = data.history.iter().map(|point| point.value).collect();
    if values.len() < 2 {
        return;
    }
    let (minimum, maximum) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
            (low.min(*value), high.max(*value))
        });
    let spread = (maximum - minimum).max(f64::EPSILON);
    let line: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Pos2::new(
                band.left() + band.width() * index as f32 / (values.len() - 1) as f32,
                band.bottom() - band.height() * ((value - minimum) / spread) as f32,
            )
        })
        .collect();
    let rising = values.last() >= values.first();
    ui.painter().text(
        Pos2::new(band.left() + 2.0, band.top() - 2.0),
        eframe::egui::Align2::LEFT_BOTTOM,
        "Past month",
        eframe::egui::FontId::proportional(10.0),
        theme::text_tertiary(),
    );
    let color = if rising { theme::UP } else { theme::DOWN };
    let progress = reveal(ui.ctx(), id.with("spark"), 0.7);
    let clip = Rect::from_min_max(
        band.min - Vec2::new(0.0, 4.0),
        Pos2::new(
            band.left() + band.width() * progress + 2.0,
            band.bottom() + 4.0,
        ),
    );
    let painter = ui.painter().with_clip_rect(clip);
    area_under(&painter, &line, band.bottom(), color, 0.22);
    let end = line[line.len() - 1];
    painter.add(eframe::egui::Shape::line(line, Stroke::new(1.75, color)));
    if progress >= 1.0 {
        ui.painter()
            .circle_filled(end, 6.0, color.gamma_multiply(0.22));
        ui.painter().circle_filled(end, 3.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_card_has_no_sparkline() {
        let short = Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 120.0));
        assert!(layout(short).spark.is_none());
        let tall = Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 300.0));
        let layout = layout(tall);
        let band = layout.spark.unwrap();
        assert!(band.top() > tall.center().y);
        assert!(band.bottom() < tall.bottom());
        assert!(layout.text.bottom() <= band.top());
    }

    #[test]
    fn a_wide_short_card_puts_the_history_beside_the_number() {
        let wide = Rect::from_min_size(Pos2::ZERO, Vec2::new(560.0, 200.0));
        let layout = layout(wide);
        let band = layout.spark.unwrap();
        assert!(band.left() > layout.text.right());
        assert!(wide.contains_rect(band) && wide.contains_rect(layout.text));
    }
}
