//! The painting helpers and fixed-height blocks the Overview and Statistics pages share: measured
//! text, panels and card frames, the page's spacing, and stat cards in a grid.
//!
//! Every block has a fixed height and paints its text at measured positions, so nothing reflows
//! while prices stream in and cards in one row always end level.

use crate::model::portfolio::format::{PortfolioValueFormatter, Tone};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio::table::{tone_color, tone_option};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, Galley, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2, pos2,
    vec2,
};
use std::sync::Arc;

/// Space around the page and between its blocks.
pub(super) const PAGE_INSET: i8 = 16;
pub(super) const SECTION_GAP: f32 = 16.0;
pub(super) const GRID_GAP: f32 = 12.0;
/// A stat or performer card: two 12 pt paddings around a 50 pt text block.
pub(super) const CARD_HEIGHT: f32 = 76.0;
pub(super) const HERO_HEIGHT: f32 = 136.0;
pub(super) const HERO_PADDING: f32 = 22.0;
/// From this content width the Overview puts a side column beside the charts and the lists.
pub(super) const WIDE_MIN: f32 = 1_000.0;
pub(super) const SIDE_WIDTH: f32 = 400.0;

/// A panel's padding, its title row and the gap under it.
const PANEL_PADDING: f32 = 16.0;
const PANEL_HEADER: f32 = 27.0;
const PANEL_GAP: f32 = 12.0;

/// One line of text at an SF weight, laid out so it can be measured before it is placed.
pub(super) fn line(ui: &Ui, text: &str, size: f32, weight: f32, color: Color32) -> Arc<Galley> {
    kit::galley(ui, text, size, weight, color, 4_000.0)
}

pub(super) fn draw(ui: &Ui, at: Pos2, galley: &Arc<Galley>) {
    ui.painter()
        .galley(at, galley.clone(), Color32::PLACEHOLDER);
}

/// Paints `text` anchored at `pos`; returns the rect it covers.
pub(super) fn text_at(
    ui: &Ui,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    size: f32,
    weight: f32,
    color: Color32,
) -> Rect {
    let galley = line(ui, text, size, weight, color);
    let rect = anchor.anchor_size(pos, galley.size());
    draw(ui, rect.min, &galley);
    rect
}

pub(super) fn caption_color() -> Color32 {
    theme::text_secondary()
}

/// The tone's colour, or `fallback` when there is none (zero, unknown, or values hidden).
pub(crate) fn tone_or(tone: Option<Tone>, fallback: Color32) -> Color32 {
    tone_option(tone).unwrap_or(fallback)
}

/// A small filled triangle: the direction mark beside a change percentage.
pub(super) fn paint_arrow(ui: &Ui, center: Pos2, up: bool, color: Color32) {
    let (half_width, half_height) = (3.5, 3.0);
    let points = if up {
        vec![
            pos2(center.x - half_width, center.y + half_height),
            pos2(center.x + half_width, center.y + half_height),
            pos2(center.x, center.y - half_height),
        ]
    } else {
        vec![
            pos2(center.x - half_width, center.y - half_height),
            pos2(center.x + half_width, center.y - half_height),
            pos2(center.x, center.y + half_height),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A raised surface: a soft shadow under the fill, and a hairline around it.
pub(crate) fn lift(ui: &Ui, rect: Rect, radius: f32) {
    let painter = ui.painter();
    painter.add(
        egui::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(30),
        }
        .as_shape(rect, radius),
    );
    painter.rect_filled(rect, radius, theme::chrome());
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
}

/// A soft light of `color` rising from the bottom of `rect`, strongest right of centre and
/// gone before the corners, so it needs no clipping to the rounded shape.
pub(super) fn glow(ui: &Ui, rect: Rect, color: Color32) {
    const COLUMNS: usize = 20;
    let from = rect.left() + rect.width() * 0.3;
    let to = rect.right() - 20.0;
    let (top, bottom) = (rect.top() + rect.height() * 0.35, rect.bottom() - 1.0);
    let mut mesh = egui::Mesh::default();
    for column in 0..=COLUMNS {
        let t = column as f32 / COLUMNS as f32;
        let x = from + (to - from) * t;
        let bell = (std::f32::consts::PI * t).sin().powi(2);
        mesh.colored_vertex(pos2(x, top), color.gamma_multiply(0.0));
        mesh.colored_vertex(pos2(x, bottom), color.gamma_multiply(0.13 * bell));
    }
    for column in 0..COLUMNS as u32 {
        let base = column * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    ui.painter().add(Shape::mesh(mesh));
}

/// A panel: fixed outer height, a title row with an optional trailing control, then content.
/// Because the height is given, two panels in a row end level whatever they hold.
pub(super) fn panel(
    ui: &mut Ui,
    content_height: f32,
    icon: Icon,
    title: &str,
    content: impl FnOnce(&mut Ui, &mut Ui),
) {
    let height = panel_height(content_height);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    lift(ui, rect, 14.0);
    let inner = rect.shrink(PANEL_PADDING);
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), PANEL_HEADER));
    let chip = Rect::from_center_size(
        pos2(header.left() + 11.0, header.center().y),
        Vec2::splat(22.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, theme::SYSTEM_ACCENT.gamma_multiply(0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chip.center(), Vec2::splat(13.0)),
        icon,
        theme::SYSTEM_ACCENT,
    );
    text_at(
        ui,
        pos2(chip.right() + 9.0, header.center().y),
        Align2::LEFT_CENTER,
        title,
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
    );
    let mut header_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(header)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let body = Rect::from_min_size(
        pos2(inner.left(), header.bottom() + PANEL_GAP),
        vec2(inner.width(), content_height),
    );
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    body_ui.set_clip_rect(
        body.expand2(vec2(PANEL_PADDING, 0.0))
            .intersect(ui.clip_rect()),
    );
    content(&mut header_ui, &mut body_ui);
}

/// The outer height of a [`panel`] whose content is `content_height` tall.
pub(super) fn panel_height(content_height: f32) -> f32 {
    PANEL_PADDING * 2.0 + PANEL_HEADER + PANEL_GAP + content_height
}

/// A tinted, bordered card of a fixed height; `inside` is the content rect. `wash` lays a faint
/// light of the card's tone over it.
pub(super) fn card_frame(ui: &mut Ui, height: f32, wash: Option<Color32>) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 12.0, theme::chrome());
    if let Some(wash) = wash {
        ui.painter()
            .rect_filled(rect, 12.0, wash.gamma_multiply(0.07));
    }
    ui.painter().rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    rect
}

/// Two columns of the given widths, tops level, with `gap` between. Both share `shared`, which
/// each takes in turn.
pub(super) fn split<T>(
    ui: &mut Ui,
    widths: [f32; 2],
    gap: f32,
    shared: &mut T,
    left: impl FnOnce(&mut Ui, &mut T),
    right: impl FnOnce(&mut Ui, &mut T),
) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            vec2(widths[0], 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(widths[0]);
                left(ui, shared);
            },
        );
        ui.allocate_ui_with_layout(
            vec2(widths[1], 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(widths[1]);
                right(ui, shared);
            },
        );
    });
}

pub struct StatCard<'a> {
    pub title: &'a str,
    pub value: String,
    pub tone: Option<Tone>,
    pub caption: Option<String>,
    /// Quieter value when there is nothing to report yet.
    pub placeholder: bool,
    pub icon: Option<Icon>,
}

impl<'a> StatCard<'a> {
    pub fn new(title: &'a str, value: String) -> Self {
        Self {
            title,
            value,
            tone: None,
            caption: None,
            placeholder: false,
            icon: None,
        }
    }

    pub fn tone(mut self, tone: Option<Tone>) -> Self {
        self.tone = tone;
        self
    }

    pub fn caption(mut self, caption: Option<String>) -> Self {
        self.caption = caption;
        self
    }

    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

pub fn stat_card(ui: &mut egui::Ui, card: &StatCard) {
    let tint = card.tone.map_or(theme::SYSTEM_ACCENT, tone_color);
    let rect = card_frame(ui, CARD_HEIGHT, card.tone.map(tone_color));
    let mut x = rect.left() + 14.0;
    if let Some(icon) = card.icon {
        let badge = Rect::from_center_size(pos2(x + 17.0, rect.center().y), Vec2::splat(34.0));
        ui.painter()
            .rect_filled(badge, 9.0, tint.gamma_multiply(0.14));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(badge.center(), Vec2::splat(17.0)),
            icon,
            tint,
        );
        x = badge.right() + 12.0;
    }
    let max_width = (rect.right() - 14.0 - x).max(20.0);
    let secondary = caption_color();
    let title = line(ui, card.title, kit::size::CAPTION, REGULAR, secondary);
    let value_color = if card.placeholder {
        secondary
    } else {
        tone_or(card.tone, theme::text_primary())
    };
    let value = kit::galley(ui, &card.value, 17.0, SEMIBOLD, value_color, max_width);
    // Always laid out, so cards with and without a caption share one text block.
    let caption = card.caption.as_deref().map(|caption| {
        kit::galley(
            ui,
            caption,
            kit::size::CAPTION,
            REGULAR,
            tone_or(card.tone, secondary),
            max_width,
        )
    });
    let block = title.size().y + 4.0 + value.size().y + 4.0 + title.size().y;
    let mut y = rect.center().y - block / 2.0;
    draw(ui, pos2(x, y), &title);
    y += title.size().y + 4.0;
    draw(ui, pos2(x, y), &value);
    y += value.size().y + 4.0;
    if let Some(caption) = caption {
        draw(ui, pos2(x, y), &caption);
    }
}

pub fn grid(ui: &mut egui::Ui, cards: &[StatCard]) {
    ui.spacing_mut().item_spacing.x = GRID_GAP;
    // Two across where four would squeeze a figure out of its card.
    let across = if ui.available_width() < 640.0 { 2 } else { 4 };
    ui.columns(across, |columns| {
        for (index, card) in cards.iter().enumerate() {
            stat_card(&mut columns[index % across], card);
        }
    });
}

/// A period label over a signed amount over a pill with the percentage, left-aligned in a block
/// that ends at `anchor.x` and is centred on `anchor.y`. Returns the block's left edge.
pub(super) fn change_block(
    ui: &Ui,
    f: PortfolioValueFormatter,
    label: &str,
    amount: Option<f64>,
    percentage: Option<f64>,
    anchor: Pos2,
) -> f32 {
    let secondary = caption_color();
    let label = line(ui, label, kit::size::CAPTION, REGULAR, secondary);
    let tone = f.tone(amount);
    let value = match amount {
        Some(amount) => line(
            ui,
            &f.signed_money(amount),
            17.0,
            SEMIBOLD,
            tone_or(tone, theme::text_primary()),
        ),
        None => line(ui, "\u{2014}", 17.0, SEMIBOLD, secondary),
    };
    let percent = percentage.filter(|_| amount.is_some()).map(|percentage| {
        line(
            ui,
            &f.percent(percentage.abs(), 2),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(tone, secondary),
        )
    });
    let arrow = amount.is_some_and(|amount| !f.privacy && amount != 0.0);
    let pill_width = percent.as_ref().map_or(0.0, |galley| {
        galley.size().x + 16.0 + if arrow { 12.0 } else { 0.0 }
    });
    let width = label.size().x.max(value.size().x).max(pill_width);
    let pill_height = if percent.is_some() { 20.0 + 5.0 } else { 0.0 };
    let height = label.size().y + 3.0 + value.size().y + pill_height;
    let left = anchor.x - width;
    let mut y = anchor.y - height / 2.0;
    draw(ui, pos2(left, y), &label);
    y += label.size().y + 3.0;
    draw(ui, pos2(left, y), &value);
    y += value.size().y + 5.0;
    if let Some(percent) = percent {
        let pill = Rect::from_min_size(pos2(left, y), vec2(pill_width, 20.0));
        let fill = tone_option(tone).unwrap_or_else(theme::text_tertiary);
        ui.painter()
            .rect_filled(pill, 10.0, fill.gamma_multiply(0.16));
        let mut x = pill.left() + 8.0;
        if arrow {
            paint_arrow(
                ui,
                pos2(x + 3.5, pill.center().y),
                amount.is_some_and(|amount| amount > 0.0),
                tone_or(tone, secondary),
            );
            x += 12.0;
        }
        draw(
            ui,
            pos2(x, pill.center().y - percent.size().y / 2.0),
            &percent,
        );
    }
    left
}
