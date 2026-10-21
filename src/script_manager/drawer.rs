//! The preview pane's drawer: the always-visible tab bar (Inputs / Report / Problems) and the
//! tab content. Ported from `ScriptPreviewDrawer`, `ScriptPreviewDrawerBar` and
//! `PineDiagnosticsListView`.

use super::prefs::DrawerTab;
use crate::model::pine_alert::frequency_name;
use crate::pine::name_check;
use crate::pine::program::VisualOutput;
use crate::pine::state::AlertEvent;
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
use crate::ui::pine_report;
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, Rect, RichText, Sense, Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

pub const BAR_HEIGHT: f32 = super::bottom_bar::HEIGHT;

impl DrawerTab {
    pub const ALL: [Self; 3] = [Self::Inputs, Self::Report, Self::Problems];

    pub fn title(self) -> &'static str {
        match self {
            Self::Inputs => "Inputs",
            Self::Report => "Report",
            Self::Problems => "Problems",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Inputs => Icon::Sliders,
            Self::Report => Icon::ChartUptrend,
            Self::Problems => Icon::Triangle,
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

/// A centred "title / caption" placeholder, like the drawer's empty states.
pub fn placeholder(ui: &mut Ui, title: &str, caption: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 30.0).max(8.0));
        ui.label(
            RichText::new(title)
                .size(11.0)
                .variation("wght", crate::ui::kit::SEMIBOLD)
                .color(theme::text_primary()),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(caption)
                .size(10.0)
                .color(theme::text_secondary()),
        );
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

/// The Problems list: the editor's diagnostics, plus the chart's runtime failure once the text
/// compiles. An undefined name is warned about at compile time and then fails the run at the same
/// spot, so the warning gives way to the error instead of listing the mistake twice.
pub fn with_runtime_error(
    diagnostics: &[Diagnostic],
    runtime_error: Option<&Diagnostic>,
) -> Vec<Diagnostic> {
    let mut list = diagnostics.to_vec();
    if list.iter().any(|d| d.severity == DiagnosticSeverity::Error) {
        return list;
    }
    if let Some(error) = runtime_error {
        list.retain(|d| {
            let is_name_warning = d.code == name_check::UNDEFINED_VARIABLE
                || d.code == name_check::UNDEFINED_FUNCTION;
            !(is_name_warning && d.range.start.offset == error.range.start.offset)
        });
        list.push(error.clone());
    }
    list
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

/// The Problems tab. Returns the diagnostic whose row was clicked.
pub fn problems(
    ui: &mut Ui,
    diagnostics: &[Diagnostic],
    copied_at: &mut Option<Instant>,
) -> Option<Diagnostic> {
    if diagnostics.is_empty() {
        placeholder(
            ui,
            "No problems",
            "Compile errors, warnings and runtime failures are listed here.",
        );
        return None;
    }
    let mut jump = None;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Diagnostics")
                        .size(10.0)
                        .variation("wght", crate::ui::kit::SEMIBOLD)
                        .color(theme::text_primary()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let just_copied =
                        copied_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2));
                    let has_errors = diagnostics
                        .iter()
                        .any(|d| d.severity == DiagnosticSeverity::Error);
                    let label = if just_copied {
                        "Copied"
                    } else if has_errors {
                        "Copy Errors"
                    } else {
                        "Copy Problems"
                    };
                    let color = if just_copied {
                        theme::text_secondary()
                    } else {
                        theme::SYSTEM_ACCENT
                    };
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let text = ui.add(
                        egui::Label::new(RichText::new(label).size(10.0).color(color))
                            .sense(Sense::click()),
                    );
                    icons::inline(
                        ui,
                        if just_copied {
                            Icon::Check
                        } else {
                            Icon::CopyDoc
                        },
                        11.0,
                        color,
                    );
                    if text.on_hover_cursor(CursorIcon::PointingHand).clicked() && !just_copied {
                        ui.ctx().copy_text(copy_text(diagnostics));
                        *copied_at = Some(Instant::now());
                    }
                    if just_copied {
                        ui.ctx().request_repaint_after(Duration::from_secs(2));
                    }
                });
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("problems-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for diagnostic in sorted(diagnostics) {
                        if problem_row(ui, diagnostic).clicked() {
                            jump = Some(diagnostic.clone());
                        }
                    }
                });
        });
    jump
}

fn problem_row(ui: &mut Ui, diagnostic: &Diagnostic) -> egui::Response {
    let (icon, color) = match diagnostic.severity {
        DiagnosticSeverity::Error => (Icon::XmarkOctagonFill, theme::syntax::string()),
        DiagnosticSeverity::Warning => (Icon::TriangleFill, theme::YELLOW),
    };
    let prefix = format!(
        "{} · {}:{}",
        diagnostic.code, diagnostic.range.start.line, diagnostic.range.start.column
    );
    let mut job = egui::text::LayoutJob::default();
    let mono = FontId::monospace(10.0);
    job.append(
        &prefix,
        0.0,
        egui::TextFormat::simple(mono.clone(), theme::text_secondary()),
    );
    job.append(
        "  ",
        0.0,
        egui::TextFormat::simple(mono.clone(), theme::text_secondary()),
    );
    job.append(
        &diagnostic.message,
        0.0,
        egui::TextFormat::simple(mono, theme::text_primary()),
    );
    let line = diagnostic.range.start.line;
    // The label is plain (no click sense) so the whole row, not just the text, is one target.
    let inner = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::inline(ui, icon, 12.0, color);
        ui.add(egui::Label::new(job).wrap().selectable(false));
    });
    let rect = inner.response.rect.with_max_x(ui.max_rect().right());
    let response = ui.interact(
        rect,
        ui.id()
            .with(("problem-row", line, &diagnostic.code, &diagnostic.message)),
        Sense::click(),
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!("Go to line {line}"))
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
    fn a_runtime_error_replaces_the_name_warning_at_the_same_spot() {
        use DiagnosticSeverity::{Error, Warning};
        let warning = diag(name_check::UNDEFINED_VARIABLE, Warning, 3, 30);
        let other = diag("PINE7006", Warning, 1, 5);
        let failure = diag("PINE4008", Error, 3, 30);
        let list = with_runtime_error(&[warning.clone(), other.clone()], Some(&failure));
        assert_eq!(list, [other.clone(), failure.clone()]);
        // A name warning elsewhere in the script stays.
        let elsewhere = diag(name_check::UNDEFINED_FUNCTION, Warning, 8, 80);
        let list = with_runtime_error(std::slice::from_ref(&elsewhere), Some(&failure));
        assert_eq!(list, [elsewhere, failure]);
        // Without a run failure, the warning is all there is.
        assert_eq!(
            with_runtime_error(std::slice::from_ref(&warning), None),
            [warning]
        );
    }

    #[test]
    fn a_compile_error_hides_the_runtime_error() {
        use DiagnosticSeverity::Error;
        let compile = diag("PINE2001", Error, 1, 5);
        let failure = diag("PINE4008", Error, 3, 30);
        assert_eq!(
            with_runtime_error(std::slice::from_ref(&compile), Some(&failure)),
            [compile]
        );
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

/// The Report tab: the strategy report, and the alerts the script raised.
pub fn report(ui: &mut Ui, visual: Option<&VisualOutput>) {
    let strategy = visual.and_then(|v| v.strategy.as_ref());
    let alerts: &[AlertEvent] = visual.map_or(&[], |v| v.alerts.as_slice());
    if strategy.is_none() && alerts.is_empty() {
        placeholder(
            ui,
            "No report",
            "Strategies report their trades and equity curve here. Indicators show alerts they raise.",
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("script-report-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| report_content(ui, visual));
        });
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

fn count_chip(ui: &mut Ui, text: &str, tint: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(9.0), tint);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 15.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 8.0, theme::with_alpha(tint, 0.14));
    ui.painter().galley(
        pos2(rect.min.x + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        tint,
    );
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
                    count_chip(ui, &format!("{live} live"), theme::syntax::variable());
                }
                if historical > 0 {
                    count_chip(
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
