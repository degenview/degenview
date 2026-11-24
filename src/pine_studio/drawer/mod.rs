//! The preview pane's drawer: the always-visible tab bar (Inputs / Report / Problems / Logs) and the
//! tab content. Ported from `ScriptPreviewDrawer`, `ScriptPreviewDrawerBar` and
//! `PineDiagnosticsListView`.

pub(in crate::pine_studio) mod bottom_bar;
mod figures;
pub(in crate::pine_studio) mod intro;
pub(in crate::pine_studio) mod logs;
mod plot;
pub(in crate::pine_studio) mod problems;
mod simulation;
pub(in crate::pine_studio) mod tab_chrome;
#[cfg(test)]
pub(in crate::pine_studio) mod tabs_tests;
mod trades;

use super::library::prefs::DrawerTab;
use crate::i18n::{tr, tr_n};
use crate::model::alert::pine::frequency_name;
use crate::pine::compiler::program::VisualOutput;
use crate::pine::runtime::state::AlertEvent;
use crate::pine::strategy::StrategyReport;
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::pine::report::{self, PineReportFormat};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, Rect, RichText, Sense, Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};
use tab_chrome::{self as chrome, BUTTON, card, hairline};

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
            Self::Inputs => tr!("Inputs"),
            Self::Report => tr!("Report"),
            Self::Problems => tr!("Problems"),
            Self::Logs => tr!("Logs"),
            Self::Profile => tr!("Profile"),
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

/// The side of the busy indicator before the status, and the gap after it.
const STATUS_INDICATOR: f32 = 10.0;
const STATUS_INDICATOR_GAP: f32 = 5.0;

/// The bottom bar under the drawer: tabs, the status string (with the candle indicator before it
/// while `busy`), the open/close chevron.
pub fn bar(
    ui: &mut Ui,
    rect: Rect,
    tab: DrawerTab,
    open: bool,
    problems: &[Diagnostic],
    status: &str,
    busy: bool,
) -> BarOutput {
    let mut output = BarOutput::default();
    bottom_bar::paint(ui, rect);
    let mut x = rect.min.x + 8.0;
    let font = FontId::proportional(10.0);
    let badge_of = |candidate: DrawerTab| {
        (candidate == DrawerTab::Problems && !problems.is_empty()).then(|| {
            let errors = problems
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Error);
            (problems.len().to_string(), errors)
        })
    };
    let galleys: Vec<_> = DrawerTab::ALL
        .iter()
        .map(|candidate| {
            ui.painter().layout_no_wrap(
                candidate.title().to_owned(),
                font.clone(),
                Color32::PLACEHOLDER,
            )
        })
        .collect();
    let badge_width_of = |candidate: DrawerTab| {
        badge_of(candidate).map_or(0.0, |(text, _)| text.len() as f32 * 6.0 + 15.0)
    };
    // The room for the tabs: up to the chevron. When their names do not all fit (a narrow window
    // or a longer language), only the open tab keeps its name and the others are their icons.
    let room = rect.max.x - 8.0 - 22.0 - 8.0 - x;
    let named: f32 = DrawerTab::ALL
        .iter()
        .zip(&galleys)
        .map(|(candidate, galley)| {
            8.0 + 12.0 + 5.0 + galley.size().x + badge_width_of(*candidate) + 8.0 + 2.0
        })
        .sum();
    let compact = named > room;
    for (candidate, galley) in DrawerTab::ALL.into_iter().zip(galleys) {
        let selected = open && candidate == tab;
        let named = !compact || selected;
        let label_width = if named { 5.0 + galley.size().x } else { 0.0 };
        let badge = badge_of(candidate);
        let badge_width = badge_width_of(candidate);
        let width = 8.0 + 12.0 + label_width + badge_width + 8.0;
        let tab_rect = Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(width, 22.0));
        let response = ui.interact(
            tab_rect,
            Id::new(("drawer-tab", candidate as u8)),
            Sense::click(),
        );
        // Without its name the tab says it on pointing, and to a screen reader.
        let response = if named {
            response
        } else {
            response.on_hover_text(candidate.title())
        };
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let color = if selected {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let painter = ui.painter();
        if selected {
            painter.rect_filled(tab_rect, 6.0, theme::selected_tint(false));
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
        if named {
            painter.galley(
                pos2(
                    tab_rect.min.x + 8.0 + 12.0 + 5.0,
                    tab_rect.center().y - galley.size().y / 2.0,
                ),
                galley.clone(),
                color,
            );
        }
        if let Some((text, errors)) = badge {
            let badge_rect = Rect::from_min_size(
                pos2(
                    tab_rect.min.x + 8.0 + 12.0 + label_width + 5.0,
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
        .on_hover_text(if open {
            tr!("Hide panel")
        } else {
            tr!("Show panel")
        });
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
        let text_left = status_rect.max.x - galley.size().x.min(status_rect.width());
        let indicator_left = text_left - STATUS_INDICATOR_GAP - STATUS_INDICATOR;
        if busy && indicator_left >= status_rect.min.x {
            LoadingIndicator::new().size(STATUS_INDICATOR).paint_at(
                ui,
                Rect::from_center_size(
                    pos2(
                        indicator_left + STATUS_INDICATOR / 2.0,
                        status_rect.center().y,
                    ),
                    Vec2::splat(STATUS_INDICATOR),
                ),
            );
        }
        painter.galley(
            pos2(text_left, status_rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_secondary(),
        );
    }
    output
}

/// How the Copy button copies: every problem, errors first, one line each.
pub fn copy_text(diagnostics: &[Diagnostic]) -> String {
    sorted(diagnostics)
        .into_iter()
        .map(problem_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// One problem as a line of text: `E1 · 3:4  message`.
pub fn problem_line(diagnostic: &Diagnostic) -> String {
    format!(
        "{} · {}:{}  {}",
        diagnostic.code,
        diagnostic.range.start.line,
        diagnostic.range.start.column,
        diagnostic.message
    )
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
        (0, 0) => tr!("No problems").to_owned(),
        (errors, 0) => tr_n!(errors, "{count} error", "{count} errors"),
        (0, warnings) => tr_n!(warnings, "{count} warning", "{count} warnings"),
        (errors, warnings) => format!(
            "{} \u{b7} {}",
            tr_n!(errors, "{count} error", "{count} errors"),
            tr_n!(warnings, "{count} warning", "{count} warnings")
        ),
    }
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
            let (icon, tip) = if just_copied {
                (Icon::Check, tr!("Copied"))
            } else {
                (Icon::CopyDoc, tr!("Copy all problems"))
            };
            let color = theme::text_secondary();
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
        intro::problems(ui);
        return None;
    }
    let mut jump = None;
    chrome::scroll(ui, "problems-scroll", |ui| {
        card(ui, |ui| {
            for (index, diagnostic) in sorted(diagnostics).into_iter().enumerate() {
                if index > 0 {
                    hairline(ui);
                }
                let response = problem_row(ui, diagnostic);
                problem_menu(ui, &response, diagnostic);
                if response.clicked() {
                    jump = Some(diagnostic.clone());
                }
            }
        });
    });
    jump
}

/// The right-click menu of a problem row: copy it whole, or only its message.
fn problem_menu(ui: &Ui, response: &egui::Response, diagnostic: &Diagnostic) {
    kit::fitted_context_menu(
        ui,
        response,
        &[(tr!("Copy problem"), None), (tr!("Copy message"), None)],
        |ui| {
            if kit::menu_item(ui, Icon::CopyDoc, tr!("Copy problem"), true, false) {
                ui.ctx().copy_text(problem_line(diagnostic));
                ui.close();
            }
            if kit::menu_item(ui, Icon::CopyDoc, tr!("Copy message"), true, false) {
                ui.ctx().copy_text(diagnostic.message.clone());
                ui.close();
            }
        },
    );
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
        tr!(
            "{code}, line {line}: {message}",
            code = diagnostic.code,
            line = line,
            message = diagnostic.message
        ),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .on_hover_text(tr!("Go to line {line}", line = line))
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
    use crate::pine::syntax::token::{DiagnosticCategory, SourcePosition, SourceRange};

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
    fn copy_takes_every_problem_errors_first_in_the_line_format() {
        use DiagnosticSeverity::{Error, Warning};
        let list = [
            diag("W1", Warning, 1, 5),
            diag("E2", Error, 9, 90),
            diag("E1", Error, 3, 30),
        ];
        assert_eq!(
            copy_text(&list),
            "E1 · 3:4  E1 message\nE2 · 9:4  E2 message\nW1 · 1:4  W1 message"
        );
        assert_eq!(copy_text(&[]), "");
    }

    #[test]
    fn one_problem_is_a_single_line() {
        let one = diag("E1", DiagnosticSeverity::Error, 3, 30);
        assert_eq!(problem_line(&one), "E1 · 3:4  E1 message");
    }
}

// MARK: Report

const ALERT_LIMIT: usize = 100;

/// "Jan 5, 3:07 PM" in the system time zone, on the language's own clock.
pub fn alert_time(ms: i64) -> String {
    let local = ms + crate::service::system::timezone::local_utc_offset_secs(ms) * 1000;
    let secs = local.div_euclid(1000);
    let (year, month, day) = crate::format::civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    crate::i18n::datetime::month_day_clock((year, month, day), (rem / 3600, rem % 3600 / 60))
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
        Some(report) => parts.push(tr_n!(
            report.trades.len(),
            "Strategy · {count} trade",
            "Strategy · {count} trades"
        )),
        None if alerts > 0 => parts.push(tr!("Indicator").to_owned()),
        None => {}
    }
    if alerts > 0 {
        parts.push(tr_n!(alerts, "{count} alert", "{count} alerts"));
    }
    if parts.is_empty() {
        tr!("No report").to_owned()
    } else {
        parts.join(" \u{b7} ")
    }
}

/// The Report tab: a toolbar with what the script reported, and under it the strategy's figures,
/// equity curve, simulation and trades, and the alerts the script raised, as cards. Where the width
/// allows, the trades stand beside the figures.
pub fn report(ui: &mut Ui, visual: Option<&VisualOutput>) {
    let strategy = visual.and_then(|v| v.strategy.as_ref());
    let alerts: &[AlertEvent] = visual.map_or(&[], |v| v.alerts.as_slice());
    chrome::toolbar(ui, |ui, width| {
        let neutral = theme::text_secondary();
        let (dot, trailing) = match strategy {
            Some(report) => {
                let net = report.net_profit();
                let tint = report::tint(net, neutral);
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
        intro::report(ui);
        return;
    }
    chrome::scroll(ui, "script-report-scroll", |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(report) = strategy else {
            alerts_card(ui, alerts);
            return;
        };
        let alerts_below = |ui: &mut Ui| {
            if !alerts.is_empty() {
                ui.add_space(chrome::SECTION);
                alerts_card(ui, alerts);
            }
        };
        if !report.trades.is_empty() && chrome::wide(ui) {
            chrome::columns(
                ui,
                0.5,
                |ui| {
                    figures::strategy(ui, report);
                    alerts_below(ui);
                },
                |ui| trades::trades_card(ui, report),
            );
        } else {
            figures::strategy(ui, report);
            ui.add_space(chrome::SECTION);
            trades::trades_card(ui, report);
            alerts_below(ui);
        }
    });
}

/// The alerts the script raised, newest first, with how many were live.
fn alerts_card(ui: &mut Ui, alerts: &[AlertEvent]) {
    let (live, historical) = alert_summary(alerts);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            tr!("Alerts"),
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        if live > 0 {
            chrome::chip(
                ui,
                &tr!("{count} live", count = live),
                theme::syntax::variable(),
            );
        }
        if historical > 0 {
            chrome::chip(
                ui,
                &tr!("{count} historical", count = historical),
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
                (tr!("No message"), theme::text_secondary())
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
            .on_hover_text(alert_hover(alert));
        }
    });
    if alerts.len() > ALERT_LIMIT {
        chrome::footnote(
            ui,
            &tr!(
                "Showing the latest {shown} of {total}",
                shown = ALERT_LIMIT,
                total = alerts.len()
            ),
        );
    }
}

/// The strategy report and the alerts box, without a scroll area of their own (the chart settings
/// show one per applied strategy inside their own page).
pub fn report_content(ui: &mut Ui, visual: Option<&VisualOutput>) {
    let strategy = visual.and_then(|v| v.strategy.as_ref());
    let alerts: &[AlertEvent] = visual.map_or(&[], |v| v.alerts.as_slice());
    ui.spacing_mut().item_spacing.y = 12.0;
    report::show(ui, strategy);
    if !alerts.is_empty() {
        alerts_section(ui, alerts);
    }
}

/// The alerts box: header with live/historical chips, collapsed by default (a log, not a
/// problem), newest first when open.
fn alerts_section(ui: &mut Ui, alerts: &[AlertEvent]) {
    let (live, historical) = alert_summary(alerts);
    report::report_box(ui, |ui| {
        let id = ui.make_persistent_id("pine-alerts-section");
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::BellFill, 12.0, theme::SYSTEM_ACCENT);
                ui.label(
                    RichText::new(tr!("Alerts"))
                        .size(10.0)
                        .variation("wght", crate::ui::kit::SEMIBOLD)
                        .color(theme::text_primary()),
                );
                if live > 0 {
                    chrome::chip(
                        ui,
                        &tr!("{count} live", count = live),
                        theme::syntax::variable(),
                    );
                }
                if historical > 0 {
                    chrome::chip(
                        ui,
                        &tr!("{count} historical", count = historical),
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
                        RichText::new(tr!(
                            "Showing the latest {shown} of {total}",
                            shown = ALERT_LIMIT,
                            total = alerts.len()
                        ))
                        .size(9.0)
                        .color(theme::text_tertiary()),
                    );
                }
            });
    });
}

/// What an alert's row says when pointed at: whether it notified, and how often it may fire.
fn alert_hover(alert: &AlertEvent) -> String {
    let frequency = frequency_name(alert.frequency);
    if alert.is_realtime {
        tr!("Live bar, notifies · {frequency}", frequency = frequency)
    } else {
        tr!(
            "Historical bar, never notifies · {frequency}",
            frequency = frequency
        )
    }
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
            (tr!("No message"), theme::text_secondary())
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
    row.response.on_hover_text(alert_hover(alert));
}

#[cfg(test)]
mod report_tests {
    use super::*;
    use crate::pine::builtins::alert::frequency::AlertFrequency;

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
        let ms = crate::pine_studio::inputs::parse_time("2024-03-09 14:30").unwrap();
        assert_eq!(alert_time(ms), "Mar 9, 2:30 PM");
        assert_eq!(alert_time(0), "Jan 1, 12:00 AM");
    }
}
