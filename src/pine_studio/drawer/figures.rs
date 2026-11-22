//! The Report tab's strategy figures: the net profit large, the figures that judge a strategy under
//! it, the rest quieter, and the equity curve with its scale and a readout under the pointer.

use super::plot::{self, Plot};
use super::tab_chrome::{self as chrome, SECTION, panel};
use crate::pine::strategy::StrategyReport;
use crate::ui::accessibility;
use crate::ui::pine::report::{self, PineReportFormat};
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Rect, Sense, Ui, WidgetType, pos2, vec2};

/// The narrowest a figure's column gets before the grid takes fewer columns.
const MIN_COLUMN: f32 = 92.0;
const COLUMN_GAP: f32 = 12.0;
const ROW_GAP: f32 = 10.0;
/// The gap between a figure's name and its value.
const LABEL_GAP: f32 = 3.0;
/// The space between the headline, the key figures and the quiet ones.
pub(super) const BLOCK_GAP: f32 = 14.0;
const EQUITY_HEIGHT: f32 = 96.0;
/// The most points of the equity curve drawn; more bars are sampled evenly.
const EQUITY_POINTS: usize = 400;

/// How loud a row of figures is.
#[derive(Clone, Copy)]
pub(super) enum Emphasis {
    /// The figures that judge a strategy.
    Key,
    /// Figures to look up rather than read first.
    Quiet,
}

/// One figure: its name, its value in `color`, and an optional aside after the value.
pub(super) struct Figure {
    pub label: &'static str,
    pub value: String,
    pub color: Color32,
    pub note: Option<String>,
}

impl Figure {
    pub(super) fn new(label: &'static str, value: String, color: Color32) -> Self {
        Self {
            label,
            value,
            color,
            note: None,
        }
    }

    fn note(mut self, note: String) -> Self {
        self.note = Some(note);
        self
    }

    /// What a screen reader says: the name, the value and the aside, which is read even where the
    /// column is too narrow to draw it.
    fn spoken(&self) -> String {
        match &self.note {
            Some(note) => format!("{}: {}, {note}", self.label, self.value),
            None => format!("{}: {}", self.label, self.value),
        }
    }
}

/// The figures card, the equity card under it, and the simulation of the trades.
pub(super) fn strategy(ui: &mut Ui, report: &StrategyReport) {
    if let Some(notice) = report::synthetic_fill_notice(report.chart) {
        chrome::banner(ui, &notice);
    }
    chrome::caption(ui, "Figures");
    panel(ui, |ui| {
        headline(ui, report);
        ui.add_space(BLOCK_GAP);
        let key = key_figures(report);
        grid(ui, &key, key.len(), Emphasis::Key);
        ui.add_space(BLOCK_GAP);
        chrome::rule(ui);
        ui.add_space(BLOCK_GAP);
        grid(ui, &quiet_figures(report), key.len(), Emphasis::Quiet);
    });
    if report.equity.len() > 1 {
        ui.add_space(SECTION);
        chrome::caption(ui, "Equity");
        panel(ui, |ui| equity_chart(ui, report));
    }
    if !report.trades.is_empty() {
        ui.add_space(SECTION);
        super::simulation::simulation_card(ui, report);
    }
}

/// The net profit as large as the tab gets, its return beside it.
fn headline(ui: &mut Ui, report: &StrategyReport) {
    let net = PineReportFormat::signed_money(report.net_profit());
    let ret = PineReportFormat::percent(report.return_fraction(), true);
    let tint = report::tint(report.net_profit(), theme::text_primary());
    let width = ui.available_width();
    let label = kit::galley(
        ui,
        "Net profit",
        kit::size::CAPTION,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let value = kit::digits(
        ui,
        &net,
        kit::size::TITLE2 + 3.0,
        kit::SEMIBOLD,
        tint,
        width,
    );
    let room = width - value.size().x - 8.0;
    let ret_galley = kit::digits(
        ui,
        &ret,
        kit::size::SUBHEADLINE,
        kit::MEDIUM,
        report::tint(report.return_fraction(), theme::text_secondary()),
        room,
    );
    let height = label.size().y + LABEL_GAP + value.size().y;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    // Painted text has no widget of its own, so the screen reader is told what it says.
    accessibility::label_as(
        response,
        WidgetType::Label,
        format!("Net profit {net}, {ret}"),
    );
    let painter = ui.painter();
    painter.galley(rect.min, label, Color32::PLACEHOLDER);
    let value_top = rect.bottom() - value.size().y;
    let value_right = rect.left() + value.size().x;
    painter.galley(pos2(rect.left(), value_top), value, Color32::PLACEHOLDER);
    if room > 30.0 {
        // On the value's baseline: both galleys end in about a fifth of their height of descent.
        let bottom = rect.bottom() - (rect.bottom() - value_top) * 0.2 + ret_galley.size().y * 0.2;
        painter.galley(
            pos2(value_right + 8.0, bottom - ret_galley.size().y),
            ret_galley,
            Color32::PLACEHOLDER,
        );
    }
}

/// Win rate, profit factor, drawdown and how many trades: what tells a strategy apart.
fn key_figures(report: &StrategyReport) -> [Figure; 4] {
    let primary = theme::text_primary();
    let dash = || "\u{2014}".to_owned();
    let (drawdown, drawdown_fraction) = report.max_drawdown();
    let win_rate = match report.win_rate() {
        Some(rate) => Figure::new("Win rate", PineReportFormat::percent(rate, false), primary)
            .note(format!("{} of {}", report.win_count(), report.trades.len())),
        None => Figure::new("Win rate", dash(), primary),
    };
    [
        win_rate,
        Figure::new(
            "Profit factor",
            report
                .profit_factor()
                .map_or_else(dash, PineReportFormat::ratio),
            primary,
        ),
        Figure::new(
            "Max drawdown",
            PineReportFormat::percent(drawdown_fraction, false),
            primary,
        )
        .note(PineReportFormat::money(drawdown)),
        Figure::new("Closed trades", report.trades.len().to_string(), primary),
    ]
}

/// The average trade, the open position's profit and where the equity ended.
fn quiet_figures(report: &StrategyReport) -> [Figure; 3] {
    let neutral = theme::text_secondary();
    let average = report.average_trade();
    [
        Figure::new(
            "Avg trade",
            average.map_or_else(|| "\u{2014}".to_owned(), PineReportFormat::signed_money),
            report::tint(average.unwrap_or(0.0), neutral),
        ),
        Figure::new(
            "Open P&L",
            PineReportFormat::signed_money(report.open_profit),
            report::tint(report.open_profit, neutral),
        ),
        Figure::new(
            "Final equity",
            PineReportFormat::money(report.final_equity()),
            neutral,
        ),
    ]
}

/// How many columns `count` figures take in `width`: as many as fit, evened out over the rows they
/// need (four take four, two or one: never three over one).
pub(super) fn grid_columns(width: f32, count: usize) -> usize {
    let fit = (((width + COLUMN_GAP) / (MIN_COLUMN + COLUMN_GAP)).floor() as usize).max(1);
    if fit >= count {
        return count.max(1);
    }
    let rows = count.div_ceil(fit);
    count.div_ceil(rows)
}

/// Figures in a grid of equal columns, each its name over its value, left-aligned. The columns are
/// those `slots` figures would take, so a shorter row under a longer one keeps its columns.
pub(super) fn grid(ui: &mut Ui, figures: &[Figure], slots: usize, emphasis: Emphasis) {
    let (value_size, value_weight, label_color) = match emphasis {
        Emphasis::Key => (kit::size::BODY, kit::MEDIUM, theme::text_secondary()),
        Emphasis::Quiet => (kit::size::SUBHEADLINE, kit::REGULAR, theme::text_tertiary()),
    };
    let width = ui.available_width();
    let columns = grid_columns(width, slots.max(figures.len()));
    let column = (width - COLUMN_GAP * (columns - 1) as f32) / columns as f32;
    for (row, cells) in figures.chunks(columns).enumerate() {
        if row > 0 {
            ui.add_space(ROW_GAP);
        }
        let galleys: Vec<_> = cells
            .iter()
            .map(|figure| {
                let label = kit::galley(
                    ui,
                    figure.label,
                    kit::size::CAPTION,
                    kit::REGULAR,
                    label_color,
                    column,
                );
                let value = kit::digits(
                    ui,
                    &figure.value,
                    value_size,
                    value_weight,
                    figure.color,
                    column,
                );
                let room = column - value.size().x - 6.0;
                let note = figure.note.as_deref().filter(|_| room > 24.0).map(|note| {
                    kit::digits(
                        ui,
                        note,
                        kit::size::CAPTION,
                        kit::REGULAR,
                        theme::text_tertiary(),
                        room,
                    )
                });
                (label, value, note)
            })
            .collect();
        let height = galleys
            .iter()
            .map(|(label, value, _)| label.size().y + LABEL_GAP + value.size().y)
            .fold(0.0, f32::max);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        for (index, figure) in cells.iter().enumerate() {
            let left = rect.left() + index as f32 * (column + COLUMN_GAP);
            let area = Rect::from_min_size(pos2(left, rect.top()), vec2(column, rect.height()));
            // Named by the figure: the grids of one card share their parent, so a position would
            // give a quiet row's first figure the id of the key row's.
            let response =
                ui.interact(area, ui.id().with(("figure", figure.label)), Sense::hover());
            accessibility::label_as(response, WidgetType::Label, figure.spoken());
        }
        let painter = ui.painter();
        for (index, (label, value, note)) in galleys.into_iter().enumerate() {
            let left = rect.left() + index as f32 * (column + COLUMN_GAP);
            let value_top = rect.top() + label.size().y + LABEL_GAP;
            let value_size = value.size();
            painter.galley(pos2(left, rect.top()), label, Color32::PLACEHOLDER);
            painter.galley(pos2(left, value_top), value, Color32::PLACEHOLDER);
            if let Some(note) = note {
                let bottom = value_top + value_size.y * 0.8 + note.size().y * 0.2;
                painter.galley(
                    pos2(left + value_size.x + 6.0, bottom - note.size().y),
                    note,
                    Color32::PLACEHOLDER,
                );
            }
        }
    }
}

/// The equity after each bar against the initial capital; the pointer reads the bar and its equity.
fn equity_chart(ui: &mut Ui, report: &StrategyReport) {
    let values = report::downsample(&report.equity, EQUITY_POINTS);
    let capital = report.settings.initial_capital;
    let (low, high) = values
        .iter()
        .fold((capital, capital), |(low, high), &value| {
            (low.min(value), high.max(value))
        });
    let (plot, hovered) = Plot::show(ui, EQUITY_HEIGHT, values.len(), low, high, capital);
    let end = values.last().copied().unwrap_or(capital);
    let tone = plot::tone(end, capital);
    plot.line(values.iter().copied(), tone);
    if let Some(step) = hovered {
        let value = values[step];
        let bar = report::sampled_index(report.equity.len(), EQUITY_POINTS, step);
        let mut reading = PineReportFormat::money(value);
        if capital > 0.0 {
            reading.push_str(&format!(
                "  {}",
                PineReportFormat::percent((value - capital) / capital, true)
            ));
        }
        plot.readout(step, &[(value, tone)], &[format!("Bar {bar}"), reading]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_even_out_their_rows() {
        let width = |columns: f32| MIN_COLUMN * columns + COLUMN_GAP * (columns - 1.0);
        assert_eq!(grid_columns(width(4.0), 4), 4);
        assert_eq!(grid_columns(width(3.0), 4), 2, "never three over one");
        assert_eq!(grid_columns(width(2.0), 4), 2);
        assert_eq!(grid_columns(width(1.5), 4), 1);
        assert_eq!(grid_columns(width(6.0), 3), 3, "no empty columns");
        assert_eq!(grid_columns(width(2.0), 3), 2);
        assert_eq!(grid_columns(10.0, 2), 1);
    }
}
