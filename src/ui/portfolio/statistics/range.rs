//! The value range strip: where the value stands between its lowest and highest point.

use super::{RANGE_HEIGHT, StatisticsInput};
use crate::model::portfolio::statistics::PortfolioStatistics;
use crate::ui::kit::{self, REGULAR, SEMIBOLD, time_text::date_text};
use crate::ui::portfolio::blocks::{caption_color, card_frame, text_at};
use crate::ui::portfolio::table::tone_option;
use crate::ui::theme;
use eframe::egui::{Align2, Rect, Ui, pos2};

/// Where the value stands between its lowest and highest point: a track filled to today's value.
pub(super) fn range_strip(ui: &mut Ui, input: &StatisticsInput, stats: &PortfolioStatistics) {
    let f = input.formatter;
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    let rect = card_frame(ui, RANGE_HEIGHT, None);
    let (left, right) = (rect.left() + 16.0, rect.right() - 16.0);
    let secondary = caption_color();
    text_at(
        ui,
        pos2(left, rect.top() + 13.0),
        Align2::LEFT_TOP,
        "Value Range",
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );

    let ends = stats
        .high
        .zip(stats.low)
        .filter(|(high, low)| high.value > low.value);
    let position = ends.map(|(high, low)| {
        ((stats.total_value - low.value) / (high.value - low.value)).clamp(0.0, 1.0) as f32
    });
    if let Some(position) = position {
        text_at(
            ui,
            pos2(right, rect.top() + 13.0),
            Align2::RIGHT_TOP,
            &format!("{:.0}% of the way up", position * 100.0),
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
    }

    let y = rect.top() + 44.0;
    let track = Rect::from_min_max(pos2(left, y - 4.0), pos2(right, y + 4.0));
    ui.painter().rect_filled(track, 4.0, theme::ink(10));
    if let Some(position) = position {
        let colour = tone_option(f.tone(Some(stats.total_pnl()))).unwrap_or(theme::SYSTEM_ACCENT);
        // The marker keeps its whole width on the card at either end.
        let x = (track.left() + track.width() * position)
            .clamp(track.left() + 8.0, track.right() - 8.0);
        ui.painter().rect_filled(
            Rect::from_min_max(track.min, pos2(x.max(track.left() + 8.0), track.bottom())),
            4.0,
            colour.gamma_multiply(0.85),
        );
        ui.painter().circle_filled(pos2(x, y), 8.0, theme::chrome());
        ui.painter()
            .circle_filled(pos2(x, y), 5.5, theme::text_primary());
    }

    let with_date = rect.width() >= 480.0;
    for (extreme, label, anchor, align) in [
        (stats.low, "Low", left, Align2::LEFT_TOP),
        (stats.high, "High", right, Align2::RIGHT_TOP),
    ] {
        let money = extreme.map_or("—".to_owned(), |e| f.money(e.value));
        let note = match extreme {
            Some(e) if with_date => format!("{label} \u{00B7} {}", on(e.timestamp)),
            _ => label.to_owned(),
        };
        let top = rect.top() + 58.0;
        let figure = text_at(
            ui,
            pos2(anchor, top),
            align,
            &money,
            13.0,
            SEMIBOLD,
            if extreme.is_some() {
                theme::text_primary()
            } else {
                secondary
            },
        );
        text_at(
            ui,
            pos2(anchor, figure.bottom() + 1.0),
            align,
            &note,
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
    }
}
