//! The Journal's Compare page: accounts side by side. An equity overlay that starts every account
//! at zero, and a table of the same figures per account. A view over [`AccountColumn`]s.

use eframe::egui::{self, Color32, Ui, vec2};

use super::journal_charts::{self as charts, Line, LineChart};
use super::journal_kit::{self as jk, Card, Sign};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::theme;
use degenview_journal::calendar::Offset;
use degenview_journal::cells::NONE;
use degenview_journal::compare::AccountColumn;
use degenview_journal::format;
use degenview_journal::metric::Metric;

/// Series colours that are neither the win/loss pair nor blue.
pub const SERIES: [Color32; 6] = [
    Color32::from_rgb(0xF5, 0xA6, 0x23),
    Color32::from_rgb(0xB0, 0x84, 0xF5),
    Color32::from_rgb(0xF0, 0x62, 0x92),
    Color32::from_rgb(0xA5, 0xD6, 0x3F),
    Color32::from_rgb(0xE0, 0x7A, 0x3A),
    Color32::from_rgb(0x9C, 0x9C, 0x9C),
];

fn pct(metric: &Metric<f64>) -> String {
    metric
        .value()
        .map_or_else(|| NONE.to_owned(), |v| format::percent(*v))
}

fn row_values(column: &AccountColumn, currency: &str) -> Vec<String> {
    let s = &column.summary;
    let money = |v: rust_decimal::Decimal| {
        format!("{} {currency}", format::signed_money(v))
            .trim()
            .to_owned()
    };
    vec![
        s.closed.to_string(),
        money(s.net_pnl),
        pct(&s.win_rate),
        s.profit_factor
            .value()
            .map_or_else(|| NONE.to_owned(), |v| format::ratio(*v)),
        s.expectancy
            .value()
            .map_or_else(|| NONE.to_owned(), |v| money(*v)),
        match s.drawdown.max_pct {
            Some(p) => format!("{} ({})", format::money(s.drawdown.max), format::percent(p)),
            None => format::money(s.drawdown.max),
        },
        s.expectancy_r
            .value()
            .map_or_else(|| NONE.to_owned(), |v| format::r_multiple(*v)),
        pct(&s.days.day_win_rate),
        column
            .edge
            .score
            .value()
            .map_or_else(|| NONE.to_owned(), |v| format!("{v:.0}")),
    ]
}

pub const ROWS: [&str; 9] = [
    "Closed trades",
    "Net P&L",
    "Win rate",
    "Profit factor",
    "Expectancy",
    "Max drawdown",
    "Average R",
    "Day win rate",
    "Edge score",
];

pub fn show(ui: &mut Ui, columns: &[AccountColumn], currency: &str, offset: Offset<'_>) {
    let overlay = Card::new("Cumulative net P&L").subtitle(
        "Every account starts at zero, so the lines compare what each earned, not how large it is.",
    );
    jk::card(
        ui,
        &overlay,
        |_| {},
        |ui| {
            let lines: Vec<Line> = columns
                .iter()
                .enumerate()
                .map(|(index, column)| Line {
                    name: column.name.clone(),
                    points: column.equity.iter().map(|(t, v)| (*t as f64, *v)).collect(),
                    color: SERIES[index % SERIES.len()],
                    fill: false,
                    dashed: false,
                })
                .collect();
            let x_label = |x: f64| format::short_date(x as i64, offset);
            let y_label = |y: f64| format::compact(y);
            charts::line_chart(
                ui,
                egui::Id::new("journal-compare"),
                &LineChart {
                    lines: &lines,
                    baseline: Some(0.0),
                    x_label: &x_label,
                    y_label: &y_label,
                    empty: "No closed trades",
                    badge: false,
                },
                220.0,
            );
        },
    );
    ui.add_space(jk::GAP);
    let table = Card::new("Side by side").subtitle(
        "Accounts in other currencies are left out, not converted. Percentages need an account's initial balance.",
    );
    jk::card(
        ui,
        &table,
        |_| {},
        |ui| {
            let label_width = 150.0;
            let width =
                ((ui.available_width() - label_width) / columns.len().max(1) as f32).max(110.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_space(vec2(label_width, 22.0));
                for (index, column) in columns.iter().enumerate() {
                    charts::fixed_cell(ui, vec2(width, 22.0), |ui| {
                        let (dot, _) =
                            ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(dot.center(), 4.0, SERIES[index % SERIES.len()]);
                        let name = if column.archived {
                            format!("{} (archived)", column.name)
                        } else {
                            column.name.clone()
                        };
                        ui.label(kit::text(name, size::BODY, SEMIBOLD, theme::text_primary()));
                    });
                }
            });
            ui.add_space(4.0);
            jk::divider(ui);
            let values: Vec<Vec<String>> =
                columns.iter().map(|c| row_values(c, currency)).collect();
            for (row, label) in ROWS.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    charts::fixed_cell(ui, vec2(label_width, 28.0), |ui| {
                        ui.label(kit::text(
                            *label,
                            size::BODY,
                            REGULAR,
                            theme::text_secondary(),
                        ));
                    });
                    for (column, column_values) in columns.iter().zip(&values) {
                        // Net P&L and expectancy carry their sign's colour.
                        let color = match row {
                            1 => Sign::of_decimal(column.summary.net_pnl).text(),
                            _ => theme::text_primary(),
                        };
                        charts::fixed_cell(ui, vec2(width, 28.0), |ui| {
                            ui.label(kit::text(
                                column_values[row].as_str(),
                                size::BODY,
                                MEDIUM,
                                color,
                            ));
                        });
                    }
                });
                if row + 1 < ROWS.len() {
                    jk::divider(ui);
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_journal::calendar::utc;
    use degenview_journal::compare::{Input, compare};
    use degenview_journal::model::{
        CostBasis, JournalAccount, Side, SourceId, Trade, TradingSourceKind,
    };
    use degenview_journal::reconstruct::reconstruct;
    use degenview_journal::testing::{d, exec};
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    #[test]
    fn the_page_draws_and_has_a_row_of_values_per_row_label() {
        let accounts: Vec<JournalAccount> = ["a", "b"]
            .iter()
            .map(|uid| JournalAccount {
                uid: (*uid).into(),
                source: SourceId("t".into()),
                kind: TradingSourceKind::Paper,
                external_id: "1".into(),
                name: format!("Account {uid}"),
                currency: "USD".into(),
                initial_balance: Some(d("1000")),
                cost_basis: CostBasis::AverageCost,
                archived: false,
                created_at_ms: 1,
            })
            .collect();
        let mut e = Vec::new();
        for (account, id, exit, t) in [
            ("a", "1", "110", 1_000),
            ("a", "2", "95", 3_000),
            ("b", "3", "120", 2_000),
        ] {
            e.push(exec(&format!("{id}x"), Side::Buy, "1", "100", t).in_account(account));
            e.push(exec(&format!("{id}y"), Side::Sell, "1", exit, t + 500).in_account(account));
        }
        let trades = reconstruct(&e, |_| CostBasis::AverageCost);
        let refs: Vec<&Trade> = trades.iter().collect();
        let columns = compare(&Input {
            accounts: &accounts,
            trades: &refs,
            currency: Some("USD".into()),
            reviews: &HashMap::new(),
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        assert_eq!(columns.len(), 2);
        for column in &columns {
            assert_eq!(row_values(column, "USD").len(), ROWS.len());
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, &columns, "USD", &utc)
        });
        out.textures_delta.clear();
    }
}
