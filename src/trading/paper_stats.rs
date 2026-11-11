//! The paper account's numbers in the words every venue's strip uses: the account strip's hero,
//! its cells, and the metric strings the paper panel shares. Pure: plain values in, text out.

use crate::model::paper::format::{
    self as fmt, BufferLevel, PnlTone, margin_buffer_level, pnl_tone,
};
use crate::model::paper::{PaperAccountMetrics, PaperCurrency};
use crate::trading::model::{AccountStats, Hero, HeroNote, StatCell, StatTone, Trend};

/// "Unrealized +$12.00 (+1.20%) open" and the other account figures, as plain strings.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricCell {
    pub title: &'static str,
    pub value: String,
    pub tone: Option<PnlTone>,
    pub buffer: Option<BufferLevel>,
    pub help: String,
}

#[cfg(test)]
/// Equity's sub-line: the open P&L and its share of the balance, led by a trend arrow.
pub fn unrealized_line(metrics: &PaperAccountMetrics, currency: PaperCurrency) -> String {
    let pnl = metrics.unrealized_pnl;
    let arrow = if pnl > 0.0 {
        "↗"
    } else if pnl < 0.0 {
        "↘"
    } else {
        "–"
    };
    format!("{arrow} {} open", unrealized_text(metrics, currency))
}

/// "+$12.00 (+1.20%)": the open P&L and its share of the balance, without the arrow or "open".
pub fn unrealized_text(metrics: &PaperAccountMetrics, currency: PaperCurrency) -> String {
    // Flat reads "0.00%", never "-0.00%".
    let pnl = if metrics.unrealized_pnl == 0.0 {
        0.0
    } else {
        metrics.unrealized_pnl
    };
    let ratio = (metrics.balance > 0.0).then(|| pnl / metrics.balance);
    let mut text = fmt::signed_money(pnl, currency);
    if let Some(ratio) = ratio {
        text.push_str(&format!(" ({})", fmt::signed_percent(ratio)));
    }
    text
}

pub fn metric_cells(
    metrics: Option<&PaperAccountMetrics>,
    currency: PaperCurrency,
) -> Vec<MetricCell> {
    let Some(metrics) = metrics else {
        return [
            "Balance",
            "Realized P&L",
            "Available",
            "Margin used",
            "Margin buffer",
        ]
        .into_iter()
        .map(|title| MetricCell {
            title,
            value: "—".to_owned(),
            tone: None,
            buffer: None,
            help: title.to_owned(),
        })
        .collect();
    };
    let money = |value| fmt::money(value, currency);
    vec![
        MetricCell {
            title: "Balance",
            value: money(metrics.balance),
            tone: None,
            buffer: None,
            help: "Balance".to_owned(),
        },
        MetricCell {
            title: "Realized P&L",
            value: fmt::signed_money(metrics.realized_pnl, currency),
            tone: Some(pnl_tone(metrics.realized_pnl)),
            buffer: None,
            help: "Realized P&L".to_owned(),
        },
        MetricCell {
            title: "Available",
            value: money(metrics.available_funds),
            tone: None,
            buffer: None,
            help: "Available".to_owned(),
        },
        MetricCell {
            title: "Margin used",
            value: money(metrics.position_margin + metrics.orders_margin),
            tone: None,
            buffer: None,
            help: format!(
                "Positions {} · Orders {}",
                money(metrics.position_margin),
                money(metrics.orders_margin)
            ),
        },
        MetricCell {
            title: "Margin buffer",
            value: fmt::percent(metrics.margin_buffer),
            tone: None,
            buffer: Some(margin_buffer_level(metrics.margin_buffer)),
            help: "Available funds as a share of equity".to_owned(),
        },
    ]
}

fn stat_tone(tone: PnlTone) -> StatTone {
    match tone {
        PnlTone::Profit => StatTone::Profit,
        PnlTone::Loss => StatTone::Loss,
        PnlTone::Flat => StatTone::Neutral,
    }
}

/// The account's numbers in the words every venue's strip uses: this is the paper account's.
pub fn stats(metrics: Option<&PaperAccountMetrics>, currency: PaperCurrency) -> AccountStats {
    let note = metrics.map(|m| {
        let trend = if m.unrealized_pnl > 0.0 {
            Trend::Up
        } else if m.unrealized_pnl < 0.0 {
            Trend::Down
        } else {
            Trend::Flat
        };
        HeroNote {
            trend,
            text: unrealized_text(m, currency),
            tone: stat_tone(pnl_tone(m.unrealized_pnl)),
            label: "open".to_owned(),
        }
    });
    AccountStats {
        hero: Hero {
            title: "Equity".to_owned(),
            value: metrics.map_or_else(|| "—".to_owned(), |m| fmt::money(m.equity, currency)),
            help: None,
            note,
        },
        cells: metric_cells(metrics, currency)
            .into_iter()
            .map(|cell| StatCell {
                title: cell.title.to_owned(),
                value: cell.value,
                tone: match (cell.tone, cell.buffer) {
                    (Some(tone), _) => stat_tone(tone),
                    (None, Some(BufferLevel::Critical)) => StatTone::Critical,
                    (None, Some(BufferLevel::Thin)) => StatTone::Thin,
                    _ => StatTone::Neutral,
                },
                help: cell.help,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> PaperAccountMetrics {
        PaperAccountMetrics {
            balance: 10_000.0,
            equity: 10_120.0,
            realized_pnl: 50.0,
            unrealized_pnl: 120.0,
            position_margin: 800.0,
            orders_margin: 200.0,
            available_funds: 9_120.0,
            margin_buffer: 0.9,
        }
    }

    #[test]
    fn the_strip_says_what_the_paper_panel_always_said() {
        let usd = PaperCurrency::Usd;
        let strip = stats(Some(&metrics()), usd);
        assert_eq!(strip.hero.title, "Equity");
        assert_eq!(strip.hero.value, "$10,120.00");
        let note = strip.hero.note.as_ref().expect("a note under the figure");
        assert_eq!(note.text, "+$120.00 (+1.20%)");
        assert_eq!((note.trend, note.tone), (Trend::Up, StatTone::Profit));
        assert_eq!(note.label, "open");
        let cells: Vec<(&str, &str, StatTone)> = strip
            .cells
            .iter()
            .map(|c| (c.title.as_str(), c.value.as_str(), c.tone))
            .collect();
        assert_eq!(
            cells,
            [
                ("Balance", "$10,000.00", StatTone::Neutral),
                ("Realized P&L", "+$50.00", StatTone::Profit),
                ("Available", "$9,120.00", StatTone::Neutral),
                ("Margin used", "$1,000.00", StatTone::Neutral),
                ("Margin buffer", "90.00%", StatTone::Neutral),
            ]
        );
        // Without an account: dashes, no note.
        let empty = stats(None, usd);
        assert_eq!(empty.hero.value, "—");
        assert!(empty.hero.note.is_none());
        assert!(empty.cells.iter().all(|c| c.value == "—"));
        // A thin buffer and a loss.
        let thin = PaperAccountMetrics {
            margin_buffer: 0.2,
            unrealized_pnl: -50.0,
            ..metrics()
        };
        let strip = stats(Some(&thin), usd);
        assert_eq!(strip.cells[4].tone, StatTone::Thin);
        assert_eq!(strip.hero.note.unwrap().trend, Trend::Down);
    }

    #[test]
    fn the_equity_sub_line_shows_open_pnl_and_its_share_of_the_balance() {
        let usd = PaperCurrency::Usd;
        assert_eq!(unrealized_line(&metrics(), usd), "↗ +$120.00 (+1.20%) open");
        let losing = PaperAccountMetrics {
            unrealized_pnl: -50.0,
            ..metrics()
        };
        assert_eq!(unrealized_line(&losing, usd), "↘ -$50.00 (-0.50%) open");
        let flat = PaperAccountMetrics {
            unrealized_pnl: 0.0,
            ..metrics()
        };
        assert_eq!(unrealized_line(&flat, usd), "– $0.00 (0.00%) open");
        let empty_balance = PaperAccountMetrics {
            balance: 0.0,
            ..metrics()
        };
        assert_eq!(
            unrealized_line(&empty_balance, usd),
            "↗ +$120.00 open",
            "no ratio without a balance"
        );
    }

    #[test]
    fn a_flat_account_shows_zero_without_a_minus_sign() {
        let mut flat = metrics();
        flat.unrealized_pnl = -0.0;
        let text = unrealized_text(&flat, PaperCurrency::Usd);
        assert!(!text.contains('-'), "{text}");
    }

    #[test]
    fn the_metric_cells_name_what_margin_is_made_of() {
        let cells = metric_cells(Some(&metrics()), PaperCurrency::Usd);
        let titles: Vec<&str> = cells.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            [
                "Balance",
                "Realized P&L",
                "Available",
                "Margin used",
                "Margin buffer"
            ]
        );
        assert_eq!(cells[0].value, "$10,000.00");
        assert_eq!(cells[1].value, "+$50.00");
        assert_eq!(cells[1].tone, Some(PnlTone::Profit));
        assert_eq!(cells[3].value, "$1,000.00");
        assert_eq!(cells[3].help, "Positions $800.00 · Orders $200.00");
        assert_eq!(cells[4].value, "90.00%");
        assert_eq!(cells[4].buffer, Some(BufferLevel::Normal));
        let thin = PaperAccountMetrics {
            margin_buffer: 0.2,
            ..metrics()
        };
        assert_eq!(
            metric_cells(Some(&thin), PaperCurrency::Usd)[4].buffer,
            Some(BufferLevel::Thin)
        );
    }

    #[test]
    fn without_an_account_every_cell_is_a_dash() {
        let cells = metric_cells(None, PaperCurrency::Usd);
        assert_eq!(cells.len(), 5);
        assert!(
            cells
                .iter()
                .all(|c| c.value == "—" && c.tone.is_none() && c.buffer.is_none())
        );
    }
}
