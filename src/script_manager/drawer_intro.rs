//! What the Problems and Report tabs say while they have nothing to show, laid out like the Logs
//! and Profile tabs' introductions: a picture, what the tab is for, three numbered steps and an
//! example of what it will hold.

use crate::ui::icons::{self, Icon};
use crate::ui::pine_logs_view::{centered, intro, step};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2,
    text::{LayoutJob, TextFormat},
    vec2,
};

/// The Problems tab with no diagnostics.
pub fn problems(ui: &mut Ui) {
    centered(ui, |ui| {
        intro(
            ui,
            "How problems show up",
            "Nothing to fix in this script.",
            |ui| {
                step(ui, 1, |ui| {
                    ui.label(muted(
                        "Write or edit the script; it is checked as you type.",
                    ));
                });
                step(ui, 2, |ui| {
                    ui.label(muted("Errors and warnings are listed here, errors first."));
                });
                step(ui, 3, |ui| {
                    ui.label(muted("Click a row to go to its line."));
                });
            },
            problem_example,
            problems_picture,
            |_| {},
        );
    });
}

/// The Report tab with no strategy and no alerts.
pub fn report(ui: &mut Ui) {
    centered(ui, |ui| {
        intro(
            ui,
            "How to read a report",
            "See how a strategy would have traded.",
            |ui| {
                step(ui, 1, |ui| {
                    ui.label(muted("Apply a strategy to the preview chart."));
                });
                step(ui, 2, |ui| {
                    ui.label(muted(
                        "Its figures, equity curve and every trade appear here.",
                    ));
                });
                step(ui, 3, |ui| {
                    ui.label(muted("An indicator lists the alerts it raises."));
                });
            },
            report_example,
            report_picture,
            |_| {},
        );
    });
}

fn muted(text: &str) -> RichText {
    kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
    )
}

/// The soft disc and the card the pictures draw on.
fn card(ui: &Ui, rect: Rect) -> Rect {
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 40.0, kit::ACCENT.gamma_multiply(0.10));
    let card = Rect::from_center_size(rect.center(), vec2(64.0, 54.0));
    painter.rect_filled(card, 9.0, theme::chrome());
    painter.rect_stroke(
        card,
        9.0,
        Stroke::new(1.5, theme::text_tertiary()),
        StrokeKind::Inside,
    );
    card
}

/// A page of findings on a soft disc: a red, an amber and a green line, the three outcomes.
fn problems_picture(ui: &Ui, rect: Rect) {
    let card = card(ui, rect);
    let rows = [(kit::RED, 0.70), (theme::YELLOW, 0.46), (kit::GREEN, 0.62)];
    for (index, (color, length)) in rows.into_iter().enumerate() {
        let y = card.top() + 14.0 + index as f32 * 13.0;
        ui.painter()
            .circle_filled(pos2(card.left() + 12.0, y), 2.6, color);
        ui.painter().hline(
            card.left() + 20.0..=card.left() + 20.0 + (card.width() - 32.0) * length,
            y,
            Stroke::new(2.2, color.gamma_multiply(0.75)),
        );
    }
}

/// An equity curve over a dashed starting line, with the closed trades as bars under it.
fn report_picture(ui: &Ui, rect: Rect) {
    let card = card(ui, rect);
    let painter = ui.painter();
    let left = card.left() + 10.0;
    let right = card.right() - 10.0;
    let base = card.top() + 28.0;
    painter.hline(
        left..=right,
        base,
        Stroke::new(1.0, theme::text_tertiary().gamma_multiply(0.6)),
    );
    let curve = [0.0, -4.0, 3.0, -1.0, 8.0, 5.0, 12.0];
    let points: Vec<_> = curve
        .iter()
        .enumerate()
        .map(|(index, rise)| {
            pos2(
                left + (right - left) * index as f32 / (curve.len() - 1) as f32,
                base - rise,
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(2.0, kit::GREEN)));
    for (index, (height, color)) in [(6.0, kit::GREEN), (4.0, kit::RED), (8.0, kit::GREEN)]
        .into_iter()
        .enumerate()
    {
        let x = left + 4.0 + index as f32 * 14.0;
        painter.rect_filled(
            Rect::from_min_size(pos2(x, card.bottom() - 8.0 - height), vec2(8.0, height)),
            2.0,
            color.gamma_multiply(0.8),
        );
    }
}

/// A row as the tab draws it, on a well.
fn problem_example(ui: &mut Ui, width: f32) {
    let painter = ui.painter().clone();
    let mono = FontId::monospace(10.0);
    let code = painter.layout_no_wrap("PINE3050".to_owned(), mono.clone(), theme::text_secondary());
    let place = painter.layout_no_wrap("3:4".to_owned(), mono, theme::text_tertiary());
    let message = painter.layout_no_wrap(
        "Undefined variable 'x'.".to_owned(),
        FontId::proportional(12.0),
        theme::text_primary(),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    painter.rect_filled(rect, 8.0, theme::well_fill());
    let mid = rect.center().y;
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(rect.left() + 18.0, mid), vec2(12.0, 12.0)),
        Icon::XmarkOctagonFill,
        theme::syntax::string(),
    );
    let capsule = Rect::from_min_size(
        pos2(rect.left() + 32.0, mid - 8.0),
        vec2(code.size().x + 10.0, 16.0),
    );
    painter.rect_filled(capsule, 5.0, theme::ink(10));
    painter.galley(
        pos2(capsule.left() + 5.0, mid - code.size().y / 2.0),
        code,
        theme::text_secondary(),
    );
    let room = rect.right() - 12.0 - place.size().x - 10.0 - (capsule.right() + 8.0);
    if room > message.size().x {
        painter.galley(
            pos2(capsule.right() + 8.0, mid - message.size().y / 2.0),
            message,
            theme::text_primary(),
        );
    }
    painter.galley(
        pos2(
            rect.right() - 12.0 - place.size().x,
            mid - place.size().y / 2.0,
        ),
        place,
        theme::text_tertiary(),
    );
}

/// The call that makes a script report, in the editor's colours.
fn report_example(ui: &mut Ui, width: f32) {
    let mono = |color: Color32| TextFormat {
        font_id: FontId::monospace(11.0),
        color,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append("strategy.entry", 0.0, mono(theme::syntax::function()));
    job.append("(", 0.0, mono(theme::text_secondary()));
    job.append("\"Long\"", 0.0, mono(theme::syntax::string()));
    job.append(", ", 0.0, mono(theme::text_secondary()));
    job.append("strategy.long", 0.0, mono(theme::syntax::variable()));
    job.append(")", 0.0, mono(theme::text_secondary()));
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::well_fill());
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 9.0),
        galley,
        Color32::PLACEHOLDER,
    );
}
