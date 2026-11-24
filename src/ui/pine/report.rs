//! The strategy backtest report panel: headline metrics, the equity curve, and the closed trades,
//! rendered from a `strategy::StrategyReport`. Pure view — nothing here runs the strategy or
//! touches the store. A port of `PineStrategyReportView` / `PineEquityCurveView` /
//! `PineReportFormat`.

use crate::i18n::tr;
use crate::pine::strategy::StrategyReport;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, RichText, Stroke};

/// Number formatting for the strategy report, ported from the Swift `PineReportFormat`.
pub struct PineReportFormat;

/// Inserts thousands separators into the whole part of a decimal string.
fn group(digits: &str) -> String {
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// Fixed-decimal, grouped formatting of a possibly-negative value, with its sign.
fn grouped(value: f64, decimals: usize) -> String {
    let text = format!("{:.decimals$}", value.abs());
    let grouped = match text.split_once('.') {
        Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
        None => group(&text),
    };
    let rounds_to_zero = !grouped.chars().any(|c| c.is_ascii_digit() && c != '0');
    if value < 0.0 && !rounds_to_zero {
        format!("-{grouped}")
    } else {
        grouped
    }
}

impl PineReportFormat {
    /// `1,234.50` — two decimals, grouped.
    pub fn money(value: f64) -> String {
        grouped(value, 2)
    }

    /// `+1,234.50` / `-1,234.50`, always signed.
    pub fn signed_money(value: f64) -> String {
        let body = grouped(value.abs(), 2);
        if value >= 0.0 {
            format!("+{body}")
        } else {
            format!("-{body}")
        }
    }

    /// A fraction as `12.34%`; with `signed`, `+12.34%` / `-12.34%`.
    pub fn percent(fraction: f64, signed: bool) -> String {
        let body = format!("{:.2}%", (fraction * 100.0).abs());
        if signed {
            if fraction >= 0.0 {
                format!("+{body}")
            } else {
                format!("-{body}")
            }
        } else {
            body
        }
    }

    /// A price with up to seven significant digits, grouped: `67,432.1`, `0.00000278`.
    pub fn price(value: f64) -> String {
        let abs = value.abs();
        if !abs.is_finite() || abs == 0.0 {
            return "0".to_owned();
        }
        let exponent = abs.log10().floor() as i32;
        let decimals = (6 - exponent).clamp(0, 12) as usize;
        let text = format!("{abs:.decimals$}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        let grouped = match text.split_once('.') {
            Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
            None => group(text),
        };
        if value < 0.0 {
            format!("-{grouped}")
        } else {
            grouped
        }
    }

    /// A value on a chart's scale, as precise as the chart's `span` needs: whole units once it
    /// covers a hundred or more (`10,720`), two decimals below that (`1,000.35`).
    pub fn axis(value: f64, span: f64) -> String {
        grouped(value, if span.abs() >= 100.0 { 0 } else { 2 })
    }

    /// A count, grouped: `1,000`.
    pub fn count(value: usize) -> String {
        group(&value.to_string())
    }

    /// A plain ratio with two decimals: `1.23`.
    pub fn ratio(value: f64) -> String {
        format!("{value:.2}")
    }
}

/// Green above zero, red below, neutral at zero.
pub(crate) fn tint(value: f64, neutral: Color32) -> Color32 {
    if value > 0.0 {
        theme::WIN
    } else if value < 0.0 {
        theme::LOSS
    } else {
        neutral
    }
}

/// The `pineReportBox` look: padding 10, quaternary fill, radius 8.
pub fn report_box<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::quaternary(0.4))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui)
        })
        .inner
}

/// Renders the whole report (metrics, equity curve, trades) in one box. A no-op when `report`
/// is `None`.
pub fn show(ui: &mut egui::Ui, report: Option<&StrategyReport>) {
    let Some(report) = report else {
        return;
    };
    let neutral = theme::text_primary();
    report_box(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.label(
            RichText::new(tr!("Strategy report"))
                .size(10.0)
                .strong()
                .color(theme::text_primary()),
        );
        if let Some(notice) = synthetic_fill_notice(report.chart) {
            ui.label(
                RichText::new(notice)
                    .size(10.0)
                    .color(theme::text_secondary()),
            );
        }
        metric_grid(ui, report, neutral);
        equity_curve(ui, &report.equity, report.settings.initial_capital);
        if !report.trades.is_empty() {
            trade_list(ui, report, neutral);
        }
    });
}

/// What to tell the user about a backtest on a price-based chart: its orders filled at that
/// chart's constructed prices, which no market traded at, so the results cannot be had in trading.
/// Paper trading and the market's own price alerts are unaffected: they use the market's prices.
pub fn synthetic_fill_notice(chart: crate::model::ChartKind) -> Option<String> {
    use crate::model::ChartKind;
    let name = match chart {
        ChartKind::Standard => return None,
        ChartKind::HeikinAshi => tr!("Heikin Ashi"),
        ChartKind::Renko => tr!("Renko"),
        ChartKind::Kagi => tr!("Kagi"),
        ChartKind::LineBreak => tr!("Line Break"),
        ChartKind::PointAndFigure => tr!("Point & Figure"),
        ChartKind::Range => tr!("Range"),
    };
    Some(tr!(
        "Run on a {name} chart: orders filled at its constructed prices, not at prices the market traded at. Treat the results as indicative only.",
        name = name
    ))
}

fn metric_grid(ui: &mut egui::Ui, report: &StrategyReport, neutral: Color32) {
    let drawdown = report.max_drawdown();
    let net = report.net_profit();
    let win_rate = report
        .win_rate()
        .map_or_else(|| "—".to_owned(), |r| PineReportFormat::percent(r, false));
    let profit_factor = report
        .profit_factor()
        .map_or_else(|| "—".to_owned(), PineReportFormat::ratio);
    let avg_trade = report
        .average_trade()
        .map_or_else(|| "—".to_owned(), PineReportFormat::signed_money);

    let metrics: [(&str, String, f64); 8] = [
        (
            tr!("Net profit"),
            format!(
                "{}  {}",
                PineReportFormat::signed_money(net),
                PineReportFormat::percent(report.return_fraction(), true)
            ),
            net,
        ),
        (tr!("Closed trades"), report.trades.len().to_string(), 0.0),
        (tr!("Win rate"), win_rate, 0.0),
        (tr!("Profit factor"), profit_factor, 0.0),
        (
            tr!("Max drawdown"),
            format!(
                "{}  {}",
                PineReportFormat::money(drawdown.0),
                PineReportFormat::percent(drawdown.1, false)
            ),
            0.0,
        ),
        (tr!("Avg trade"), avg_trade, 0.0),
        (
            tr!("Open P&L"),
            PineReportFormat::signed_money(report.open_profit),
            report.open_profit,
        ),
        (
            tr!("Final equity"),
            PineReportFormat::money(report.final_equity()),
            0.0,
        ),
    ];

    // `LazyVGrid(.adaptive(minimum: 130), spacing: 8)`: as many columns as fit.
    let width = ui.available_width();
    let columns = (((width + 8.0) / (130.0 + 8.0)).floor() as usize).max(1);
    let column_width = (width - 8.0 * (columns - 1) as f32) / columns as f32;
    egui::Grid::new("pine-report-metrics")
        .num_columns(columns)
        .spacing(egui::vec2(8.0, 8.0))
        .min_col_width(column_width)
        .show(ui, |ui| {
            for (index, (title, value, tone)) in metrics.into_iter().enumerate() {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        RichText::new(title)
                            .size(9.0)
                            .color(theme::text_secondary()),
                    );
                    ui.label(
                        RichText::new(value)
                            .size(10.0)
                            .monospace()
                            .color(tint(tone, neutral)),
                    );
                });
                if (index + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
}

fn trade_list(ui: &mut egui::Ui, report: &StrategyReport, neutral: Color32) {
    let latest: Vec<_> = report.trades.iter().rev().take(100).collect();
    egui::CollapsingHeader::new(
        RichText::new(tr!("Trades ({count})", count = report.trades.len()))
            .size(10.0)
            .color(theme::text_primary()),
    )
    .id_salt("pine-report-trades")
    .default_open(false)
    .show(ui, |ui| {
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for trade in latest {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let cell = |ui: &mut egui::Ui, width: f32, text: String| {
                            ui.allocate_ui(egui::vec2(width, 12.0), |ui| {
                                ui.label(
                                    RichText::new(text)
                                        .size(9.0)
                                        .monospace()
                                        .color(theme::text_primary()),
                                );
                            });
                        };
                        cell(ui, 34.0, format!("#{}", trade.id));
                        cell(
                            ui,
                            38.0,
                            if trade.is_long {
                                tr!("Long")
                            } else {
                                tr!("Short")
                            }
                            .to_owned(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} → {}",
                                PineReportFormat::price(trade.entry_price),
                                PineReportFormat::price(trade.exit_price)
                            ))
                            .size(9.0)
                            .monospace()
                            .color(theme::text_primary()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(PineReportFormat::signed_money(trade.profit))
                                    .size(9.0)
                                    .monospace()
                                    .color(tint(trade.profit, neutral)),
                            );
                        });
                    })
                    .response
                    .on_hover_text(trade_summary(trade));
                }
            });
    });
}

/// What hovering a trade says: its entry and exit ids, its size, and the comments of its orders.
pub(crate) fn trade_summary(trade: &crate::pine::strategy::Trade) -> String {
    let mut text = tr!(
        "{entry} \u{2192} {exit} \u{b7} qty {quantity}",
        entry = trade.entry_id,
        exit = trade.exit_id,
        quantity = PineReportFormat::price(trade.quantity)
    );
    let comments = trade.comments();
    if !comments.is_empty() {
        text.push('\n');
        text.push_str(&comments);
    }
    text
}

/// The height of the equity curve in the report box.
const EQUITY_HEIGHT: f32 = 64.0;

/// Draws the equity curve against the starting capital, downsampled to `POINT_LIMIT` points, with
/// its name in the corner.
fn equity_curve(ui: &mut egui::Ui, equity: &[f64], initial_capital: f64) {
    const POINT_LIMIT: usize = 400;
    const INSET: f32 = 2.0;

    let values = downsample(equity, POINT_LIMIT);
    if values.len() <= 1 {
        return;
    }
    let rect = ui
        .allocate_exact_size(
            egui::vec2(ui.available_width(), EQUITY_HEIGHT),
            egui::Sense::hover(),
        )
        .0;
    let painter = ui.painter_at(rect);
    let weak = theme::text_secondary();

    let low = values
        .iter()
        .copied()
        .fold(initial_capital, f64::min)
        .min(initial_capital);
    let high = values
        .iter()
        .copied()
        .fold(initial_capital, f64::max)
        .max(initial_capital);
    let span = (high - low).max(1e-9);
    let usable = rect.height() - 2.0 * INSET;
    let y = |value: f64| rect.max.y - ((value - low) / span) as f32 * usable - INSET;
    let x = |index: usize| rect.min.x + rect.width() * index as f32 / (values.len() - 1) as f32;

    let baseline_y = y(initial_capital);
    painter.extend(egui::Shape::dashed_line(
        &[
            egui::pos2(rect.min.x, baseline_y),
            egui::pos2(rect.max.x, baseline_y),
        ],
        Stroke::new(1.0, theme::with_alpha(weak, 0.5)),
        3.0,
        3.0,
    ));

    let tone = if values.last().copied().unwrap_or(initial_capital) >= initial_capital {
        theme::WIN
    } else {
        theme::LOSS
    };
    let points: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, &value)| egui::pos2(x(index), y(value)))
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, tone)));

    painter.text(
        rect.min + egui::vec2(4.0, 4.0),
        Align2::LEFT_TOP,
        tr!("Equity"),
        FontId::proportional(9.0),
        weak,
    );
}

/// Keeps the first and last points and evenly spaced ones between.
pub(crate) fn downsample(values: &[f64], limit: usize) -> Vec<f64> {
    if values.len() <= limit || limit <= 1 {
        return values.to_vec();
    }
    (0..limit)
        .map(|index| values[sampled_index(values.len(), limit, index)])
        .collect()
}

/// Which of `len` values the `index`th point of [`downsample`]`(_, limit)` is.
pub(crate) fn sampled_index(len: usize, limit: usize, index: usize) -> usize {
    if len <= limit || limit <= 1 {
        index
    } else {
        index * (len - 1) / (limit - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_signed_money_group_and_keep_two_decimals() {
        assert_eq!(PineReportFormat::money(1_234.5), "1,234.50");
        assert_eq!(PineReportFormat::money(0.0), "0.00");
        assert_eq!(PineReportFormat::signed_money(12.0), "+12.00");
        assert_eq!(PineReportFormat::signed_money(-12.0), "-12.00");
        assert_eq!(PineReportFormat::signed_money(0.0), "+0.00");
    }

    #[test]
    fn percent_and_ratio_match_the_swift_format() {
        assert_eq!(PineReportFormat::percent(0.233_49, false), "23.35%");
        assert_eq!(PineReportFormat::percent(0.233_49, true), "+23.35%");
        assert_eq!(PineReportFormat::percent(-0.041, true), "-4.10%");
        assert_eq!(PineReportFormat::ratio(1.0), "1.00");
        assert_eq!(PineReportFormat::ratio(0.0), "0.00");
    }

    #[test]
    fn price_keeps_significant_digits_and_groups() {
        assert_eq!(PineReportFormat::price(67_432.1), "67,432.1");
        assert_eq!(PineReportFormat::price(0.000_002_78), "0.00000278");
        assert_eq!(PineReportFormat::price(0.0), "0");
        assert_eq!(PineReportFormat::price(1.5), "1.5");
        assert_eq!(PineReportFormat::price(-3.25), "-3.25");
    }

    #[test]
    fn downsample_keeps_ends_and_spaces_the_rest() {
        let values: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let sampled = downsample(&values, 11);
        assert_eq!(sampled.first(), Some(&0.0));
        assert_eq!(sampled.last(), Some(&99.0));
        assert_eq!(sampled.len(), 11);
        assert_eq!(sampled_index(100, 11, 10), 99);
        assert_eq!(sampled_index(100, 11, 5), 49);
        assert_eq!(sampled_index(5, 11, 3), 3, "a short list is not sampled");
    }

    #[test]
    fn axis_values_are_as_precise_as_the_span_needs() {
        assert_eq!(PineReportFormat::axis(1_074_760.71, 74_760.0), "1,074,761");
        assert_eq!(PineReportFormat::axis(10_000.0, 720.0), "10,000");
        assert_eq!(PineReportFormat::axis(1_000.354, 12.0), "1,000.35");
        assert_eq!(PineReportFormat::axis(-42.0, 300.0), "-42");
        assert_eq!(PineReportFormat::count(1_000), "1,000");
        assert_eq!(PineReportFormat::count(200), "200");
    }

    #[test]
    fn the_report_draws_headlessly() {
        let report = StrategyReport {
            settings: Default::default(),
            trades: Vec::new(),
            open_trades: Vec::new(),
            equity: vec![1_000_000.0, 1_020_000.0, 1_010_000.0, 1_050_000.0],
            open_profit: 0.0,
            chart: crate::model::ChartKind::Renko,
        };
        egui::__run_test_ui(|ui| {
            show(ui, Some(&report));
            show(ui, None);
        });
    }
}
