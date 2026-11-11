//! Portfolio cards on the chart grid: the header (an icon badge, the title and the portfolio it
//! covers, a star, a remove button) and a body per kind. A card paints a `CardData` it is handed
//! and never reaches for the ledger, the network or the database, so every state (loading,
//! empty, privacy) is a function of that value.
//!
//! The body is drawn with the painter like the candle charts; each kind lives in its own file.
//! A card is not a market chart: it has no indicators, alerts, drawings or scripts, and the
//! grid keeps those paths away from it.

mod allocation;
#[cfg(test)]
mod gallery_tests;
mod top_performers;
mod value;
mod value_chart;

use crate::model::portfolio::card::{CardData, CardKind, PortfolioCard};
use crate::model::portfolio::format::Tone;
use crate::model::portfolio::statistics::PortfolioHistoryRange;
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use crate::ui::{ChartCardAction, WatchlistMembership};
use eframe::egui::{
    self, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use std::sync::Arc;

pub const HEADER_HEIGHT: f32 = 45.0;
/// The row of summary and range control above a body.
const CONTROLS_HEIGHT: f32 = 36.0;
/// Below this width the summary and the range switch no longer fit one row, so the switch drops
/// to a row of its own, stretched across the card.
const NARROW_CARD: f32 = 420.0;
/// The two rows of a narrow card's controls.
const NARROW_CONTROLS_HEIGHT: f32 = 34.0 + 32.0;

/// How tall the controls above a body are for a card `width` wide.
pub(in crate::ui) fn controls_height(width: f32) -> f32 {
    if width < NARROW_CARD {
        NARROW_CONTROLS_HEIGHT
    } else {
        CONTROLS_HEIGHT
    }
}

pub fn icon(kind: CardKind) -> Icon {
    match kind {
        CardKind::ValueChart => Icon::ChartUptrend,
        CardKind::Value => Icon::Banknote,
        CardKind::Allocation => Icon::PieChart,
        CardKind::TopPerformers => Icon::BarChart,
    }
}

/// The glyph on its tinted badge, `size` square. Also the row icon in a watchlist.
pub fn paint_badge(painter: &egui::Painter, rect: Rect, kind: CardKind) {
    // Neutral, like the tile of a settings card: colour is kept for data and state.
    let radius = rect.width() * 0.29;
    painter.rect_filled(rect, radius, theme::ink(10));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::ink(14)),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(rect.width() * 0.55)),
        icon(kind),
        theme::text_secondary(),
    );
}

// MARK: Header

/// The header row. Reordering has its own handle, like a market card's.
#[allow(clippy::too_many_arguments)]
pub fn header(
    ui: &mut Ui,
    card: &PortfolioCard,
    scope: &str,
    drag_id: u64,
    is_favorite: bool,
    watchlists: &[WatchlistMembership],
    action: &mut ChartCardAction,
) {
    let width = ui.available_width();
    // Handle, badge, star and remove button take this much; the rest is the title.
    let title_width = (width - 10.0 - 28.0 - 27.0 * 2.0 - 24.0).max(60.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, HEADER_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.dnd_drag_source(egui::Id::new(("chart-drag", drag_id)), drag_id, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 20.0), Sense::hover());
                let color = theme::text_secondary().gamma_multiply(0.7);
                for x in [-2.0, 2.0] {
                    for y in [-4.0, 0.0, 4.0] {
                        ui.painter()
                            .circle_filled(rect.center() + Vec2::new(x, y), 1.0, color);
                    }
                }
            })
            .response
            .on_hover_text("Drag to reorder charts");

            let (badge, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
            paint_badge(ui.painter(), badge, card.kind);

            ui.vertical(|ui| {
                ui.set_width(title_width);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(
                    egui::Label::new(kit::text(
                        card.kind.title(),
                        13.0,
                        kit::SEMIBOLD,
                        theme::text_primary(),
                    ))
                    .truncate(),
                );
                ui.add(
                    egui::Label::new(kit::text(
                        scope,
                        11.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ))
                    .truncate(),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(
                    ui,
                    Icon::Xmark,
                    27.0,
                    theme::text_secondary(),
                    "Remove Chart",
                )
                .clicked()
                {
                    action.remove_requested = true;
                }
                ui.spacing_mut().item_spacing.x = 4.0;
                let (star, color, tip) = if is_favorite {
                    (
                        Icon::StarFill,
                        theme::text_primary(),
                        "Remove from Favorites",
                    )
                } else {
                    (Icon::Star, theme::text_secondary(), "Add to Favorites")
                };
                let favorite = icons::button(ui, star, 27.0, color, tip);
                favorite.context_menu(|ui| {
                    ui.label(egui::RichText::new("Add to Watchlist").small().weak());
                    for list in watchlists {
                        let mut included = list.contains_market;
                        if ui.checkbox(&mut included, &list.name).changed() {
                            action.watchlist_change = Some((list.id, included));
                            ui.close();
                        }
                    }
                });
                if favorite.clicked() {
                    action.toggle_favorite = true;
                }
            });
        },
    );
}

// MARK: Body

/// Draws the card's body into what is left of `ui`. `data` is `None` when the workspace has no
/// portfolio access (a test, or before the app wires it). Returns whether the card was clicked.
pub fn body(ui: &mut Ui, card: &mut PortfolioCard, data: Option<&CardData>, chart_id: u64) -> bool {
    let size = ui.available_size().max(Vec2::splat(1.0));
    let id = ui.id().with(("portfolio-card", chart_id));
    // Allocated first so the controls drawn after it win the pointer where they overlap.
    let (area, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(area);

    let Some(data) = data else {
        kit::empty_state(
            ui,
            area,
            icon(card.kind),
            "Portfolio Unavailable",
            "This card needs the portfolio, which isn't available here.",
        );
        return response.clicked();
    };
    if !data.has_portfolios {
        kit::empty_state(
            ui,
            area,
            Icon::Briefcase,
            "No Portfolios",
            "Create a portfolio in the Portfolio tab to see it here.",
        );
        return response.clicked();
    }
    if let Some(error) = &data.error
        && data.holdings.is_empty()
    {
        kit::empty_state(ui, area, Icon::Briefcase, "Portfolio Unavailable", error);
        return response.clicked();
    }

    let controls_height = if card.kind.has_range() {
        controls_height(area.width())
    } else {
        0.0
    };
    let controls = Rect::from_min_size(area.min, Vec2::new(area.width(), controls_height));
    let content = Rect::from_min_max(
        Pos2::new(area.left(), controls.bottom()),
        area.right_bottom(),
    );
    let content = match card.kind {
        // The big number and the donut use the whole card.
        CardKind::Value | CardKind::Allocation => area,
        CardKind::ValueChart | CardKind::TopPerformers => content,
    };

    let pointer = response.hover_pos();
    match card.kind {
        CardKind::ValueChart => value_chart::paint(ui, content, id, card, data, pointer),
        CardKind::Value => value::paint(ui, content, id, data, pointer),
        CardKind::Allocation => allocation::paint(ui, content, id, data, pointer),
        CardKind::TopPerformers => top_performers::paint(ui, content, id, card, data, pointer),
    }

    if card.kind.has_range() {
        paint_controls(ui, controls, id, card, data);
    }
    accessibility::label_as(
        response.clone(),
        egui::WidgetType::Panel,
        describe(card, data),
    );
    drop(painter);
    response.clicked()
}

/// The summary at the left of the controls and the range switch at the right, or under it when
/// the card is too narrow for both.
fn paint_controls(ui: &mut Ui, row: Rect, id: Id, card: &mut PortfolioCard, data: &CardData) {
    let options: Vec<(PortfolioHistoryRange, &str)> = PortfolioHistoryRange::ALL
        .iter()
        .map(|range| (*range, range.title()))
        .collect();
    let mut range = card.range;
    let narrow = row.width() < NARROW_CARD;
    let summary = if narrow {
        let switch = Rect::from_min_max(
            Pos2::new(row.left() + 4.0, row.bottom() - 28.0),
            Pos2::new(row.right() - 4.0, row.bottom() - 4.0),
        );
        let mut controls = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(switch)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        kit::segmented_filling(
            &mut controls,
            id.with("range"),
            &options,
            &mut range,
            switch.width(),
        );
        Rect::from_min_max(
            row.min + Vec2::new(6.0, 0.0),
            Pos2::new(row.right() - 6.0, switch.top() - 2.0),
        )
    } else {
        let mut controls = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row.shrink2(Vec2::new(4.0, 4.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        kit::segmented(&mut controls, id.with("range"), &options, &mut range);
        let summary_width = (row.width() - controls.min_rect().width() - 24.0).max(0.0);
        Rect::from_min_size(
            row.min + Vec2::new(6.0, 0.0),
            Vec2::new(summary_width, row.height()),
        )
    };
    card.range = range;
    match card.kind {
        CardKind::ValueChart => value_chart::paint_summary(ui, summary, card, data),
        CardKind::TopPerformers => top_performers::paint_summary(ui, summary, card, data),
        CardKind::Value | CardKind::Allocation => {}
    }
}

/// What a screen reader says for the card.
fn describe(card: &PortfolioCard, data: &CardData) -> String {
    let head = format!("{}, {}", card.kind.title(), data.scope_name);
    match card.kind {
        CardKind::Value | CardKind::ValueChart => {
            format!("{head}: {}", data.formatter.money(data.total_value))
        }
        CardKind::Allocation => {
            let slices = crate::ui::portfolio::charts::slices(&data.holdings);
            let parts: Vec<String> = slices
                .iter()
                .take(5)
                .map(|slice| format!("{} {}", slice.label, data.formatter.percent(slice.share, 0)))
                .collect();
            format!("{head}: {}", parts.join(", "))
        }
        CardKind::TopPerformers => {
            let parts: Vec<String> = data
                .performance
                .ranked
                .iter()
                .take(3)
                .map(|performer| {
                    format!(
                        "{} {}",
                        performer.ticker,
                        data.formatter.signed_percent(performer.change, 1)
                    )
                })
                .collect();
            format!("{head} over {}: {}", card.range.title(), parts.join(", "))
        }
    }
}

// MARK: Shared painting

pub(crate) fn tone_color(tone: Option<Tone>) -> Color32 {
    match tone {
        Some(Tone::Gain) => theme::UP,
        Some(Tone::Loss) => theme::DOWN,
        None => theme::text_secondary(),
    }
}

/// How far an entrance has run, `0…1` eased out, starting the first frame `id` is asked about.
/// Asking again with a new id (another range, another card) plays it again.
pub(crate) fn reveal(ctx: &egui::Context, id: Id, seconds: f64) -> f32 {
    let now = ctx.input(|input| input.time);
    let start = ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(id, || now));
    let progress = ((now - start) / seconds).clamp(0.0, 1.0);
    if progress < 1.0 {
        ctx.request_repaint();
    }
    let remaining = 1.0 - progress as f32;
    1.0 - remaining * remaining * remaining
}

/// A one-line galley at the largest size from `max` down to `min` that fits `width`.
pub(crate) fn fitted(
    ui: &Ui,
    text: &str,
    max: f32,
    min: f32,
    weight: f32,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut size = max;
    loop {
        let galley = kit::galley(ui, text, size, weight, color, f32::INFINITY);
        if galley.size().x <= width || size <= min {
            return kit::galley(ui, text, size.max(min), weight, color, width);
        }
        size -= 1.0;
    }
}

/// A small pill of text on a tinted fill; returns its rectangle.
pub(crate) fn chip(ui: &Ui, left_center: Pos2, text: &str, color: Color32, max_width: f32) -> Rect {
    let galley = kit::galley(
        ui,
        text,
        11.0,
        kit::SEMIBOLD,
        color,
        (max_width - 14.0).max(8.0),
    );
    let rect = Rect::from_min_size(
        Pos2::new(left_center.x, left_center.y - 10.0),
        Vec2::new(galley.size().x + 14.0, 20.0),
    );
    ui.painter()
        .rect_filled(rect, 10.0, theme::with_alpha(color, 0.14));
    ui.painter().galley(
        Pos2::new(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        Color32::PLACEHOLDER,
    );
    rect
}

/// A badge of lines near `anchor`, kept inside `bounds`; `lines` are `(text, size, weight, color)`.
pub(crate) fn tooltip(ui: &Ui, anchor: Pos2, bounds: Rect, lines: &[(String, f32, f32, Color32)]) {
    let galleys: Vec<Arc<egui::Galley>> = lines
        .iter()
        .map(|(text, size, weight, color)| kit::galley(ui, text, *size, *weight, *color, 260.0))
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 20.0;
    let height = galleys.iter().map(|g| g.size().y + 2.0).sum::<f32>() + 10.0;
    let x =
        (anchor.x - width / 2.0).clamp(bounds.left(), (bounds.right() - width).max(bounds.left()));
    let above = anchor.y - height - 12.0;
    let y = if above >= bounds.top() - 4.0 {
        above
    } else {
        anchor.y + 14.0
    };
    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height));
    let painter = ui.painter();
    painter.add(
        egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(60),
        }
        .as_shape(rect, 8.0),
    );
    painter.rect_filled(rect, 8.0, theme::chrome().gamma_multiply(0.97));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
    let mut cursor = rect.top() + 6.0;
    for galley in galleys {
        let line_height = galley.size().y + 2.0;
        painter.galley(
            Pos2::new(rect.left() + 10.0, cursor),
            galley,
            Color32::PLACEHOLDER,
        );
        cursor += line_height;
    }
}

/// A spinner and a message in a pill, centred in `area`.
pub(crate) fn loading_pill(ui: &Ui, area: Rect, message: &str) {
    let label = kit::galley(
        ui,
        message,
        12.0,
        kit::REGULAR,
        theme::text_primary(),
        area.width() - 80.0,
    );
    let pill = Rect::from_center_size(
        area.center(),
        Vec2::new(14.0 + 8.0 + label.size().x + 28.0, label.size().y + 20.0),
    );
    ui.painter().rect_filled(pill, 10.0, theme::ink(14));
    ui.painter().rect_stroke(
        pill,
        10.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    // A spinner drawn in place: a quarter turn of arc that keeps turning.
    let time = ui.ctx().input(|input| input.time) as f32;
    ui.ctx().request_repaint();
    let start = time * 5.0;
    ui.painter().add(Shape::line(
        crate::ui::portfolio::charts::arc(
            Pos2::new(pill.left() + 21.0, pill.center().y),
            5.5,
            start,
            start + 4.4,
        ),
        Stroke::new(1.8, theme::text_secondary()),
    ));
    ui.painter().galley(
        Pos2::new(pill.left() + 36.0, pill.center().y - label.size().y / 2.0),
        label,
        Color32::PLACEHOLDER,
    );
}

/// A dashed horizontal line.
pub(crate) fn dashed_h(painter: &egui::Painter, y: f32, from: f32, to: f32, stroke: Stroke) {
    let mut x = from;
    while x < to {
        painter.line_segment([Pos2::new(x, y), Pos2::new((x + 4.0).min(to), y)], stroke);
        x += 8.0;
    }
}

/// A dashed vertical line.
pub(crate) fn dashed_v(painter: &egui::Painter, x: f32, from: f32, to: f32, stroke: Stroke) {
    let mut y = from;
    while y < to {
        painter.line_segment([Pos2::new(x, y), Pos2::new(x, (y + 4.0).min(to))], stroke);
        y += 8.0;
    }
}

/// A filled mesh under a line, from `top` colour at the line to nearly clear at `bottom`.
pub(crate) fn area_under(
    painter: &egui::Painter,
    line: &[Pos2],
    bottom: f32,
    color: Color32,
    top_alpha: f32,
) {
    if line.len() < 2 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for point in line {
        mesh.colored_vertex(*point, color.gamma_multiply(top_alpha));
        mesh.colored_vertex(Pos2::new(point.x, bottom), color.gamma_multiply(0.0));
    }
    for step in 0..(line.len() - 1) {
        let base = (step * 2) as u32;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(Shape::mesh(mesh));
}

/// Text at `anchor` in the small grey of axis labels.
pub(crate) fn axis_text(ui: &Ui, anchor: Pos2, align: Align2, text: &str) {
    ui.painter().text(
        anchor,
        align,
        text,
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_distinct_icon() {
        let icons: Vec<Icon> = CardKind::ALL.iter().map(|kind| icon(*kind)).collect();
        for (index, icon) in icons.iter().enumerate() {
            assert!(!icons[..index].contains(icon), "{icon:?} is used twice");
        }
    }

    #[test]
    fn the_reveal_eases_from_nothing_to_everything() {
        let ctx = egui::Context::default();
        let id = Id::new("reveal-test");
        let first = reveal(&ctx, id, 10.0);
        assert!(first < 0.05, "an entrance starts at rest, got {first}");
        // A different id starts its own entrance.
        assert!(reveal(&ctx, Id::new("other"), 10.0) < 0.05);
    }
}
