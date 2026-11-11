//! A provider error as a card: a headline, a few lines of detail, the request id and the buttons
//! that follow from it. The assistant's panel, its model menu and Settings all show the same
//! error, so they share this instead of printing the provider's paragraph on one line. The text
//! comes as an [`ErrorReport`], built from the structure of the error and never by reading its prose;
//! what is cut from view is still on the clipboard behind **Copy details**.

use crate::service::llm::report::ErrorReport;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align, Id, Layout, Margin, Ui,
    text::{LayoutJob, TextFormat},
};
use std::time::{Duration, Instant};

/// How long "Copy details" says "Copied".
const COPIED: Duration = Duration::from_millis(1500);

pub struct ErrorNotice<'a> {
    pub report: &'a ErrorReport,
    /// Lines of detail shown; the rest is behind Copy details and the hover text.
    pub detail_lines: usize,
    /// The label of a retry button, if the error can be retried.
    pub retry: Option<&'a str>,
    /// Draws a dismiss button.
    pub dismiss: bool,
    /// Distinguishes two cards on one screen (the Copy feedback is kept by it).
    pub id: Id,
}

impl<'a> ErrorNotice<'a> {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, report: &'a ErrorReport) -> Self {
        Self {
            report,
            detail_lines: 3,
            retry: None,
            dismiss: false,
            id: Id::new(("error-notice", id)),
        }
    }

    pub fn retry(mut self, label: &'a str) -> Self {
        self.retry = Some(label);
        self
    }

    pub fn dismissable(mut self) -> Self {
        self.dismiss = true;
        self
    }

    pub fn detail_lines(mut self, lines: usize) -> Self {
        self.detail_lines = lines;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorEvent {
    Retry,
    Dismiss,
}

/// Draws the card across the width it is given and reports a click on Try again or Dismiss.
pub fn show(ui: &mut Ui, notice: &ErrorNotice<'_>) -> Option<ErrorEvent> {
    let tint = kit::RED;
    let mut event = None;
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.1))
        .stroke(egui::Stroke::new(1.0, theme::with_alpha(tint, 0.22)))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::TriangleFill, 13.0, tint);
                let reserved = if notice.dismiss { 26.0 } else { 0.0 };
                let column = (ui.available_width() - reserved).max(40.0);
                ui.vertical(|ui| {
                    ui.set_width(column);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    body(ui, notice, column);
                    if let Some(clicked) = buttons(ui, notice) {
                        event = Some(clicked);
                    }
                });
                if notice.dismiss {
                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        if icons::button(ui, Icon::Xmark, 18.0, theme::text_tertiary(), "Dismiss")
                            .clicked()
                        {
                            event = Some(ErrorEvent::Dismiss);
                        }
                    });
                }
            });
        });
    event
}

/// The headline, the clipped detail and the request id.
fn body(ui: &mut Ui, notice: &ErrorNotice<'_>, width: f32) {
    let summary = notice.report;
    let headline = lines(
        ui,
        &summary.headline,
        kit::size::SUBHEADLINE + 0.5,
        kit::SEMIBOLD,
        theme::text_primary(),
        width,
        3,
    );
    let response = ui.add(egui::Label::new(headline).selectable(true));
    if !summary.detail.is_empty() {
        let detail = lines(
            ui,
            &summary.detail,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
            width,
            notice.detail_lines,
        );
        let clipped = detail.rows.len() >= notice.detail_lines && detail.elided;
        let response = ui.add(egui::Label::new(detail).selectable(true));
        if clipped {
            response.on_hover_text(summary.copy_text());
        }
    } else if summary.headline.chars().count() > 60 {
        response.on_hover_text(summary.copy_text());
    }
    if let Some(id) = &summary.request_id {
        // An id is one long word: it breaks wherever the line ends, so all of it shows.
        let mut job = LayoutJob::default();
        job.append(
            &format!("Request ID {id}"),
            0.0,
            TextFormat {
                font_id: egui::FontId::monospace(kit::size::CAPTION),
                color: theme::text_tertiary(),
                ..Default::default()
            },
        );
        job.wrap.max_width = width.max(1.0);
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.add(egui::Label::new(galley).selectable(true));
    }
}

/// Try again and Copy details, on one row that wraps in a narrow card.
fn buttons(ui: &mut Ui, notice: &ErrorNotice<'_>) -> Option<ErrorEvent> {
    let mut event = None;
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        if let Some(label) = notice.retry
            && kit::push_button(ui, Some(Icon::Repeat), label, true).clicked()
        {
            event = Some(ErrorEvent::Retry);
        }
        let copied = ui
            .ctx()
            .data(|data| data.get_temp::<Instant>(notice.id))
            .is_some_and(|at| at.elapsed() < COPIED);
        let (icon, label) = if copied {
            (Icon::Check, "Copied")
        } else {
            (Icon::CopyDoc, "Copy details")
        };
        if kit::push_button(ui, Some(icon), label, !copied).clicked() {
            ui.ctx().copy_text(notice.report.copy_text());
            ui.ctx()
                .data_mut(|data| data.insert_temp(notice.id, Instant::now()));
            ui.ctx().request_repaint_after(COPIED);
        }
    });
    event
}

/// Wrapped text of at most `rows` lines; what does not fit ends in an ellipsis.
fn lines(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: f32,
    color: egui::Color32,
    width: f32,
    rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: egui::FontId::proportional(size),
            color,
            coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
            ..Default::default()
        },
    );
    job.wrap.max_width = width.max(1.0);
    job.wrap.max_rows = rows.max(1);
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::render_tests::{Screen, gallery_dir};
    use eframe::egui::vec2;

    #[test]
    fn every_shape_of_error_stays_inside_the_width_it_is_given() {
        for (shape, error) in crate::service::llm::sample::shapes() {
            let report = error.report();
            for width in [240.0, 300.0, 520.0] {
                let mut screen = Screen::new(vec2(width + 40.0, 500.0));
                let mut last = None;
                for _ in 0..3 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        ui.set_max_width(width);
                        show(
                            ui,
                            &ErrorNotice::new("test", &report)
                                .retry("Try again")
                                .dismissable(),
                        );
                        let used = ui.min_rect();
                        assert!(
                            used.width() <= width + 1.0,
                            "{shape} at {width}: the card used {:?}",
                            used.size()
                        );
                        assert!(
                            used.height() < 280.0,
                            "{shape} at {width}: the card used {:?}",
                            used.size()
                        );
                    }));
                }
                if gallery_dir().is_some() {
                    screen.save(
                        &format!("error-notice-{shape}-{width}"),
                        last.expect("a frame"),
                    );
                }
            }
        }
    }
}
