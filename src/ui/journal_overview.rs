//! The Journal's Overview page: headline cards, the realised-equity curve, the Edge Score, daily
//! P&L, drawdown, the distribution of results, long against short and the latest trades. A view
//! over a precomputed [`Overview`]; it returns what the user clicked.

use eframe::egui::{self, Color32, Sense, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::icons::Icon;
use super::journal_charts::{self as charts, Bar, Line, LineChart};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use crate::model::journal::calendar::Offset;
use crate::model::journal::cells::NONE;
use crate::model::journal::edge::{ComponentKey, MIN_TRADES};
use crate::model::journal::format;
use crate::model::journal::metric::Metric;
use crate::model::journal::model::{Direction, Trade};
use crate::model::journal::overview::{Overview, SideStats};

/// Card chrome and chart sizes.
const CARD_GAP: f32 = 12.0;
const KPI_HEIGHT: f32 = 118.0;
const CHART_HEIGHT: f32 = 200.0;
/// Below this width the page stacks its cards.
const WIDE: f32 = 980.0;

#[derive(Default)]
pub struct OverviewState {
    pub show_previous: bool,
    /// Drawdown as a percentage of the balance instead of an amount.
    pub drawdown_percent: bool,
}

#[derive(Default)]
pub struct OverviewOutput {
    /// A currency picked from the selector.
    pub currency: Option<Option<String>>,
    /// A recent trade clicked.
    pub open_trade: Option<String>,
}

/// Chips to choose which currency the page describes, when the selection has more than one.
pub fn currency_chips(
    ui: &mut Ui,
    currencies: &[Option<String>],
    current: &Option<String>,
) -> Option<Option<String>> {
    if currencies.len() < 2 {
        return None;
    }
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label(kit::text(
            "Amounts are never added across currencies. Showing",
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
        for candidate in currencies {
            let label = candidate
                .clone()
                .unwrap_or_else(|| "no currency".to_owned());
            if kit::value_chip(ui, &label, candidate == current).clicked() {
                chosen = Some(candidate.clone());
            }
        }
    });
    ui.add_space(10.0);
    chosen
}

fn tone(value: f64) -> Color32 {
    if value > 0.0 {
        theme::WIN
    } else if value < 0.0 {
        theme::LOSS
    } else {
        theme::text_primary()
    }
}

fn decimal_tone(value: Decimal) -> Color32 {
    tone(value.to_f64().unwrap_or(0.0))
}

/// A metric's text, or a dash with the reason as its tooltip.
fn metric_text<T>(
    metric: &Metric<T>,
    format: impl FnOnce(&T) -> String,
) -> (String, Option<String>) {
    match metric {
        Metric::Value(value) => (format(value), None),
        Metric::Unavailable(reason) => (NONE.to_owned(), Some(reason.explain())),
    }
}

/// Cards across `ui` with the given relative widths, one row.
fn weighted_row(ui: &mut Ui, weights: &[f32], mut content: impl FnMut(&mut Ui, usize)) {
    let total_gap = CARD_GAP * (weights.len() as f32 - 1.0);
    let sum: f32 = weights.iter().sum();
    let available = ui.available_width() - total_gap;
    let top = ui.cursor().min;
    let mut left = top.x;
    let mut bottom = top.y;
    for (index, weight) in weights.iter().enumerate() {
        let width = available * weight / sum;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
            egui::pos2(left, top.y),
            egui::pos2(left + width, f32::INFINITY),
        )));
        child.set_width(width);
        content(&mut child, index);
        bottom = bottom.max(child.min_rect().bottom());
        left += width + CARD_GAP;
    }
    ui.allocate_rect(
        egui::Rect::from_min_max(top, egui::pos2(top.x + available + total_gap, bottom)),
        Sense::hover(),
    );
    ui.add_space(CARD_GAP);
}

fn card<R>(
    ui: &mut Ui,
    height: Option<f32>,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let width = ui.available_width();
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 28.0);
        if let Some(height) = height {
            ui.set_min_height(height - 24.0);
        }
        content(ui)
    })
}

fn title_row(ui: &mut Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(kit::text(
                title,
                size::BODY + 1.0,
                SEMIBOLD,
                theme::text_primary(),
            ));
            ui.label(kit::text(
                subtitle,
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), right);
    });
    ui.add_space(8.0);
}

/// A small text toggle ("vs prev", "%").
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) {
    if kit::value_chip(ui, label, *on).clicked() {
        *on = !*on;
    }
}

pub fn show(
    ui: &mut Ui,
    overview: &Overview,
    state: &mut OverviewState,
    offset: Offset<'_>,
    trades: &[Trade],
) -> OverviewOutput {
    let mut output = OverviewOutput::default();
    let currency = overview.currency.as_deref().unwrap_or("");
    let wide = ui.available_width() >= WIDE;

    output.currency = currency_chips(ui, &overview.currencies, &overview.currency);

    // Headline cards.
    let weights: Vec<f32> = if wide { vec![1.0; 5] } else { vec![1.0, 1.0] };
    let cards: [&dyn Fn(&mut Ui); 5] = [
        &|ui| net_card(ui, overview, currency),
        &|ui| win_rate_card(ui, overview),
        &|ui| profit_factor_card(ui, overview),
        &|ui| avg_card(ui, overview, currency),
        &|ui| day_win_card(ui, overview),
    ];
    if wide {
        weighted_row(ui, &weights, |ui, index| {
            card(ui, Some(KPI_HEIGHT), |ui| cards[index](ui));
        });
    } else {
        for pair in cards.chunks(2) {
            weighted_row(ui, &weights[..pair.len()], |ui, index| {
                card(ui, Some(KPI_HEIGHT), |ui| pair[index](ui));
            });
        }
    }

    stat_tiles(ui, overview, currency, offset, wide);

    // Equity and Edge Score.
    let equity_weights: &[f32] = if wide { &[2.0, 1.0] } else { &[1.0] };
    weighted_row(ui, equity_weights, |ui, index| {
        card(ui, Some(CHART_HEIGHT + 70.0), |ui| match index {
            0 => equity_card(ui, overview, state, offset, currency),
            _ => edge_card(ui, overview),
        });
    });

    // Daily P&L and drawdown.
    let pair_weights: &[f32] = if wide { &[1.0, 1.0] } else { &[1.0] };
    weighted_row(ui, pair_weights, |ui, index| {
        card(ui, Some(CHART_HEIGHT + 70.0), |ui| match index {
            0 => daily_card(ui, overview, currency),
            _ => drawdown_card(ui, overview, state, offset, currency),
        });
    });

    // Distribution, long against short, recent.
    let triple: &[f32] = if wide { &[1.0, 1.0, 1.0] } else { &[1.0] };
    weighted_row(ui, triple, |ui, index| {
        card(ui, Some(CHART_HEIGHT + 70.0), |ui| match index {
            0 => distribution_card(ui, overview, currency),
            1 => sides_card(ui, overview, currency),
            _ => recent_card(ui, overview, trades, offset, &mut output),
        });
    });
    output
}

// MARK: Cards

fn caption(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text.to_uppercase(),
        size::CAPTION,
        SEMIBOLD,
        theme::text_tertiary(),
    ));
}

fn big(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    ui.label(kit::text(text, 24.0, SEMIBOLD, color))
}

fn note(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.label(kit::text(text, size::CAPTION + 1.0, REGULAR, color));
}

fn delta_note(ui: &mut Ui, current: Option<f64>, previous: Option<f64>, as_points: bool) {
    let (Some(current), Some(previous)) = (current, previous) else {
        return;
    };
    let text = if as_points {
        format!("{:+.1} pts vs prev", (current - previous) * 100.0)
    } else if previous.abs() > f64::EPSILON {
        format!(
            "{:+.1}% vs prev",
            (current - previous) / previous.abs() * 100.0
        )
    } else {
        return;
    };
    let color = if current > previous {
        theme::WIN
    } else if current < previous {
        theme::LOSS
    } else {
        theme::text_tertiary()
    };
    note(ui, text, color);
}

fn net_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    let s = &overview.summary;
    caption(ui, "Net P&L");
    let text = format!("{} {currency}", format::signed_money(s.net_pnl))
        .trim()
        .to_owned();
    big(ui, &text, decimal_tone(s.net_pnl)).on_hover_text(
        "Net P&L: gross P&L of closed trades minus fees. Open trades are not counted.",
    );
    note(
        ui,
        format!("{} closed \u{00b7} {} open", s.closed, s.open),
        theme::text_secondary(),
    );
    delta_note(
        ui,
        s.net_pnl.to_f64(),
        overview.previous.as_ref().and_then(|p| p.net_pnl.to_f64()),
        false,
    );
    if !s.fees_complete {
        note(ui, "Some fees unknown: a floor", theme::text_tertiary());
    }
}

fn win_rate_card(ui: &mut Ui, overview: &Overview) {
    let s = &overview.summary;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            caption(ui, "Win rate");
            let (text, reason) = metric_text(&s.win_rate, |v| format::percent(*v));
            let r = big(ui, &text, theme::text_primary());
            r.on_hover_text(reason.unwrap_or_else(|| {
                "Winning trades divided by closed trades (breakevens count in the total)."
                    .to_owned()
            }));
            note(
                ui,
                format!(
                    "{} W \u{00b7} {} BE \u{00b7} {} L",
                    s.wins, s.breakevens, s.losses
                ),
                theme::text_secondary(),
            );
            delta_note(
                ui,
                s.win_rate.copied(),
                overview.previous.as_ref().and_then(|p| p.win_rate.copied()),
                true,
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            charts::ring(ui, s.win_rate.copied(), theme::WIN, 44.0);
        });
    });
}

fn profit_factor_card(ui: &mut Ui, overview: &Overview) {
    let s = &overview.summary;
    caption(ui, "Profit factor");
    let (text, reason) = metric_text(&s.profit_factor, |v| format::ratio(*v));
    big(ui, &text, theme::text_primary())
        .on_hover_text(reason.unwrap_or_else(|| "Gross profit divided by gross loss.".to_owned()));
    note(
        ui,
        format!(
            "{} / {}",
            format::money(s.gross_profit),
            format::money(s.gross_loss)
        ),
        theme::text_secondary(),
    );
    delta_note(
        ui,
        s.profit_factor.copied(),
        overview
            .previous
            .as_ref()
            .and_then(|p| p.profit_factor.copied()),
        false,
    );
}

fn avg_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    let s = &overview.summary;
    caption(ui, "Average win / loss");
    let (text, reason) = metric_text(&s.payoff, |v| format::ratio(*v));
    big(ui, &text, theme::text_primary())
        .on_hover_text(reason.unwrap_or_else(|| {
            "Average winning trade divided by average losing trade.".to_owned()
        }));
    let win = s
        .average_win
        .value()
        .and_then(|v| v.to_f64())
        .unwrap_or(0.0);
    let loss = s
        .average_loss
        .value()
        .and_then(|v| v.to_f64())
        .unwrap_or(0.0);
    charts::split_bar(ui, win, loss, ui.available_width().min(160.0));
    let part = |m: &Metric<Decimal>, signed: bool| match m {
        Metric::Value(v) => {
            if signed {
                format::signed_money(*v)
            } else {
                format::money(*v)
            }
        }
        Metric::Unavailable(_) => NONE.to_owned(),
    };
    let _ = currency;
    note(
        ui,
        format!(
            "{} / -{}",
            part(&s.average_win, true),
            part(&s.average_loss, false)
        ),
        theme::text_secondary(),
    );
}

fn day_win_card(ui: &mut Ui, overview: &Overview) {
    let d = &overview.summary.days;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            caption(ui, "Day win rate");
            let (text, reason) = metric_text(&d.day_win_rate, |v| format::percent(*v));
            big(ui, &text, theme::text_primary()).on_hover_text(reason.unwrap_or_else(|| {
                "Winning days divided by trading days (a trading day has a closed trade)."
                    .to_owned()
            }));
            note(
                ui,
                format!("{} trading days", d.trading_days),
                theme::text_secondary(),
            );
            delta_note(
                ui,
                d.day_win_rate.copied(),
                overview
                    .previous
                    .as_ref()
                    .and_then(|p| p.days.day_win_rate.copied()),
                true,
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            charts::ring(ui, d.day_win_rate.copied(), theme::WIN, 44.0);
        });
    });
}

fn stat_tiles(ui: &mut Ui, overview: &Overview, currency: &str, offset: Offset<'_>, wide: bool) {
    let s = &overview.summary;
    let money = |value: Decimal| {
        format!("{} {currency}", format::signed_money(value))
            .trim()
            .to_owned()
    };
    let metric_money = |m: &Metric<Decimal>| match m {
        Metric::Value(v) => money(*v),
        Metric::Unavailable(_) => NONE.to_owned(),
    };
    let mut tiles: Vec<(&str, String, &str)> = vec![
        (
            "Gross P&L",
            money(s.gross_pnl),
            "Realised P&L of closed trades before fees.",
        ),
        (
            "Fees",
            if s.fees_complete {
                format::money(s.fees)
            } else {
                format!("\u{2265} {}", format::money(s.fees))
            },
            "Fees charged on closed trades, with the entry fees of what has closed.",
        ),
        (
            "Total trades",
            format!("{}", s.closed + s.open),
            "Closed and open trades in the selection.",
        ),
        (
            "Expectancy",
            metric_money(&s.expectancy),
            "Net P&L per closed trade.",
        ),
        (
            "Average R",
            match &s.expectancy_r {
                Metric::Value(r) => format!("{} ({} trades)", format::r_multiple(*r), s.r.count),
                Metric::Unavailable(_) => NONE.to_owned(),
            },
            "Mean realised R over trades that have a stop. Needs a stop on the trade or in its review.",
        ),
        (
            "Max drawdown",
            match s.drawdown.max_pct {
                Some(pct) => format!(
                    "{} ({})",
                    format::money(s.drawdown.max),
                    format::percent(pct)
                ),
                None => format::money(s.drawdown.max),
            },
            "Deepest fall of realised equity below its running peak. The percentage needs an initial balance.",
        ),
        (
            "Trading days",
            format!("{}", s.days.trading_days),
            "Days with at least one closed trade, in your time zone.",
        ),
        (
            "Winning days",
            format!("{}", s.days.winning),
            "Days whose net P&L is positive.",
        ),
        (
            "Best day",
            match &s.days.best {
                Metric::Value(day) => format!(
                    "{} \u{00b7} {}",
                    money(day.net),
                    format::short_civil(day.date)
                ),
                Metric::Unavailable(_) => NONE.to_owned(),
            },
            "The trading day with the highest net P&L.",
        ),
        (
            "Worst day",
            match &s.days.worst {
                Metric::Value(day) => format!(
                    "{} \u{00b7} {}",
                    money(day.net),
                    format::short_civil(day.date)
                ),
                Metric::Unavailable(_) => NONE.to_owned(),
            },
            "The trading day with the lowest net P&L.",
        ),
        (
            "Avg duration",
            match &s.durations.average_ms {
                Metric::Value(ms) => format::duration(*ms),
                Metric::Unavailable(_) => NONE.to_owned(),
            },
            "Mean time from open to close.",
        ),
        (
            "Streak",
            match s.streaks.current {
                0 => NONE.to_owned(),
                1 => "1 win".to_owned(),
                n if n > 0 => format!("{n} wins"),
                -1 => "1 loss".to_owned(),
                n => format!("{} losses", -n),
            },
            "The winning or losing streak in progress, in close order. A breakeven ends it.",
        ),
    ];
    let _ = offset;
    let per_row = if wide { 6 } else { 2 };
    for chunk in tiles.chunks_mut(per_row) {
        let weights = vec![1.0; chunk.len().max(per_row)];
        weighted_row(ui, &weights[..per_row.min(weights.len())], |ui, index| {
            if let Some((label, value, tip)) = chunk.get(index) {
                let frame = egui::Frame::new()
                    .fill(theme::quaternary(0.3))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(12, 9));
                let width = ui.available_width();
                let response = frame
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        caption(ui, label);
                        ui.label(kit::text(
                            value.as_str(),
                            size::BODY + 1.0,
                            MEDIUM,
                            theme::text_primary(),
                        ));
                    })
                    .response;
                response.on_hover_text(*tip);
            }
        });
    }
}

fn equity_card(
    ui: &mut Ui,
    overview: &Overview,
    state: &mut OverviewState,
    offset: Offset<'_>,
    currency: &str,
) {
    let has_balance = overview.initial_balance.is_some();
    let title = if has_balance {
        "Realised equity"
    } else {
        "Cumulative net P&L"
    };
    let subtitle = format!(
        "{} closed trades \u{00b7} {} fees{}",
        overview.summary.closed,
        format::money(overview.summary.fees),
        if has_balance {
            ""
        } else {
            " \u{00b7} no initial balance, so not account equity"
        }
    );
    let has_previous = !overview.previous_equity.is_empty();
    title_row(ui, title, &subtitle, |ui| {
        if has_previous {
            toggle(ui, &mut state.show_previous, "vs prev");
        }
    });
    let value = |point: &crate::model::journal::equity::EquityPoint| {
        point
            .equity
            .unwrap_or(point.cum_net)
            .to_f64()
            .unwrap_or(0.0)
    };
    let series = |points: &[crate::model::journal::equity::EquityPoint]| -> Vec<(f64, f64)> {
        points.iter().map(|p| (p.ts_ms as f64, value(p))).collect()
    };
    let last = overview.equity.last().map(value);
    let baseline = overview
        .initial_balance
        .and_then(|b| b.to_f64())
        .or(Some(0.0));
    let color = match (last, baseline) {
        (Some(last), Some(base)) if last < base => theme::LOSS,
        _ => theme::WIN,
    };
    let mut lines = vec![Line {
        name: "Now",
        points: series(&overview.equity),
        color,
        fill: true,
        dashed: false,
    }];
    if state.show_previous && has_previous {
        lines.push(Line {
            name: "Previous",
            points: series(&overview.previous_equity),
            color: theme::text_tertiary(),
            fill: false,
            dashed: true,
        });
    }
    let x_label = |x: f64| format::short_date(x as i64, offset);
    let y_label = |y: f64| format::compact(y);
    charts::line_chart(
        ui,
        egui::Id::new("journal-equity"),
        &LineChart {
            lines: &lines,
            baseline,
            x_label: &x_label,
            y_label: &y_label,
            empty: "No closed trades in this range",
        },
        CHART_HEIGHT,
    );
    let _ = currency;
}

fn edge_card(ui: &mut Ui, overview: &Overview) {
    let edge = &overview.edge;
    title_row(ui, "Edge score", "Six dimensions of repeatability", |ui| {
        let (text, color) = match &edge.score {
            Metric::Value(score) => (format!("{score:.0} /100"), theme::text_primary()),
            Metric::Unavailable(_) => {
                (format!("needs {MIN_TRADES} trades"), theme::text_tertiary())
            }
        };
        ui.label(kit::text(text, size::BODY, SEMIBOLD, color)).on_hover_text(format!(
            "Edge Score v{} (LuxAlgo's published formula). Measures the record, not the future; \
             {} closed trades.",
            edge.version, edge.closed_trades
        ));
    });
    let axes: Vec<(&str, f64)> = edge
        .components
        .iter()
        .map(|c| (short_axis(c.key), c.value))
        .collect();
    charts::radar(ui, &axes, CHART_HEIGHT - 20.0, theme::text_secondary());
    ui.add_space(4.0);
    note(
        ui,
        "Hover a point for its score. Weights: profit factor 25, avg win/loss 20, win rate, drawdown, consistency 15, recovery 10.",
        theme::text_tertiary(),
    );
}

fn short_axis(key: ComponentKey) -> &'static str {
    match key {
        ComponentKey::WinRate => "Win rate",
        ComponentKey::ProfitFactor => "Profit factor",
        ComponentKey::AvgWinLoss => "Avg W/L",
        ComponentKey::Drawdown => "Drawdown",
        ComponentKey::Recovery => "Recovery",
        ComponentKey::Consistency => "Consistency",
    }
}

fn daily_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    title_row(ui, "Daily P&L", "Realised result by trading day", |_| {});
    let bars: Vec<Bar> = overview
        .days
        .iter()
        .map(|day| {
            let value = day.net.to_f64().unwrap_or(0.0);
            Bar {
                label: format::short_civil(day.date),
                value,
                color: if value >= 0.0 {
                    theme::WIN
                } else {
                    theme::LOSS
                },
                detail: format!(
                    "{} {currency} \u{00b7} {} trade{}",
                    format::signed_money(day.net),
                    day.trades,
                    if day.trades == 1 { "" } else { "s" }
                ),
            }
        })
        .collect();
    charts::bar_chart(
        ui,
        &bars,
        &|v| format::compact(v),
        CHART_HEIGHT,
        "No trading days in this range",
    );
}

fn drawdown_card(
    ui: &mut Ui,
    overview: &Overview,
    state: &mut OverviewState,
    offset: Offset<'_>,
    currency: &str,
) {
    let can_percent = overview.drawdown.iter().any(|p| p.pct.is_some());
    let has_previous = !overview.previous_drawdown.is_empty();
    let percent = state.drawdown_percent && can_percent;
    title_row(
        ui,
        "Drawdown",
        "Distance from the running equity peak",
        |ui| {
            if has_previous {
                toggle(ui, &mut state.show_previous, "vs prev");
            }
            if can_percent {
                toggle(ui, &mut state.drawdown_percent, "%");
            }
        },
    );
    let series = |points: &[crate::model::journal::equity::DrawdownPoint]| -> Vec<(f64, f64)> {
        points
            .iter()
            .map(|p| {
                let depth = if percent {
                    p.pct.unwrap_or(0.0) * 100.0
                } else {
                    p.depth.to_f64().unwrap_or(0.0)
                };
                (p.ts_ms as f64, -depth)
            })
            .collect()
    };
    let mut lines = vec![Line {
        name: "Drawdown",
        points: series(&overview.drawdown),
        color: theme::LOSS,
        fill: true,
        dashed: false,
    }];
    if state.show_previous && has_previous {
        lines.push(Line {
            name: "Previous",
            points: series(&overview.previous_drawdown),
            color: theme::text_tertiary(),
            fill: false,
            dashed: true,
        });
    }
    let x_label = |x: f64| format::short_date(x as i64, offset);
    let y_label = |y: f64| {
        if percent {
            format!("{:.1}%", y)
        } else {
            format::compact(y)
        }
    };
    charts::line_chart(
        ui,
        egui::Id::new("journal-drawdown"),
        &LineChart {
            lines: &lines,
            baseline: Some(0.0),
            x_label: &x_label,
            y_label: &y_label,
            empty: "No closed trades in this range",
        },
        CHART_HEIGHT,
    );
    let _ = currency;
}

fn distribution_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    title_row(
        ui,
        "Result distribution",
        "Closed trades by net P&L",
        |_| {},
    );
    let bars: Vec<Bar> = overview
        .histogram
        .bins
        .iter()
        .map(|bin| Bar {
            label: format::compact(bin.low),
            value: bin.count as f64,
            color: if bin.high <= 0.0 {
                theme::LOSS
            } else {
                theme::WIN
            },
            detail: format!(
                "{} to {} {currency} \u{00b7} {} trade{}",
                format::compact(bin.low),
                format::compact(bin.high),
                bin.count,
                if bin.count == 1 { "" } else { "s" }
            ),
        })
        .collect();
    charts::bar_chart(
        ui,
        &bars,
        &|v| format!("{v:.0}"),
        CHART_HEIGHT,
        "No closed trades in this range",
    );
}

fn sides_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    title_row(ui, "Long vs short", "Closed trades by direction", |_| {});
    for (name, stats) in [("Long", &overview.long), ("Short", &overview.short)] {
        side_row(
            ui,
            name,
            stats,
            currency,
            overview.long.trades + overview.short.trades,
        );
        ui.add_space(14.0);
    }
}

fn side_row(ui: &mut Ui, name: &str, stats: &SideStats, currency: &str, total: usize) {
    ui.horizontal(|ui| {
        ui.label(kit::text(name, size::BODY, SEMIBOLD, theme::text_primary()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let text = format!("{} {currency}", format::signed_money(stats.net))
                .trim()
                .to_owned();
            ui.label(kit::text(text, size::BODY, MEDIUM, decimal_tone(stats.net)));
        });
    });
    charts::split_bar(
        ui,
        stats.trades as f64,
        (total - stats.trades.min(total)) as f64,
        ui.available_width(),
    );
    let rate = match &stats.win_rate {
        Metric::Value(rate) => format::percent(*rate),
        Metric::Unavailable(_) => NONE.to_owned(),
    };
    note(
        ui,
        format!(
            "{} trades \u{00b7} {} W / {} L \u{00b7} win rate {rate}",
            stats.trades, stats.wins, stats.losses
        ),
        theme::text_secondary(),
    );
}

fn recent_card(
    ui: &mut Ui,
    overview: &Overview,
    trades: &[Trade],
    offset: Offset<'_>,
    output: &mut OverviewOutput,
) {
    title_row(
        ui,
        "Recent trades",
        &format!(
            "{} closed \u{00b7} {} open",
            overview.summary.closed, overview.summary.open
        ),
        |_| {},
    );
    if overview.recent.is_empty() {
        ui.label(kit::text(
            "Nothing closed in this range.",
            size::BODY,
            REGULAR,
            theme::text_tertiary(),
        ));
        return;
    }
    for key in &overview.recent {
        let Some(trade) = trades.iter().find(|trade| &trade.key == key) else {
            continue;
        };
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, kit::primary(0.05));
        }
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(6.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 8.0;
        paper_grid::text_cell(
            &mut row,
            &trade.instrument.display_name,
            theme::text_primary(),
        );
        let (side, tint) = match trade.direction {
            Direction::Long => ("LONG", theme::WIN),
            Direction::Short => ("SHORT", theme::LOSS),
        };
        paper_grid::tag(&mut row, side, tint);
        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let net = trade.net_pnl();
            paper_grid::figure_bold(ui, &format::signed_money(net), decimal_tone(net));
            if let Some(close) = trade.close_ts_ms {
                paper_grid::figure(
                    ui,
                    &format::short_date(close, offset),
                    theme::text_tertiary(),
                );
            }
        });
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            output.open_trade = Some(trade.key.clone());
        }
    }
    let _ = Icon::Check;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::calendar::utc;
    use crate::model::journal::model::{CostBasis, Side};
    use crate::model::journal::overview::{Input, compute};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::exec;

    fn trades() -> Vec<Trade> {
        let mut e = Vec::new();
        for i in 0..8i64 {
            let exit = if i % 3 == 0 { "90" } else { "112" };
            e.push(exec(
                &format!("{i}a"),
                Side::Buy,
                "1",
                "100",
                (i + 1) * 86_400_000,
            ));
            e.push(exec(
                &format!("{i}b"),
                Side::Sell,
                "1",
                exit,
                (i + 1) * 86_400_000 + 1_000,
            ));
        }
        reconstruct(&e, |_| CostBasis::AverageCost)
    }

    fn draw(overview: &Overview, trades: &[Trade], width: f32) {
        let ctx = egui::Context::default();
        let mut state = OverviewState {
            show_previous: true,
            drawdown_percent: true,
        };
        for _ in 0..2 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        vec2(width, 1800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        show(ui, overview, &mut state, &utc, trades);
                    });
                },
            );
            out.textures_delta.clear();
        }
    }

    #[test]
    fn the_page_draws_wide_narrow_and_empty() {
        let trades = trades();
        let refs: Vec<&Trade> = trades.iter().collect();
        let overview = compute(&Input {
            trades: &refs,
            previous: Some(&refs[..3]),
            before: &[],
            accounts: &[],
            from_ms: Some(0),
            to_ms: Some(100 * 86_400_000),
            currency: None,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        draw(&overview, &trades, 1400.0);
        draw(&overview, &trades, 700.0);
        let empty = compute(&Input {
            trades: &[],
            previous: None,
            before: &[],
            accounts: &[],
            from_ms: None,
            to_ms: None,
            currency: None,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        draw(&empty, &[], 1400.0);
    }
}
