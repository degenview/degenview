//! The Journal's P&L calendar: one tile per day of a month, tinted by the day's result, with the
//! net P&L, the number of trades and the win rate, a week total at the end of each row, and the
//! month's totals underneath. A view over a [`MonthView`]; it returns what the user did.
//!
//! A day is never shown by colour alone: the sign is in the figure, and an arrow marks gains and
//! losses.

use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use rust_decimal::Decimal;

use super::header_controls;
use super::icons::{self, Icon};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::theme;
use degenview_journal::calendar::{Civil, DayCell, MonthView, WeekTotal};
use degenview_journal::format;

const GAP: f32 = 6.0;
const TILE_HEIGHT: f32 = 78.0;

#[derive(Default)]
pub struct Output {
    /// -1 for the previous month, 1 for the next.
    pub shift: Option<i32>,
    pub today: bool,
    /// A day with trades was clicked.
    pub open_day: Option<Civil>,
}

fn tint_of(net: Decimal) -> egui::Color32 {
    if net > Decimal::ZERO {
        theme::WIN
    } else if net < Decimal::ZERO {
        theme::LOSS
    } else {
        theme::text_secondary()
    }
}

pub fn show(
    ui: &mut Ui,
    view: &MonthView,
    week_start: u8,
    currency: &str,
    today: Option<Civil>,
) -> Output {
    let mut output = Output::default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            format::month_title(view.year, view.month),
            18.0,
            SEMIBOLD,
            theme::text_primary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if header_controls::icon_button(ui, Icon::ChevronRight, "Next month", false).clicked() {
                output.shift = Some(1);
            }
            if header_controls::icon_button(ui, Icon::ChevronLeft, "Previous month", false)
                .clicked()
            {
                output.shift = Some(-1);
            }
            if kit::sheet_button(ui, "Today", false, true).clicked() {
                output.today = true;
            }
        });
    });
    ui.add_space(12.0);

    let week_width = 96.0_f32.min(ui.available_width() / 6.0);
    let day_width = (ui.available_width() - week_width - GAP * 7.0) / 7.0;
    let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for column in 0..7 {
            let name = names[(usize::from(week_start % 7) + column) % 7];
            let (rect, _) = ui.allocate_exact_size(vec2(day_width, 18.0), Sense::hover());
            ui.painter().text(
                rect.left_center() + vec2(6.0, 0.0),
                Align2::LEFT_CENTER,
                name.to_uppercase(),
                FontId::proportional(size::CAPTION),
                theme::text_tertiary(),
            );
        }
        let (rect, _) = ui.allocate_exact_size(vec2(week_width, 18.0), Sense::hover());
        ui.painter().text(
            rect.left_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            "WEEK",
            FontId::proportional(size::CAPTION),
            theme::text_tertiary(),
        );
    });
    ui.add_space(4.0);

    let grid = view.grid(week_start);
    let weeks = view.week_totals(week_start);
    for (row, week) in grid.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for slot in week {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(day_width, TILE_HEIGHT), Sense::click());
                let Some(day) = slot else { continue };
                let cell = view.days.get(day);
                let civil = Civil {
                    year: view.year,
                    month: view.month,
                    day: *day,
                };
                if tile(
                    ui,
                    rect,
                    &response,
                    *day,
                    cell,
                    currency,
                    today == Some(civil),
                ) {
                    output.open_day = Some(civil);
                }
            }
            let (rect, _) = ui.allocate_exact_size(vec2(week_width, TILE_HEIGHT), Sense::hover());
            week_total(ui, rect, weeks.get(row), currency);
        });
        ui.add_space(GAP);
    }

    ui.add_space(6.0);
    footer(ui, view, currency);
    output
}

fn tile(
    ui: &mut Ui,
    rect: Rect,
    response: &egui::Response,
    day: u8,
    cell: Option<&DayCell>,
    currency: &str,
    is_today: bool,
) -> bool {
    let painter = ui.painter();
    let (fill, border) = match cell {
        Some(cell) => {
            let tint = tint_of(cell.net);
            (theme::with_alpha(tint, 0.10), theme::with_alpha(tint, 0.55))
        }
        None => (
            theme::quaternary(0.12),
            theme::with_alpha(theme::separator(), 0.7),
        ),
    };
    painter.rect_filled(rect, 8.0, fill);
    let border = if is_today {
        theme::text_primary()
    } else {
        border
    };
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(if is_today { 1.5 } else { 1.0 }, border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.left_top() + vec2(8.0, 7.0),
        Align2::LEFT_TOP,
        day.to_string(),
        FontId::proportional(size::CAPTION + 1.0),
        if cell.is_some() {
            theme::text_secondary()
        } else {
            theme::text_tertiary()
        },
    );
    let Some(cell) = cell else { return false };
    let tint = tint_of(cell.net);
    let net = format::signed_money(cell.net);
    let center = rect.center() + vec2(0.0, 4.0);
    let galley = kit::galley(
        ui,
        &net,
        size::BODY + 1.0,
        SEMIBOLD,
        tint,
        rect.width() - 28.0,
    );
    let text_pos = pos2(
        center.x - galley.size().x / 2.0 + 6.0,
        center.y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, tint);
    if cell.net != Decimal::ZERO {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(text_pos.x - 8.0, center.y), vec2(9.0, 9.0)),
            if cell.net > Decimal::ZERO {
                Icon::ArrowUp
            } else {
                Icon::ArrowDown
            },
            tint,
        );
    }
    let rate = cell.win_rate().map_or(String::new(), |rate| {
        format!(" \u{00b7} {:.0}%", rate * 100.0)
    });
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 11.0),
        Align2::CENTER_CENTER,
        format!("{}t{rate}", cell.trades),
        FontId::proportional(size::CAPTION),
        theme::text_secondary(),
    );
    let response = response
        .clone()
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    response.on_hover_ui(|ui| {
        ui.label(kit::text(
            format::date(cell.date),
            size::BODY,
            SEMIBOLD,
            theme::text_primary(),
        ));
        ui.label(kit::text(
            format!("{} {currency}", format::signed_money(cell.net))
                .trim()
                .to_owned(),
            size::BODY,
            MEDIUM,
            tint,
        ));
        ui.label(kit::text(
            format!(
                "{} trade{} \u{00b7} {} W / {} BE / {} L",
                cell.trades,
                if cell.trades == 1 { "" } else { "s" },
                cell.wins,
                cell.breakevens,
                cell.losses
            ),
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_secondary(),
        ));
        ui.label(kit::text(
            "Click to see the day's trades",
            size::CAPTION,
            REGULAR,
            theme::text_tertiary(),
        ));
    });
    clicked
}

fn week_total(ui: &mut Ui, rect: Rect, week: Option<&WeekTotal>, currency: &str) {
    let painter = ui.painter();
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.5)),
        egui::StrokeKind::Inside,
    );
    let Some(week) = week.filter(|week| week.trading_days > 0) else {
        return;
    };
    let tint = tint_of(week.net);
    painter.text(
        pos2(rect.center().x, rect.center().y - 6.0),
        Align2::CENTER_CENTER,
        format::signed_money(week.net),
        FontId::proportional(size::BODY),
        tint,
    );
    painter.text(
        pos2(rect.center().x, rect.center().y + 12.0),
        Align2::CENTER_CENTER,
        format!("{}t \u{00b7} {}d", week.trades, week.trading_days),
        FontId::proportional(size::CAPTION),
        theme::text_secondary(),
    );
    let _ = currency;
}

fn footer(ui: &mut Ui, view: &MonthView, currency: &str) {
    let money = |value: Decimal| {
        format!("{} {currency}", format::signed_money(value))
            .trim()
            .to_owned()
    };
    let day = |cell: &Option<DayCell>| match cell {
        Some(cell) => format!(
            "{} \u{00b7} {}",
            format::short_civil(cell.date),
            money(cell.net)
        ),
        None => "\u{2014}".to_owned(),
    };
    let stats = [
        ("Month net P&L", money(view.net), tint_of(view.net)),
        ("Best day", day(&view.best_day), theme::text_primary()),
        ("Worst day", day(&view.worst_day), theme::text_primary()),
        (
            "Activity",
            format!(
                "{} trade{} \u{00b7} {}/{} green",
                view.trades,
                if view.trades == 1 { "" } else { "s" },
                view.green_days,
                view.trading_days
            ),
            theme::text_primary(),
        ),
    ];
    kit::card_frame(false).show(ui, |ui| {
        ui.columns(stats.len(), |columns| {
            for (column, (label, value, color)) in columns.iter_mut().zip(stats) {
                column.label(kit::text(
                    label.to_uppercase(),
                    size::CAPTION,
                    SEMIBOLD,
                    theme::text_tertiary(),
                ));
                column.label(kit::text(value, size::BODY + 1.0, MEDIUM, color));
            }
        });
    });
}

/// The civil date of today, for the "today" outline and the Today button.
pub fn civil_today(now_ms: i64, offset: &dyn Fn(i64) -> i64) -> Civil {
    degenview_journal::calendar::civil_from_days(degenview_journal::calendar::day_number(
        now_ms, offset,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_journal::calendar::{Outcome, days_from_civil, month_view, utc};
    use degenview_journal::model::{CostBasis, Side};
    use degenview_journal::reconstruct::reconstruct;
    use degenview_journal::testing::exec;

    #[test]
    fn the_month_draws_with_and_without_trades() {
        let day = |n: i64| {
            (days_from_civil(Civil {
                year: 2026,
                month: 8,
                day: n as u8,
            }) * 86_400
                + 3_600)
                * 1_000
        };
        let e = vec![
            exec("a", Side::Buy, "1", "100", day(3)),
            exec("b", Side::Sell, "1", "90", day(3) + 1_000),
            exec("c", Side::Buy, "1", "100", day(5)),
            exec("d", Side::Sell, "1", "150", day(5) + 1_000),
        ];
        let trades = reconstruct(&e, |_| CostBasis::AverageCost);
        let refs: Vec<_> = trades.iter().collect();
        let outcome = |t: &degenview_journal::model::Trade| {
            if t.net_pnl() > Decimal::ZERO {
                Outcome::Win
            } else {
                Outcome::Loss
            }
        };
        let ctx = egui::Context::default();
        for (view, today) in [
            (
                month_view(&refs, 2026, 8, &utc, &outcome),
                Some(Civil {
                    year: 2026,
                    month: 8,
                    day: 5,
                }),
            ),
            (month_view(&[], 2026, 2, &utc, &outcome), None),
        ] {
            for width in [1300.0, 640.0] {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 900.0))),
                        ..Default::default()
                    },
                    |ui| {
                        let result = show(ui, &view, 0, "USD", today);
                        assert!(
                            result.shift.is_none() && !result.today && result.open_day.is_none()
                        );
                    },
                );
                out.textures_delta.clear();
            }
        }
    }

    #[test]
    fn today_and_weekday_helpers() {
        let civil = civil_today(0, &utc);
        assert_eq!(
            civil,
            Civil {
                year: 1970,
                month: 1,
                day: 1
            }
        );
    }
}
