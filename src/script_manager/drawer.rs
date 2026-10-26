//! The preview pane's drawer: the always-visible tab bar (Inputs / Report / Problems / Logs) and the
//! tab content. Ported from `ScriptPreviewDrawer`, `ScriptPreviewDrawerBar` and
//! `PineDiagnosticsListView`.

use super::prefs::DrawerTab;
use super::tab_chrome::{self as chrome, BUTTON, card, hairline};
use crate::model::pine_alert::frequency_name;
use crate::pine::program::VisualOutput;
use crate::pine::state::AlertEvent;
use crate::pine::strategy::{StrategyReport, Trade};
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
use crate::ui::pine_logs_view::centered;
use crate::ui::pine_report::{self, PineReportFormat};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, Rect, RichText, Sense, Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

pub const BAR_HEIGHT: f32 = super::bottom_bar::HEIGHT;

impl DrawerTab {
    pub const ALL: [Self; 5] = [
        Self::Inputs,
        Self::Report,
        Self::Problems,
        Self::Logs,
        Self::Profile,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Inputs => "Inputs",
            Self::Report => "Report",
            Self::Problems => "Problems",
            Self::Logs => "Logs",
            Self::Profile => "Profile",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Inputs => Icon::Sliders,
            Self::Report => Icon::ChartUptrend,
            Self::Problems => Icon::Triangle,
            Self::Logs => Icon::DocText,
            Self::Profile => Icon::Gauge,
        }
    }
}

/// What the bar changed.
#[derive(Default)]
pub struct BarOutput {
    pub tab: Option<DrawerTab>,
    pub toggle: bool,
}

/// The bottom bar under the drawer: tabs, the status string, the open/close chevron.
pub fn bar(
    ui: &mut Ui,
    rect: Rect,
    tab: DrawerTab,
    open: bool,
    problems: &[Diagnostic],
    status: &str,
) -> BarOutput {
    let mut output = BarOutput::default();
    super::bottom_bar::paint(ui, rect);
    let mut x = rect.min.x + 8.0;
    let font = FontId::proportional(10.0);
    for candidate in DrawerTab::ALL {
        let selected = open && candidate == tab;
        let label = candidate.title();
        let galley =
            ui.painter()
                .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
        let badge = (candidate == DrawerTab::Problems && !problems.is_empty()).then(|| {
            let errors = problems
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Error);
            (problems.len().to_string(), errors)
        });
        let badge_width = badge
            .as_ref()
            .map_or(0.0, |(text, _)| text.len() as f32 * 6.0 + 15.0);
        let width = 8.0 + 12.0 + 5.0 + galley.size().x + badge_width + 8.0;
        let tab_rect = Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(width, 22.0));
        let response = ui.interact(tab_rect, Id::new(("drawer-tab", label)), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let color = if selected {
            theme::SYSTEM_ACCENT
        } else {
            theme::text_secondary()
        };
        let painter = ui.painter();
        if selected {
            painter.rect_filled(tab_rect, 6.0, theme::with_alpha(theme::SYSTEM_ACCENT, 0.15));
        } else if response.hovered() {
            painter.rect_filled(tab_rect, 6.0, theme::ink(10));
        }
        icons::paint(
            painter,
            Rect::from_center_size(
                pos2(tab_rect.min.x + 8.0 + 6.0, tab_rect.center().y),
                Vec2::splat(12.0),
            ),
            candidate.icon(),
            color,
        );
        painter.galley(
            pos2(
                tab_rect.min.x + 8.0 + 12.0 + 5.0,
                tab_rect.center().y - galley.size().y / 2.0,
            ),
            galley.clone(),
            color,
        );
        if let Some((text, errors)) = badge {
            let badge_rect = Rect::from_min_size(
                pos2(
                    tab_rect.min.x + 8.0 + 12.0 + 5.0 + galley.size().x + 5.0,
                    tab_rect.center().y - 7.5,
                ),
                vec2(text.len() as f32 * 6.0 + 10.0, 15.0),
            );
            let (fill, ink) = if errors {
                (theme::syntax::string(), Color32::WHITE)
            } else {
                (theme::YELLOW, Color32::BLACK)
            };
            painter.rect_filled(badge_rect, 7.5, fill);
            painter.text(
                badge_rect.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::monospace(9.0),
                ink,
            );
        }
        if response.clicked() {
            if selected {
                output.toggle = true;
            } else {
                output.tab = Some(candidate);
            }
        }
        x += width + 2.0;
    }

    // The chevron, then the status string filling what is left.
    let chevron = Rect::from_center_size(
        pos2(rect.max.x - 8.0 - 11.0, rect.center().y),
        Vec2::splat(22.0),
    );
    let response = ui
        .interact(chevron, Id::new("drawer-chevron"), Sense::click())
        .on_hover_text(if open { "Hide panel" } else { "Show panel" });
    if response.hovered() {
        ui.painter().rect_filled(chevron, 5.0, theme::ink(14));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chevron.center(), Vec2::splat(10.0)),
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronUp
        },
        theme::text_secondary(),
    );
    if response.clicked() {
        output.toggle = true;
    }
    let status_rect = Rect::from_min_max(
        pos2(x + 8.0, rect.min.y),
        pos2(chevron.min.x - 8.0, rect.max.y),
    );
    if status_rect.width() > 20.0 {
        let painter = ui.painter().with_clip_rect(status_rect);
        let galley = painter.layout_no_wrap(
            status.to_owned(),
            FontId::proportional(10.0),
            theme::text_secondary(),
        );
        // Tail truncation: the string is clipped at the left edge of the chevron.
        painter.galley(
            pos2(
                status_rect.max.x - galley.size().x.min(status_rect.width()),
                status_rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::text_secondary(),
        );
    }
    output
}

/// A centred "title / caption" message, like the drawer's empty states.
pub fn placeholder(ui: &mut Ui, title: &str, caption: &str) {
    centered(ui, |ui| {
        ui.label(kit::text(
            title,
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(2.0);
        ui.label(kit::text(
            caption,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
}

/// How the Copy button copies: only the errors, or everything when there are only warnings.
pub fn copy_text(diagnostics: &[Diagnostic]) -> String {
    let has_errors = diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error);
    sorted(diagnostics)
        .into_iter()
        .filter(|d| !has_errors || d.severity == DiagnosticSeverity::Error)
        .map(|d| {
            format!(
                "{} · {}:{}  {}",
                d.code, d.range.start.line, d.range.start.column, d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Errors first, then source order.
pub fn sorted(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    let mut list: Vec<&Diagnostic> = diagnostics.iter().collect();
    list.sort_by_key(|d| {
        (
            d.severity != DiagnosticSeverity::Error,
            d.range.start.offset,
        )
    });
    list
}

/// "3 errors", "1 warning", "2 errors · 1 warning", or "No problems".
pub fn problems_summary(diagnostics: &[Diagnostic]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let warnings = diagnostics.len() - errors;
    match (errors, warnings) {
        (0, 0) => "No problems".to_owned(),
        (errors, 0) => counted(errors, "error"),
        (0, warnings) => counted(warnings, "warning"),
        (errors, warnings) => format!(
            "{} \u{b7} {}",
            counted(errors, "error"),
            counted(warnings, "warning")
        ),
    }
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// The Problems tab: a toolbar saying how many there are, with the button that copies them, and
/// the diagnostics as rows of a card. Returns the diagnostic whose row was clicked.
pub fn problems(
    ui: &mut Ui,
    diagnostics: &[Diagnostic],
    copied_at: &mut Option<Instant>,
) -> Option<Diagnostic> {
    let has_errors = diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error);
    let just_copied = copied_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2));
    chrome::toolbar(ui, |ui, width| {
        let copy = !diagnostics.is_empty();
        let dot = if diagnostics.is_empty() {
            kit::GREEN
        } else if has_errors {
            theme::syntax::string()
        } else {
            theme::YELLOW
        };
        let field = chrome::field_width(width, usize::from(copy));
        chrome::state_field(ui, field, dot, &problems_summary(diagnostics), None);
        if copy {
            ui.add_space(6.0);
            let (icon, tip, color) = if just_copied {
                (Icon::Check, "Copied", theme::text_secondary())
            } else if has_errors {
                (Icon::CopyDoc, "Copy errors", theme::text_secondary())
            } else {
                (Icon::CopyDoc, "Copy problems", theme::text_secondary())
            };
            if icons::button(ui, icon, BUTTON, color, tip).clicked() && !just_copied {
                ui.ctx().copy_text(copy_text(diagnostics));
                *copied_at = Some(Instant::now());
            }
            if just_copied {
                ui.ctx().request_repaint_after(Duration::from_secs(2));
            }
        }
    });
    if diagnostics.is_empty() {
        placeholder(
            ui,
            "No problems",
            "Compile errors, warnings and runtime failures are listed here.",
        );
        return None;
    }
    let mut jump = None;
    chrome::scroll(ui, "problems-scroll", |ui| {
        card(ui, |ui| {
            for (index, diagnostic) in sorted(diagnostics).into_iter().enumerate() {
                if index > 0 {
                    hairline(ui);
                }
                if problem_row(ui, diagnostic).clicked() {
                    jump = Some(diagnostic.clone());
                }
            }
        });
    });
    jump
}

fn problem_row(ui: &mut Ui, diagnostic: &Diagnostic) -> egui::Response {
    let (icon, tint) = match diagnostic.severity {
        DiagnosticSeverity::Error => (Icon::XmarkOctagonFill, theme::syntax::string()),
        DiagnosticSeverity::Warning => (Icon::TriangleFill, theme::YELLOW),
    };
    let line = diagnostic.range.start.line;
    let response = list_row(
        ui,
        &ListRow {
            icon,
            tint,
            code: Some(&diagnostic.code),
            text: &diagnostic.message,
            text_color: theme::text_primary(),
            trail: &format!("{line}:{}", diagnostic.range.start.column),
            clickable: true,
        },
    );
    crate::ui::accessibility::label(
        response,
        format!("{}, line {line}: {}", diagnostic.code, diagnostic.message),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .on_hover_text(format!("Go to line {line}"))
}

/// What a card's row says: a status icon, an optional code in a capsule, the words, and a figure at
/// the right edge.
struct ListRow<'a> {
    icon: Icon,
    tint: Color32,
    code: Option<&'a str>,
    text: &'a str,
    text_color: Color32,
    trail: &'a str,
    clickable: bool,
}

/// One row of a card, as tall as its words. The words wrap; where the width would squeeze them,
/// they drop under the code and the figure instead.
fn list_row(ui: &mut Ui, row: &ListRow<'_>) -> egui::Response {
    const PAD_X: f32 = 12.0;
    const PAD_Y: f32 = 7.0;
    const LINE: f32 = 16.0;
    const INDENT: f32 = 20.0;
    const SQUEEZED: f32 = 140.0;

    let width = ui.available_width();
    let painter = ui.painter().clone();
    let mono = FontId::monospace(10.0);
    let trail = painter.layout_no_wrap(row.trail.to_owned(), mono.clone(), theme::text_tertiary());
    let code = row
        .code
        .map(|code| painter.layout_no_wrap(code.to_owned(), mono, theme::text_secondary()));
    let beside_x = PAD_X + INDENT + code.as_ref().map_or(0.0, |code| code.size().x + 10.0 + 8.0);
    let beside = width - beside_x - PAD_X - trail.size().x - 10.0;
    let stacked = beside < SQUEEZED;
    let (words_x, wrap) = if stacked {
        (PAD_X + INDENT, width - PAD_X * 2.0 - INDENT)
    } else {
        (beside_x, beside)
    };
    let words = painter.layout(
        row.text.to_owned(),
        FontId::proportional(12.0),
        row.text_color,
        wrap.max(40.0),
    );
    let height = if stacked {
        PAD_Y * 2.0 + LINE + 3.0 + words.size().y
    } else {
        PAD_Y * 2.0 + words.size().y.max(LINE)
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, height),
        if row.clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if row.clickable && response.hovered() {
        painter.rect_filled(rect, 6.0, theme::ink(10));
    }
    let top = rect.top() + PAD_Y;
    let mid = top + LINE / 2.0;
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(rect.left() + PAD_X + 6.0, mid), Vec2::splat(12.0)),
        row.icon,
        row.tint,
    );
    if let Some(code) = code {
        let capsule = Rect::from_min_size(
            pos2(rect.left() + PAD_X + INDENT, mid - 8.0),
            vec2(code.size().x + 10.0, 16.0),
        );
        painter.rect_filled(capsule, 5.0, theme::ink(10));
        painter.galley(
            pos2(capsule.left() + 5.0, mid - code.size().y / 2.0),
            code,
            theme::text_secondary(),
        );
    }
    painter.galley(
        pos2(
            rect.right() - PAD_X - trail.size().x,
            mid - trail.size().y / 2.0,
        ),
        trail,
        theme::text_tertiary(),
    );
    let words_top = if stacked { top + LINE + 3.0 } else { top + 1.0 };
    painter.galley(
        pos2(rect.left() + words_x, words_top),
        words,
        row.text_color,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{DiagnosticCategory, SourcePosition, SourceRange};

    fn diag(code: &str, severity: DiagnosticSeverity, line: usize, offset: usize) -> Diagnostic {
        Diagnostic {
            code: code.into(),
            severity,
            category: DiagnosticCategory::Semantic,
            message: format!("{code} message"),
            range: SourceRange::new(
                SourcePosition::new(line, 4, offset),
                SourcePosition::new(line, 5, offset + 1),
            ),
        }
    }

    #[test]
    fn rows_sort_errors_first_then_source_order() {
        use DiagnosticSeverity::{Error, Warning};
        let list = [
            diag("W1", Warning, 1, 5),
            diag("E2", Error, 9, 90),
            diag("E1", Error, 3, 30),
        ];
        let order: Vec<&str> = sorted(&list).iter().map(|d| d.code.as_str()).collect();
        assert_eq!(order, ["E1", "E2", "W1"]);
    }

    #[test]
    fn copy_prefers_errors_and_uses_the_swift_line_format() {
        use DiagnosticSeverity::{Error, Warning};
        let mixed = [diag("W1", Warning, 1, 5), diag("E1", Error, 3, 30)];
        assert_eq!(copy_text(&mixed), "E1 · 3:4  E1 message");
        let warnings = [diag("W1", Warning, 1, 5), diag("W2", Warning, 2, 9)];
        assert_eq!(
            copy_text(&warnings),
            "W1 · 1:4  W1 message\nW2 · 2:4  W2 message"
        );
    }
}

// MARK: Report

const ALERT_LIMIT: usize = 100;

/// "Jan 5, 3:07 PM"-style stamp in the system time zone: `MMM d, h:mm`.
pub fn alert_time(ms: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let local = ms + crate::service::timezone::local_utc_offset_secs(ms) * 1000;
    let secs = local.div_euclid(1000);
    let (_, month, day) = super::inputs::civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let clock = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!(
        "{} {day}, {clock}:{:02} {}",
        MONTHS[(month as usize).clamp(1, 12) - 1],
        rem % 3600 / 60,
        if hour < 12 { "AM" } else { "PM" }
    )
}

/// (live, historical) alert counts.
pub fn alert_summary(alerts: &[AlertEvent]) -> (usize, usize) {
    let live = alerts.iter().filter(|a| a.is_realtime).count();
    (live, alerts.len() - live)
}

/// The latest `ALERT_LIMIT` alerts, newest first.
pub fn visible_alerts(alerts: &[AlertEvent]) -> Vec<&AlertEvent> {
    alerts.iter().rev().take(ALERT_LIMIT).collect()
}

/// "Strategy \u{b7} 24 trades", "Indicator \u{b7} 3 alerts", "Strategy \u{b7} 24 trades \u{b7} 3 alerts"
/// or "No report".
pub fn report_summary(strategy: Option<&StrategyReport>, alerts: usize) -> String {
    let mut parts = Vec::new();
    match strategy {
        Some(report) => parts.push(format!(
            "Strategy \u{b7} {}",
            counted(report.trades.len(), "trade")
        )),
        None if alerts > 0 => parts.push("Indicator".to_owned()),
        None => {}
    }
    if alerts > 0 {
        parts.push(counted(alerts, "alert"));
    }
    if parts.is_empty() {
        "No report".to_owned()
    } else {
        parts.join(" \u{b7} ")
    }
}

/// The Report tab: a toolbar with what the script reported, and under it the strategy's figures,
/// equity curve and trades, and the alerts the script raised, as cards. Where the width allows, the
/// trades stand beside the figures.
pub fn report(ui: &mut Ui, visual: Option<&VisualOutput>) {
    let strategy = visual.and_then(|v| v.strategy.as_ref());
    let alerts: &[AlertEvent] = visual.map_or(&[], |v| v.alerts.as_slice());
    chrome::toolbar(ui, |ui, width| {
        let neutral = theme::text_secondary();
        let (dot, trailing) = match strategy {
            Some(report) => {
                let net = report.net_profit();
                let tint = pine_report::tint(net, neutral);
                let text = format!(
                    "{}  {}",
                    PineReportFormat::signed_money(net),
                    PineReportFormat::percent(report.return_fraction(), true)
                );
                (tint, Some((text, tint)))
            }
            None if !alerts.is_empty() => (kit::ACCENT, None),
            None => (neutral, None),
        };
        chrome::state_field(
            ui,
            chrome::field_width(width, 0),
            dot,
            &report_summary(strategy, alerts.len()),
            trailing.as_ref().map(|(text, tint)| (text.as_str(), *tint)),
        );
    });
    if strategy.is_none() && alerts.is_empty() {
        placeholder(
            ui,
            "No report",
            "Strategies report their trades and equity curve here. Indicators show alerts they raise.",
        );
        return;
    }
    chrome::scroll(ui, "script-report-scroll", |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match strategy {
            Some(report) if !report.trades.is_empty() && chrome::wide(ui) => {
                chrome::columns(
                    ui,
                    0.5,
                    |ui| {
                        figures_card(ui, report);
                        if !alerts.is_empty() {
                            ui.add_space(16.0);
                            alerts_card(ui, alerts);
                        }
                    },
                    |ui| trades_card(ui, report),
                );
            }
            Some(report) => {
                figures_card(ui, report);
                if !report.trades.is_empty() {
                    ui.add_space(16.0);
                    trades_card(ui, report);
                }
                if !alerts.is_empty() {
                    ui.add_space(16.0);
                    alerts_card(ui, alerts);
                }
            }
            None => alerts_card(ui, alerts),
        }
    });
}

/// The strategy's headline figures and its equity curve.
fn figures_card(ui: &mut Ui, report: &StrategyReport) {
    if let Some(notice) = pine_report::synthetic_fill_notice(report.chart) {
        chrome::banner(ui, &notice);
    }
    let padded = egui::Margin::symmetric(12, 10);
    chrome::caption(ui, "Figures");
    card(ui, |ui| {
        egui::Frame::new().inner_margin(padded).show(ui, |ui| {
            ui.set_width(ui.available_width());
            pine_report::metric_grid(ui, report, theme::text_primary());
        });
    });
    if report.equity.len() > 1 {
        ui.add_space(16.0);
        chrome::caption(ui, "Equity");
        card(ui, |ui| {
            egui::Frame::new().inner_margin(padded).show(ui, |ui| {
                ui.set_width(ui.available_width());
                pine_report::equity_curve(
                    ui,
                    &report.equity,
                    report.settings.initial_capital,
                    72.0,
                    false,
                );
            });
        });
    }
}

const TRADE_LIMIT: usize = 100;

/// The closed trades, newest first.
fn trades_card(ui: &mut Ui, report: &StrategyReport) {
    chrome::caption(ui, &format!("Trades ({})", report.trades.len()));
    card(ui, |ui| {
        for (index, trade) in report.trades.iter().rev().take(TRADE_LIMIT).enumerate() {
            if index > 0 {
                hairline(ui);
            }
            trade_row(ui, trade);
        }
    });
    if report.trades.len() > TRADE_LIMIT {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "Showing the latest {TRADE_LIMIT} of {}",
                report.trades.len()
            ))
            .size(9.0)
            .color(theme::text_tertiary()),
        );
    }
}

/// One trade: its number and side, where it went in and out, and what it made.
fn trade_row(ui: &mut Ui, trade: &Trade) {
    const HEIGHT: f32 = 28.0;
    const PAD_X: f32 = 12.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    let mid = rect.center().y;
    let number = painter.layout_no_wrap(
        format!("#{}", trade.id),
        FontId::monospace(10.0),
        theme::text_tertiary(),
    );
    painter.galley(
        pos2(rect.left() + PAD_X, mid - number.size().y / 2.0),
        number,
        theme::text_tertiary(),
    );
    let (side, tint) = if trade.is_long {
        ("Long", kit::ACCENT)
    } else {
        ("Short", kit::ORANGE)
    };
    let capsule_x = rect.left() + PAD_X + 40.0;
    let galley = painter.layout_no_wrap(side.to_owned(), FontId::proportional(9.0), tint);
    let capsule = Rect::from_min_size(
        pos2(capsule_x, mid - 7.5),
        vec2(galley.size().x + 14.0, 15.0),
    );
    painter.rect_filled(capsule, 8.0, theme::with_alpha(tint, 0.14));
    painter.galley(
        pos2(capsule.left() + 7.0, mid - galley.size().y / 2.0),
        galley,
        tint,
    );
    let profit = painter.layout_no_wrap(
        PineReportFormat::signed_money(trade.profit),
        FontId::monospace(10.5),
        pine_report::tint(trade.profit, theme::text_primary()),
    );
    let profit_x = rect.right() - PAD_X - profit.size().x;
    painter.galley(
        pos2(profit_x, mid - profit.size().y / 2.0),
        profit,
        Color32::PLACEHOLDER,
    );
    let prices_x = capsule.right() + 10.0;
    let room = profit_x - 10.0 - prices_x;
    if room > 30.0 {
        let prices = kit::galley(
            ui,
            &format!(
                "{} \u{2192} {}",
                PineReportFormat::price(trade.entry_price),
                PineReportFormat::price(trade.exit_price)
            ),
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
            room,
        );
        painter.galley(
            pos2(prices_x, mid - prices.size().y / 2.0),
            prices,
            theme::text_secondary(),
        );
    }
    response.on_hover_text(format!(
        "{} \u{2192} {} \u{b7} qty {}",
        trade.entry_id,
        trade.exit_id,
        PineReportFormat::price(trade.quantity)
    ));
}

/// The alerts the script raised, newest first, with how many were live.
fn alerts_card(ui: &mut Ui, alerts: &[AlertEvent]) {
    let (live, historical) = alert_summary(alerts);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            "Alerts",
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        if live > 0 {
            chrome::chip(ui, &format!("{live} live"), theme::syntax::variable());
        }
        if historical > 0 {
            chrome::chip(
                ui,
                &format!("{historical} historical"),
                theme::text_secondary(),
            );
        }
    });
    ui.add_space(6.0);
    card(ui, |ui| {
        for (index, alert) in visible_alerts(alerts).into_iter().enumerate() {
            if index > 0 {
                hairline(ui);
            }
            let (icon, tint) = if alert.is_realtime {
                (Icon::BellFill, theme::SYSTEM_ACCENT)
            } else {
                (Icon::Clock, theme::text_secondary())
            };
            let (text, color) = if alert.message.is_empty() {
                ("No message", theme::text_secondary())
            } else {
                (alert.message.as_str(), theme::text_primary())
            };
            list_row(
                ui,
                &ListRow {
                    icon,
                    tint,
                    code: None,
                    text,
                    text_color: color,
                    trail: &alert_time(alert.time),
                    clickable: false,
                },
            )
            .on_hover_text(format!(
                "{} \u{b7} {}",
                if alert.is_realtime {
                    "Live bar, notifies"
                } else {
                    "Historical bar, never notifies"
                },
                frequency_name(alert.frequency)
            ));
        }
    });
    if alerts.len() > ALERT_LIMIT {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "Showing the latest {ALERT_LIMIT} of {}",
                alerts.len()
            ))
            .size(9.0)
            .color(theme::text_tertiary()),
        );
    }
}

/// The strategy report and the alerts box, without a scroll area of their own (the chart settings
/// show one per applied strategy inside their own page).
pub fn report_content(ui: &mut Ui, visual: Option<&VisualOutput>) {
    let strategy = visual.and_then(|v| v.strategy.as_ref());
    let alerts: &[AlertEvent] = visual.map_or(&[], |v| v.alerts.as_slice());
    ui.spacing_mut().item_spacing.y = 12.0;
    pine_report::show(ui, strategy);
    if !alerts.is_empty() {
        alerts_section(ui, alerts);
    }
}

/// The alerts box: header with live/historical chips, collapsed by default (a log, not a
/// problem), newest first when open.
fn alerts_section(ui: &mut Ui, alerts: &[AlertEvent]) {
    let (live, historical) = alert_summary(alerts);
    pine_report::report_box(ui, |ui| {
        let id = ui.make_persistent_id("pine-alerts-section");
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::BellFill, 12.0, theme::SYSTEM_ACCENT);
                ui.label(
                    RichText::new("Alerts")
                        .size(10.0)
                        .variation("wght", crate::ui::kit::SEMIBOLD)
                        .color(theme::text_primary()),
                );
                if live > 0 {
                    chrome::chip(ui, &format!("{live} live"), theme::syntax::variable());
                }
                if historical > 0 {
                    chrome::chip(
                        ui,
                        &format!("{historical} historical"),
                        theme::text_secondary(),
                    );
                }
            })
            .body(|ui| {
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for alert in visible_alerts(alerts) {
                            alert_row(ui, alert);
                        }
                    });
                if alerts.len() > ALERT_LIMIT {
                    ui.label(
                        RichText::new(format!(
                            "Showing the latest {ALERT_LIMIT} of {}",
                            alerts.len()
                        ))
                        .size(9.0)
                        .color(theme::text_tertiary()),
                    );
                }
            });
    });
}

fn alert_row(ui: &mut Ui, alert: &AlertEvent) {
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (icon, tint) = if alert.is_realtime {
            (Icon::BellFill, theme::SYSTEM_ACCENT)
        } else {
            (Icon::Clock, theme::text_secondary())
        };
        let (slot, _) = ui.allocate_exact_size(vec2(14.0, 12.0), Sense::hover());
        icons::paint(
            ui.painter(),
            Rect::from_center_size(slot.center(), Vec2::splat(10.0)),
            icon,
            tint,
        );
        let (text, color) = if alert.message.is_empty() {
            ("No message", theme::text_secondary())
        } else {
            (alert.message.as_str(), theme::text_primary())
        };
        ui.label(RichText::new(text).size(10.0).color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(alert_time(alert.time))
                    .size(9.0)
                    .monospace()
                    .color(theme::text_secondary()),
            );
        });
    });
    row.response.on_hover_text(format!(
        "{} · {}",
        if alert.is_realtime {
            "Live bar, notifies"
        } else {
            "Historical bar, never notifies"
        },
        frequency_name(alert.frequency)
    ));
}

#[cfg(test)]
mod report_tests {
    use super::*;
    use crate::pine::alert_frequency::AlertFrequency;

    fn alert(id: usize, live: bool) -> AlertEvent {
        AlertEvent {
            id,
            site: 0,
            bar: id,
            time: 0,
            message: format!("m{id}"),
            frequency: AlertFrequency::OncePerBar,
            is_realtime: live,
            is_confirmed: true,
        }
    }

    #[test]
    fn summary_counts_live_and_historical() {
        let alerts = [alert(1, true), alert(2, false), alert(3, false)];
        assert_eq!(alert_summary(&alerts), (1, 2));
    }

    #[test]
    fn only_the_latest_hundred_show_newest_first() {
        let alerts: Vec<AlertEvent> = (0..150).map(|i| alert(i, false)).collect();
        let shown = visible_alerts(&alerts);
        assert_eq!(shown.len(), 100);
        assert_eq!(shown[0].id, 149);
        assert_eq!(shown[99].id, 50);
    }

    #[test]
    fn times_read_like_the_swift_format() {
        // 2024-03-09 14:30 UTC (tests run in UTC).
        let ms = super::super::inputs::parse_time("2024-03-09 14:30").unwrap();
        assert_eq!(alert_time(ms), "Mar 9, 2:30 PM");
        assert_eq!(alert_time(0), "Jan 1, 12:00 AM");
    }
}
