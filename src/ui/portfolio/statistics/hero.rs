//! The Statistics hero: the total P&L, how it splits between realized and unrealized, and the
//! last day's move.

use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::PortfolioStatistics;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio::blocks::{
    HERO_HEIGHT, HERO_PADDING, caption_color, change_block, glow, lift, text_at, tone_or,
};
use crate::ui::portfolio::table::{tone_color, tone_option};
use crate::ui::theme;
use eframe::egui::{Align2, Color32, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

/// How the profit divides between what is banked and what is still open, as shares of the two
/// magnitudes. `None` when there is neither.
pub fn composition_shares(unrealized: f64, realized: f64) -> Option<[f32; 2]> {
    let total = unrealized.abs() + realized.abs();
    (total.is_finite() && total > 0.0).then(|| {
        [
            (unrealized.abs() / total) as f32,
            (realized.abs() / total) as f32,
        ]
    })
}

/// Shares of the priced assets that are in profit, at a loss and level, summing to one. All
/// zero when nothing is priced.
pub fn win_shares(winners: usize, losers: usize, priced: usize) -> [f32; 3] {
    if priced == 0 {
        return [0.0; 3];
    }
    let share = |count: usize| count as f32 / priced as f32;
    let level = priced.saturating_sub(winners + losers);
    [share(winners), share(losers), share(level)]
}

/// Total P&L large, how it splits between realized and unrealized in the middle, and the last
/// day's move at the edge.
pub(super) fn statistics_hero(
    ui: &mut Ui,
    f: PortfolioValueFormatter,
    stats: &PortfolioStatistics,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_HEIGHT), Sense::hover());
    let total = stats.total_pnl();
    let tone = f.tone(Some(total));
    let tint = tone.map_or(theme::SYSTEM_ACCENT, tone_color);
    lift(ui, rect, 16.0);
    if stats.asset_count > 0 {
        glow(ui, rect, tint);
    }
    let inner = rect.shrink(HERO_PADDING);
    let secondary = caption_color();

    let chip = Rect::from_center_size(
        pos2(inner.left() + 11.0, inner.top() + 11.0),
        Vec2::splat(22.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, theme::SYSTEM_ACCENT.gamma_multiply(0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chip.center(), Vec2::splat(13.0)),
        Icon::ChartUptrend,
        theme::SYSTEM_ACCENT,
    );
    let name = text_at(
        ui,
        pos2(chip.right() + 9.0, chip.center().y),
        Align2::LEFT_CENTER,
        "Total P&L",
        15.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let all_time = text_at(
        ui,
        pos2(name.right() + 6.0, chip.center().y),
        Align2::LEFT_CENTER,
        "\u{00B7} All time",
        15.0,
        REGULAR,
        secondary,
    );
    let balance = text_at(
        ui,
        pos2(inner.left(), inner.top() + 32.0),
        Align2::LEFT_TOP,
        &f.signed_money(total),
        40.0,
        SEMIBOLD,
        tone_or(tone, theme::text_primary()),
    );

    // The return on what was put in, in the figure's own colour.
    let caption_middle = inner.bottom() - 6.0;
    let mut left_end = all_time.right().max(balance.right());
    if stats.cost_basis > 0.0 {
        let percent = total / stats.cost_basis;
        let figure = text_at(
            ui,
            pos2(inner.left(), caption_middle),
            Align2::LEFT_CENTER,
            &f.signed_percent(percent, 2),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(f.tone(Some(percent)), secondary),
        );
        let rest = text_at(
            ui,
            pos2(figure.right() + 5.0, caption_middle),
            Align2::LEFT_CENTER,
            "on cost basis",
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
        left_end = left_end.max(rest.right());
    } else {
        let none = text_at(
            ui,
            pos2(inner.left(), caption_middle),
            Align2::LEFT_CENTER,
            "No cost basis yet",
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
        left_end = left_end.max(none.right());
    }

    let middle = rect.center().y;
    let edge = change_block(
        ui,
        f,
        "24h",
        stats.day_change.map(|d| d.amount),
        stats.day_change.and_then(|d| d.percentage),
        pos2(inner.right(), middle),
    );
    ui.painter().vline(
        edge - 18.0,
        middle - 26.0..=middle + 26.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );
    let room = Rect::from_min_max(
        pos2(left_end + 36.0, inner.top() + 4.0),
        pos2(edge - 36.0, inner.bottom() - 4.0),
    );
    if room.width() >= 220.0 {
        composition(ui, f, room, stats);
    }
}

/// A bar of realized against unrealized profit, with a line each beneath it.
fn composition(ui: &Ui, f: PortfolioValueFormatter, room: Rect, stats: &PortfolioStatistics) {
    let secondary = caption_color();
    let room = Rect::from_min_size(room.min, vec2(room.width().min(420.0), room.height()));
    let top = room.center().y - 37.0;
    text_at(
        ui,
        pos2(room.left(), top),
        Align2::LEFT_TOP,
        "Breakdown",
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let open = (stats.unrealized_pnl, 1.0);
    let banked = (stats.realized_pnl, 0.55);
    let colour = |(amount, strength): (f64, f32)| {
        tone_option(f.tone(Some(amount))).map_or(theme::ink(80), |c| c.gamma_multiply(strength))
    };

    let bar = Rect::from_min_size(pos2(room.left(), top + 22.0), vec2(room.width(), 10.0));
    ui.painter().rect_filled(bar, 5.0, theme::ink(10));
    if let Some([open_share, banked_share]) =
        composition_shares(stats.unrealized_pnl, stats.realized_pnl)
    {
        let gap = if open_share > 0.0 && banked_share > 0.0 {
            3.0
        } else {
            0.0
        };
        let usable = bar.width() - gap;
        let open_width = usable * open_share;
        let paint = |left: f32, width: f32, colour: Color32| {
            if width > 0.0 {
                ui.painter().rect_filled(
                    Rect::from_min_size(pos2(left, bar.top()), vec2(width, bar.height())),
                    5.0,
                    colour,
                );
            }
        };
        paint(bar.left(), open_width, colour(open));
        paint(
            bar.left() + open_width + gap,
            usable * banked_share,
            colour(banked),
        );
    }

    for (index, (label, figure)) in [("Unrealized", open), ("Realized", banked)]
        .into_iter()
        .enumerate()
    {
        let y = top + 46.0 + index as f32 * 20.0;
        ui.painter()
            .circle_filled(pos2(room.left() + 4.0, y), 3.5, colour(figure));
        text_at(
            ui,
            pos2(room.left() + 14.0, y),
            Align2::LEFT_CENTER,
            label,
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
        text_at(
            ui,
            pos2(room.right(), y),
            Align2::RIGHT_CENTER,
            &f.signed_money(figure.0),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(f.tone(Some(figure.0)), theme::text_primary()),
        );
    }
}
