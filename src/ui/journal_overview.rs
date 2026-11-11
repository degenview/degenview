//! The Journal's Overview page: headline cards, a strip of smaller figures, the realised-equity
//! curve, the Edge Score, daily P&L, drawdown, the distribution of results, long against short and
//! the latest trades. A view over a precomputed [`Overview`]; it returns what the user clicked.
//! The look (cards, chips, pills, colours) is `journal_kit`'s, shared with every other page.

use eframe::egui::{self, Sense, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::icons::Icon;
use super::journal_charts::{self as charts, Bar, Line, LineChart};
use super::journal_kit::{self as jk, Card, Chip, Sign};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::theme;
use degenview_journal::calendar::Offset;
use degenview_journal::cells::NONE;
use degenview_journal::edge::{self, ComponentKey, MIN_TRADES};
use degenview_journal::equity::{DrawdownPoint, EquityPoint};
use degenview_journal::format;
use degenview_journal::metric::Metric;
use degenview_journal::model::{Direction, Trade};
use degenview_journal::overview::{Overview, SideStats};

/// Card sizes.
const KPI_HEIGHT: f32 = 148.0;
const CHART_HEIGHT: f32 = 200.0;
/// A chart card's height: the chart plus its header.
const CHART_CARD: f32 = CHART_HEIGHT + 96.0;
/// Below this width the page stacks its cards.
const WIDE: f32 = 980.0;
/// Six headline cards side by side need at least this width (each ring sits beside its figure).
const KPI_SIX_MIN: f32 = 1240.0;

#[derive(Default)]
pub struct OverviewState {
    pub show_previous: bool,
    /// Drawdown as a percentage of the balance instead of an amount.
    pub drawdown_percent: bool,
    /// The equity card shows cumulative net P&L instead of account equity.
    pub cumulative: bool,
}

#[derive(Default)]
pub struct OverviewOutput {
    /// A currency picked from the selector.
    pub currency: Option<Option<String>>,
    /// A recent trade clicked.
    pub open_trade: Option<String>,
    /// "View all" on the recent trades.
    pub view_all: bool,
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

fn decimal_sign(value: Decimal) -> Sign {
    Sign::of_decimal(value)
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
    let total_gap = jk::GAP * (weights.len() as f32 - 1.0);
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
        left += width + jk::GAP;
    }
    ui.allocate_rect(
        egui::Rect::from_min_max(top, egui::pos2(top.x + available + total_gap, bottom)),
        Sense::hover(),
    );
    ui.add_space(jk::GAP);
}

/// `weights` side by side when `wide`, else one card per row, each the full width.
fn cards_row(ui: &mut Ui, wide: bool, weights: &[f32], mut content: impl FnMut(&mut Ui, usize)) {
    if wide {
        weighted_row(ui, weights, content);
    } else {
        for index in 0..weights.len() {
            weighted_row(ui, &[1.0], |ui, _| content(ui, index));
        }
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

    // Headline cards: the Net P&L hero is the widest.
    let cards: [&dyn Fn(&mut Ui); 6] = [
        &|ui| net_card(ui, overview, currency),
        &|ui| win_rate_card(ui, overview),
        &|ui| profit_factor_card(ui, overview),
        &|ui| avg_card(ui, overview),
        &|ui| expectancy_card(ui, overview, currency),
        &|ui| day_win_card(ui, overview),
    ];
    // Six across need room for a ring beside its text; below that, three across, then two.
    let per_row = if ui.available_width() >= KPI_SIX_MIN {
        6
    } else if wide {
        3
    } else {
        2
    };
    if per_row == 6 {
        weighted_row(ui, &[1.55, 1.0, 1.0, 1.15, 1.0, 1.0], |ui, index| {
            cards[index](ui)
        });
    } else {
        for group in cards.chunks(per_row) {
            weighted_row(ui, &[1.0; 3][..group.len()], |ui, index| group[index](ui));
        }
    }

    jk::chip_strip(ui, &chips(overview, currency));
    ui.add_space(jk::GAP / 2.0);

    // Equity and Edge Score.
    cards_row(ui, wide, &[2.0, 1.0], |ui, index| match index {
        0 => equity_card(ui, overview, state, offset),
        _ => edge_card(ui, overview),
    });

    // Daily P&L and drawdown.
    cards_row(ui, wide, &[1.0, 1.0], |ui, index| match index {
        0 => daily_card(ui, overview, currency),
        _ => drawdown_card(ui, overview, state, offset, currency),
    });

    // Distribution, long against short, recent.
    cards_row(ui, wide, &[1.0, 1.0, 1.0], |ui, index| match index {
        0 => distribution_card(ui, overview, currency),
        1 => sides_card(ui, overview, currency),
        _ => recent_card(ui, overview, trades, offset, &mut output),
    });
    output
}

// MARK: Headline cards

/// One headline card: a label with its definition, the figure and notes on the left, `right` (a
/// ring, a sparkline) on the right.
fn kpi(
    ui: &mut Ui,
    tint: Option<Sign>,
    label: &str,
    info: &str,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) {
    let mut spec = Card::new("").min_height(KPI_HEIGHT);
    spec.tint = tint;
    jk::card(
        ui,
        &spec,
        |_| {},
        |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        jk::label(ui, label);
                        jk::info_mark(ui, info);
                    });
                    ui.add_space(2.0);
                    left(ui);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), right);
            });
        },
    );
}

/// "+x% vs previous period" under a figure; silent without both sides.
fn delta_note(ui: &mut Ui, current: Option<f64>, previous: Option<f64>, as_points: bool) {
    let (Some(current), Some(previous)) = (current, previous) else {
        return;
    };
    let (text, sign) = if as_points {
        jk::delta_text(current * 100.0, previous * 100.0, 1, " pts")
    } else if previous.abs() > f64::EPSILON {
        jk::delta_text((current - previous) / previous.abs() * 100.0, 0.0, 1, "%")
    } else {
        return;
    };
    jk::delta_line(ui, &text, sign);
}

fn net_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    let s = &overview.summary;
    let sign = decimal_sign(s.net_pnl);
    let tint = (s.closed > 0 && sign != Sign::Flat).then_some(sign);
    let series: Vec<f64> = overview
        .equity
        .iter()
        .map(|p| p.cum_net.to_f64().unwrap_or(0.0))
        .collect();
    kpi(
        ui,
        tint,
        "Net P&L",
        "Net P&L: gross P&L of closed trades minus fees. Open trades are not counted.",
        |ui| {
            let text = format!("{} {currency}", format::signed_money(s.net_pnl))
                .trim()
                .to_owned();
            jk::value(ui, &text, sign.text());
            jk::note(
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
                jk::note(ui, "Some fees unknown: a floor", theme::text_tertiary());
            }
        },
        |ui| {
            let width = (ui.available_width() - 8.0).clamp(0.0, 150.0);
            if width > 60.0 {
                jk::sparkline(ui, vec2(width, 52.0), &series, sign);
            }
        },
    );
}

fn win_rate_card(ui: &mut Ui, overview: &Overview) {
    let s = &overview.summary;
    kpi(
        ui,
        None,
        "Win rate",
        "Winning trades divided by closed trades (breakevens count in the total).",
        |ui| {
            let (text, reason) = metric_text(&s.win_rate, |v| format::percent(*v));
            let response = jk::value(ui, &text, theme::text_primary());
            if let Some(reason) = reason {
                response.on_hover_text(reason);
            }
            jk::note(
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
        },
        |ui| charts::ring(ui, s.win_rate.copied(), Sign::Gain.data(), 52.0),
    );
}

fn profit_factor_card(ui: &mut Ui, overview: &Overview) {
    let s = &overview.summary;
    kpi(
        ui,
        None,
        "Profit factor",
        "Gross profit divided by gross loss.",
        |ui| {
            let (text, reason) = metric_text(&s.profit_factor, |v| format::ratio(*v));
            let response = jk::value(ui, &text, theme::text_primary());
            if let Some(reason) = reason {
                response.on_hover_text(reason);
            }
            jk::note(
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
        },
        |_| {},
    );
}

fn avg_card(ui: &mut Ui, overview: &Overview) {
    let s = &overview.summary;
    kpi(
        ui,
        None,
        "Avg win / loss",
        "Average winning trade divided by average losing trade.",
        |ui| {
            let (text, reason) = metric_text(&s.payoff, |v| format::ratio(*v));
            let response = jk::value(ui, &text, theme::text_primary());
            if let Some(reason) = reason {
                response.on_hover_text(reason);
            }
            let amount = |m: &Metric<Decimal>| m.value().and_then(|v| v.to_f64()).unwrap_or(0.0);
            ui.add_space(2.0);
            charts::split_bar(
                ui,
                amount(&s.average_win),
                amount(&s.average_loss),
                ui.available_width().min(150.0),
            );
            let part = |m: &Metric<Decimal>, signed: bool| match m {
                Metric::Value(v) if signed => format::signed_money(*v),
                Metric::Value(v) => format::money(*v),
                Metric::Unavailable(_) => NONE.to_owned(),
            };
            jk::note(
                ui,
                format!(
                    "{} / -{}",
                    part(&s.average_win, true),
                    part(&s.average_loss, false)
                ),
                theme::text_secondary(),
            );
        },
        |_| {},
    );
}

fn expectancy_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    let s = &overview.summary;
    kpi(
        ui,
        None,
        "Expectancy",
        "Net P&L per closed trade.",
        |ui| match &s.expectancy {
            Metric::Value(v) => {
                let text = format!("{} {currency}", format::signed_money(*v))
                    .trim()
                    .to_owned();
                jk::value(ui, &text, decimal_sign(*v).text());
                jk::note(ui, "per closed trade", theme::text_secondary());
            }
            Metric::Unavailable(reason) => {
                jk::value(ui, NONE, theme::text_primary()).on_hover_text(reason.explain());
                jk::note(ui, "per closed trade", theme::text_secondary());
            }
        },
        |_| {},
    );
}

fn day_win_card(ui: &mut Ui, overview: &Overview) {
    let d = &overview.summary.days;
    kpi(
        ui,
        None,
        "Day win rate",
        "Winning days divided by trading days (a trading day has a closed trade).",
        |ui| {
            let (text, reason) = metric_text(&d.day_win_rate, |v| format::percent(*v));
            let response = jk::value(ui, &text, theme::text_primary());
            if let Some(reason) = reason {
                response.on_hover_text(reason);
            }
            jk::note(
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
        },
        |ui| charts::ring(ui, d.day_win_rate.copied(), Sign::Gain.data(), 52.0),
    );
}

// MARK: Chips

fn chips(overview: &Overview, currency: &str) -> Vec<Chip> {
    let s = &overview.summary;
    let money = |value: Decimal| {
        format!("{} {currency}", format::signed_money(value))
            .trim()
            .to_owned()
    };
    let day = |m: &Metric<degenview_journal::calendar::DayCell>| match m {
        Metric::Value(day) => (
            format!(
                "{} \u{00b7} {}",
                money(day.net),
                format::short_civil(day.date)
            ),
            decimal_sign(day.net),
        ),
        Metric::Unavailable(_) => (NONE.to_owned(), Sign::Flat),
    };
    let (best, best_sign) = day(&s.days.best);
    let (worst, worst_sign) = day(&s.days.worst);
    vec![
        Chip {
            icon: Icon::Hash,
            label: "Total trades",
            value: format!("{}", s.closed + s.open),
            sign: Sign::Flat,
            tip: "Closed and open trades in the selection.",
        },
        Chip {
            icon: Icon::ChartUptrend,
            label: "Gross P&L",
            value: money(s.gross_pnl),
            sign: decimal_sign(s.gross_pnl),
            tip: "Realised P&L of closed trades before fees.",
        },
        Chip {
            icon: Icon::Percent,
            label: "Fees",
            value: if s.fees_complete {
                format::money(s.fees)
            } else {
                format!("\u{2265} {}", format::money(s.fees))
            },
            sign: Sign::Flat,
            tip: "Fees charged on closed trades, with the entry fees of what has closed.",
        },
        Chip {
            icon: Icon::Calendar,
            label: "Trading days",
            value: format!("{}", s.days.trading_days),
            sign: Sign::Flat,
            tip: "Days with at least one closed trade, in your time zone.",
        },
        Chip {
            icon: Icon::Check,
            label: "Winning days",
            value: format!("{}", s.days.winning),
            sign: Sign::Flat,
            tip: "Days whose net P&L is positive.",
        },
        Chip {
            icon: Icon::ArrowUpRight,
            label: "Best day",
            value: best,
            sign: best_sign,
            tip: "The trading day with the highest net P&L.",
        },
        Chip {
            icon: Icon::ArrowDownRight,
            label: "Worst day",
            value: worst,
            sign: worst_sign,
            tip: "The trading day with the lowest net P&L.",
        },
        Chip {
            icon: Icon::ChartLine,
            label: "Max drawdown",
            value: match s.drawdown.max_pct {
                Some(pct) => format!(
                    "{} ({})",
                    format::money(s.drawdown.max),
                    format::percent(pct)
                ),
                None => format::money(s.drawdown.max),
            },
            sign: Sign::Flat,
            tip: "Deepest fall of realised equity below its running peak. The percentage needs an initial balance.",
        },
        Chip {
            icon: Icon::Sparkles,
            label: "Average R",
            value: match &s.expectancy_r {
                Metric::Value(r) => format!("{} ({})", format::r_multiple(*r), s.r.count),
                Metric::Unavailable(_) => NONE.to_owned(),
            },
            sign: match &s.expectancy_r {
                Metric::Value(r) => Sign::of(r.to_f64().unwrap_or(0.0)),
                Metric::Unavailable(_) => Sign::Flat,
            },
            tip: "Mean realised R over trades that have a stop (their count in brackets). Needs a stop on the trade or in its review.",
        },
        Chip {
            icon: Icon::Clock,
            label: "Avg duration",
            value: match &s.durations.average_ms {
                Metric::Value(ms) => format::duration(*ms),
                Metric::Unavailable(_) => NONE.to_owned(),
            },
            sign: Sign::Flat,
            tip: "Mean time from open to close.",
        },
        Chip {
            icon: Icon::Flame,
            label: "Streak",
            value: match s.streaks.current {
                0 => NONE.to_owned(),
                1 => "1 win".to_owned(),
                n if n > 0 => format!("{n} wins"),
                -1 => "1 loss".to_owned(),
                n => format!("{} losses", -n),
            },
            sign: Sign::of(s.streaks.current as f64),
            tip: "The winning or losing streak in progress, in close order. A breakeven ends it.",
        },
    ]
}

// MARK: Charts

fn equity_card(ui: &mut Ui, overview: &Overview, state: &mut OverviewState, offset: Offset<'_>) {
    let has_balance = overview.initial_balance.is_some();
    let equity_mode = has_balance && !state.cumulative;
    let title = if equity_mode {
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
    let show_previous = state.show_previous;
    let spec = Card::new(title).subtitle(&subtitle).min_height(CHART_CARD);
    jk::card(
        ui,
        &spec,
        |ui| {
            if has_balance {
                let mut equity = !state.cumulative;
                jk::flag_segments(
                    ui,
                    egui::Id::new("journal-equity-mode"),
                    "Cumulative P&L",
                    "Equity",
                    &mut equity,
                );
                state.cumulative = !equity;
            }
            if has_previous && kit::value_chip(ui, "vs prev", state.show_previous).clicked() {
                state.show_previous = !state.show_previous;
            }
        },
        |ui| {
            let value = |point: &EquityPoint| {
                let amount = if equity_mode {
                    point.equity.unwrap_or(point.cum_net)
                } else {
                    point.cum_net
                };
                amount.to_f64().unwrap_or(0.0)
            };
            let series = |points: &[EquityPoint]| -> Vec<(f64, f64)> {
                points.iter().map(|p| (p.ts_ms as f64, value(p))).collect()
            };
            let last = overview.equity.last().map(value);
            let baseline = if equity_mode {
                overview.initial_balance.and_then(|b| b.to_f64())
            } else {
                Some(0.0)
            };
            let sign = match (last, baseline) {
                (Some(last), Some(base)) if last < base => Sign::Loss,
                _ => Sign::Gain,
            };
            let mut lines = vec![Line {
                name: "Now".into(),
                points: series(&overview.equity),
                color: sign.data(),
                fill: true,
                dashed: false,
            }];
            if show_previous && has_previous {
                lines.push(Line {
                    name: "Previous".into(),
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
                    badge: true,
                },
                CHART_HEIGHT,
            );
        },
    );
}

/// The Edge Score of the previous period, when there is one to compare with.
fn previous_edge(overview: &Overview) -> Option<f64> {
    let previous = overview.previous.as_ref()?;
    edge::edge_score(previous, overview.initial_balance)
        .score
        .copied()
}

fn edge_card(ui: &mut Ui, overview: &Overview) {
    let edge = &overview.edge;
    let info = format!(
        "Edge Score v{} (LuxAlgo's published formula). Measures the record, not the future; {} \
         closed trades. Weights: profit factor 25, avg win/loss 20, win rate, drawdown, \
         consistency 15, recovery 10.",
        edge.version, edge.closed_trades
    );
    let spec = Card::new("Edge score").info(&info).min_height(CHART_CARD);
    jk::card(
        ui,
        &spec,
        |_| {},
        |ui| {
            let axes: Vec<(&str, f64)> = edge
                .components
                .iter()
                .map(|c| (short_axis(c.key), c.value))
                .collect();
            let total = ui.available_width();
            let left_width = (total * 0.34).clamp(90.0, 150.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // The score and its delta stack in a column of their own.
                ui.allocate_ui_with_layout(
                    vec2(left_width, CHART_HEIGHT),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left_width);
                        match &edge.score {
                            Metric::Value(score) => {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(kit::text(
                                        format!("{score:.0}"),
                                        34.0,
                                        SEMIBOLD,
                                        theme::text_primary(),
                                    ));
                                    ui.label(kit::text(
                                        "/ 100",
                                        size::BODY + 1.0,
                                        REGULAR,
                                        theme::text_tertiary(),
                                    ));
                                });
                                if let Some(previous) = previous_edge(overview) {
                                    let (text, sign) = jk::delta_text(*score, previous, 0, "");
                                    jk::delta_line(ui, &text, sign);
                                }
                            }
                            Metric::Unavailable(_) => {
                                ui.label(kit::text(
                                    format!("needs {MIN_TRADES} trades"),
                                    size::BODY,
                                    MEDIUM,
                                    theme::text_tertiary(),
                                ));
                            }
                        }
                    },
                );
                ui.allocate_ui(
                    vec2((total - left_width - 8.0).max(60.0), CHART_HEIGHT),
                    |ui| {
                        charts::radar(ui, &axes, CHART_HEIGHT + 24.0, Sign::Gain.data());
                    },
                );
            });
        },
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
    let spec = Card::new("Daily P&L")
        .subtitle("By trading day")
        .min_height(CHART_CARD);
    jk::card(
        ui,
        &spec,
        |_| {},
        |ui| {
            let bars: Vec<Bar> = overview
                .days
                .iter()
                .map(|day| {
                    let sign = decimal_sign(day.net);
                    Bar {
                        label: format::short_civil(day.date),
                        value: day.net.to_f64().unwrap_or(0.0),
                        color: if sign == Sign::Loss {
                            Sign::Loss.data()
                        } else {
                            Sign::Gain.data()
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
        },
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
    let show_previous = state.show_previous;
    let spec = Card::new("Drawdown")
        .subtitle("Distance from the running equity peak")
        .min_height(CHART_CARD);
    jk::card(
        ui,
        &spec,
        |ui| {
            if can_percent {
                let mut flag = state.drawdown_percent;
                let amount = if currency.is_empty() {
                    "Amount"
                } else {
                    currency
                };
                jk::flag_segments(
                    ui,
                    egui::Id::new("journal-drawdown-unit"),
                    amount,
                    "%",
                    &mut flag,
                );
                state.drawdown_percent = flag;
            }
            if has_previous && kit::value_chip(ui, "vs prev", state.show_previous).clicked() {
                state.show_previous = !state.show_previous;
            }
        },
        |ui| {
            let series = |points: &[DrawdownPoint]| -> Vec<(f64, f64)> {
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
                name: "Drawdown".into(),
                points: series(&overview.drawdown),
                color: Sign::Loss.data(),
                fill: true,
                dashed: false,
            }];
            if show_previous && has_previous {
                lines.push(Line {
                    name: "Previous".into(),
                    points: series(&overview.previous_drawdown),
                    color: theme::text_tertiary(),
                    fill: false,
                    dashed: true,
                });
            }
            let x_label = |x: f64| format::short_date(x as i64, offset);
            let y_label = |y: f64| {
                if percent {
                    format!("{y:.1}%")
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
                    badge: false,
                },
                CHART_HEIGHT,
            );
        },
    );
}

fn distribution_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    let spec = Card::new("Result distribution")
        .subtitle("Closed trades by net P&L")
        .min_height(CHART_CARD);
    jk::card(
        ui,
        &spec,
        |_| {},
        |ui| {
            let bars: Vec<Bar> = overview
                .histogram
                .bins
                .iter()
                .map(|bin| Bar {
                    label: format::compact(bin.low),
                    value: bin.count as f64,
                    color: if bin.high <= 0.0 {
                        Sign::Loss.data()
                    } else {
                        Sign::Gain.data()
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
        },
    );
}

fn sides_card(ui: &mut Ui, overview: &Overview, currency: &str) {
    let spec = Card::new("Long vs short")
        .subtitle("Closed trades by direction")
        .min_height(CHART_CARD);
    jk::card(
        ui,
        &spec,
        |_| {},
        |ui| {
            let total = overview.long.trades + overview.short.trades;
            for (name, stats) in [("Long", &overview.long), ("Short", &overview.short)] {
                side_row(ui, name, stats, currency, total);
                ui.add_space(16.0);
            }
        },
    );
}

fn side_row(ui: &mut Ui, name: &str, stats: &SideStats, currency: &str, total: usize) {
    ui.horizontal(|ui| {
        ui.label(kit::text(name, size::BODY, SEMIBOLD, theme::text_primary()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let text = format!("{} {currency}", format::signed_money(stats.net))
                .trim()
                .to_owned();
            ui.label(kit::text(
                text,
                size::BODY,
                MEDIUM,
                decimal_sign(stats.net).text(),
            ));
        });
    });
    ui.add_space(2.0);
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
    jk::note(
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
    let subtitle = format!(
        "{} closed \u{00b7} {} open",
        overview.summary.closed, overview.summary.open
    );
    let spec = Card::new("Recent trades")
        .subtitle(&subtitle)
        .min_height(CHART_CARD);
    let mut view_all = false;
    jk::card(
        ui,
        &spec,
        |ui| {
            if !overview.recent.is_empty() && jk::link(ui, "View all").clicked() {
                view_all = true;
            }
        },
        |ui| {
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
                recent_row(ui, trade, offset, output);
            }
        },
    );
    output.view_all |= view_all;
}

fn recent_row(ui: &mut Ui, trade: &Trade, offset: Offset<'_>, output: &mut OverviewOutput) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    jk::paint_row(ui, rect, false, response.hovered());
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    row.add(
        egui::Label::new(kit::text(
            &trade.instrument.display_name,
            size::BODY,
            SEMIBOLD,
            theme::text_primary(),
        ))
        .truncate(),
    );
    let (side, tone) = match trade.direction {
        Direction::Long => ("LONG", kit::Tone::Good),
        Direction::Short => ("SHORT", kit::Tone::Bad),
    };
    jk::pill(&mut row, side, tone);
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let net = trade.net_pnl();
        ui.label(kit::text(
            format::signed_money(net),
            size::BODY,
            SEMIBOLD,
            decimal_sign(net).text(),
        ));
        if let Some(close) = trade.close_ts_ms {
            ui.label(kit::text(
                format::short_date(close, offset),
                size::BODY - 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        }
    });
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        output.open_trade = Some(trade.key.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_journal::calendar::utc;
    use degenview_journal::model::{CostBasis, Side};
    use degenview_journal::overview::{Input, compute};
    use degenview_journal::reconstruct::reconstruct;
    use degenview_journal::testing::exec;
    use std::collections::HashMap;

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
            cumulative: false,
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
    fn the_edge_delta_needs_a_previous_period_with_a_score() {
        let trades = trades();
        let refs: Vec<&Trade> = trades.iter().collect();
        let reviews = HashMap::new();
        let make = |previous: Option<&'_ [&'_ Trade]>| {
            compute(&Input {
                trades: &refs,
                previous,
                before: &[],
                accounts: &[],
                from_ms: Some(0),
                to_ms: Some(100 * 86_400_000),
                currency: None,
                reviews: &reviews,
                breakeven_tolerance: Decimal::ZERO,
                offset: &utc,
            })
        };
        // No previous period: nothing to compare.
        assert_eq!(previous_edge(&make(None)), None);
        // Three trades are below the Edge Score's minimum, so a previous period has no score.
        assert_eq!(previous_edge(&make(Some(&refs[..3]))), None);
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
            reviews: &HashMap::new(),
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        for width in [1400.0, 1290.0, 1100.0, 700.0] {
            draw(&overview, &trades, width);
        }
        let empty = compute(&Input {
            trades: &[],
            previous: None,
            before: &[],
            accounts: &[],
            from_ms: None,
            to_ms: None,
            currency: None,
            reviews: &HashMap::new(),
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        draw(&empty, &[], 1400.0);
    }
}
