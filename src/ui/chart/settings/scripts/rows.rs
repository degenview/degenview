//! The Scripts page's rows: the card they sit in, a saved script to add and a script on the chart
//! with its switch, badges and settings.

use super::{RowData, RowEvent};
use crate::model::pine::instance::VisualOrder;
use crate::model::pine::script::{LocalScript, ScriptType};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontId, Id, Margin, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

/// The candle indicator before a script row's "Evaluating…", and the gap after it.
const META_INDICATOR: f32 = 12.0;
const META_GAP: f32 = 5.0;

/// A card of `count` rows with an inset hairline between them.
pub(super) fn row_card(ui: &mut Ui, count: usize, mut row: impl FnMut(&mut Ui, usize)) {
    kit::card_frame(false)
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in 0..count {
                if index > 0 {
                    let y = ui.cursor().min.y;
                    let rect = ui.max_rect();
                    ui.painter().line_segment(
                        [pos2(rect.min.x + 54.0, y), pos2(rect.max.x - 12.0, y)],
                        Stroke::new(1.0, kit::hairline()),
                    );
                }
                row(ui, index);
            }
        });
}

/// A wash under the pointer. The first and last rows round the corners they share with the card,
/// so the wash never pokes out of it.
fn hover_wash(ui: &Ui, rect: Rect, index: usize, count: usize) {
    let radius = (kit::CARD_RADIUS - 1.0) as u8;
    ui.painter().rect_filled(
        rect.shrink(1.0),
        CornerRadius {
            nw: if index == 0 { radius } else { 0 },
            ne: if index == 0 { radius } else { 0 },
            sw: if index + 1 == count { radius } else { 0 },
            se: if index + 1 == count { radius } else { 0 },
        },
        theme::ink(6),
    );
}

/// The switch at a row's trailing edge, `on` or off, animated. `what` names the script for
/// screen readers and the tooltip.
fn switch_at(ui: &mut Ui, row: Rect, id: Id, on: bool, what: &str) -> egui::Response {
    let rect = Rect::from_center_size(
        pos2(row.max.x - 12.0 - 18.0, row.center().y),
        vec2(36.0, 20.0),
    );
    let response = ui.interact(rect, id, Sense::click());
    let t = ui.ctx().animate_bool_with_time(id, on, 0.12);
    kit::paint_switch(ui.painter(), rect, t, false);
    let tip = if on {
        format!("Turn {what} off — it keeps its settings")
    } else {
        format!("Turn {what} on")
    };
    crate::ui::accessibility::label(
        response,
        format!("{what}, {}", if on { "on" } else { "off" }),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .on_hover_text(tip)
}

/// A neutral 30pt tile holding `icon`, as on every settings card.
fn badge_tile(ui: &Ui, row: Rect, tint: Color32, icon: Icon, stroke: Option<Color32>) -> Rect {
    let badge = Rect::from_center_size(pos2(row.min.x + 27.0, row.center().y), Vec2::splat(30.0));
    match stroke {
        Some(color) => {
            ui.painter()
                .rect_filled(badge, 8.0, theme::with_alpha(color, 0.14));
            ui.painter().rect_stroke(
                badge,
                8.0,
                Stroke::new(1.0, theme::with_alpha(color, 0.3)),
                StrokeKind::Inside,
            );
        }
        None => {
            ui.painter().rect_filled(badge, 8.0, theme::ink(10));
            ui.painter().rect_stroke(
                badge,
                8.0,
                Stroke::new(1.0, theme::ink(14)),
                StrokeKind::Inside,
            );
        }
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(15.0)),
        icon,
        tint,
    );
    badge
}

/// A saved script that is not on this chart yet: switching it on, or clicking its row, applies it.
pub(super) fn available_row(
    ui: &mut Ui,
    script: &Arc<LocalScript>,
    index: usize,
    count: usize,
    events: &mut Vec<RowEvent>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    if response.hovered() {
        hover_wash(ui, rect, index, count);
    }
    let strategy = script.script_type == ScriptType::Strategy;
    let badge = badge_tile(
        ui,
        rect,
        theme::text_tertiary(),
        if strategy {
            Icon::Cycle
        } else {
            Icon::ChartLine
        },
        None,
    );
    let switch = switch_at(
        ui,
        rect,
        Id::new(("script-switch", &script.id)),
        false,
        &script.name,
    );
    if switch.clicked() || response.clicked() {
        events.push(RowEvent::Enable(script.clone()));
    }
    let width = (switch.rect.min.x - 12.0 - (badge.max.x + 12.0)).max(40.0);
    let title = kit::galley(
        ui,
        &script.name,
        kit::size::SUBHEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let meta = kit::galley(
        ui,
        script.script_type.display_name(),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = rect.center().y - (title.size().y + 2.0 + meta.size().y) / 2.0;
    let left = badge.max.x + 12.0;
    ui.painter()
        .galley(pos2(left, top), title.clone(), theme::text_primary());
    ui.painter().galley(
        pos2(left, top + title.size().y + 2.0),
        meta,
        theme::text_secondary(),
    );
    crate::ui::accessibility::label(response, format!("Turn on {}", script.name));
}

/// One applied script: what it is, how it is doing, and what can be done to it.
pub(super) fn instance_row(
    ui: &mut Ui,
    row: &RowData,
    index: usize,
    count: usize,
    events: &mut Vec<RowEvent>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let failing = row.failure.is_some();
    let dim = if row.visible { 1.0 } else { 0.55 };

    if response.hovered() {
        hover_wash(ui, rect, index, count);
    }

    // A neutral tile, as on every settings card. Colour says something here: orange when the
    // script fails, the accent while it draws, and nothing once it is off.
    let kind_icon = if failing {
        Icon::TriangleFill
    } else if row.is_strategy {
        Icon::Cycle
    } else {
        Icon::ChartLine
    };
    let tint = if failing {
        theme::ORANGE
    } else if row.visible {
        theme::SYSTEM_ACCENT
    } else {
        theme::text_tertiary()
    };
    let badge = badge_tile(ui, rect, tint, kind_icon, failing.then_some(theme::ORANGE));

    // Controls, right to left: the switch, settings, remove.
    let switch = switch_at(
        ui,
        rect,
        Id::new(("instance-switch", row.id)),
        row.visible,
        &row.title,
    );
    if switch.clicked() {
        events.push(RowEvent::ToggleVisible(row.id));
    }
    let mut x = switch.rect.min.x - 6.0;
    let mut button = |ui: &mut Ui, icon: Icon, tip: &str, destructive: bool| -> egui::Response {
        let hit = Rect::from_center_size(pos2(x - 12.0, rect.center().y), Vec2::splat(24.0));
        x -= 26.0;
        let response = ui.interact(
            hit,
            Id::new(("instance-button", row.id, tip.to_owned())),
            Sense::click(),
        );
        if response.hovered() {
            ui.painter().rect_filled(hit, 6.0, theme::ink(18));
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let color = if destructive && response.hovered() {
            theme::syntax::string()
        } else {
            theme::text_secondary()
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(hit.center(), Vec2::splat(13.0)),
            icon,
            color,
        );
        response.on_hover_text(tip.to_owned())
    };
    let settings = button(
        ui,
        Icon::Sliders,
        &format!("Edit {} settings", row.title),
        false,
    );
    let anchor = settings.rect;
    if settings.clicked() {
        events.push(RowEvent::Configure(row.id, anchor));
    }
    if button(
        ui,
        Icon::Xmark,
        &format!("Remove {} from this chart", row.title),
        true,
    )
    .clicked()
    {
        events.push(RowEvent::Remove(row.id));
    }
    let text_right = x - 4.0;

    let title_width = (text_right - (badge.max.x + 12.0)).max(40.0);
    let title = kit::galley(
        ui,
        &row.title,
        kit::size::SUBHEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary().gamma_multiply(dim),
        title_width,
    );
    let meta_text = row.failure.clone().unwrap_or_else(|| {
        if row.evaluating {
            "Evaluating…".to_owned()
        } else {
            let mut parts = vec![
                if row.is_strategy {
                    "Strategy"
                } else {
                    "Indicator"
                }
                .to_owned(),
            ];
            if !row.is_strategy
                && let Some(overlay) = row.overlay
            {
                parts.push(if overlay { "On chart" } else { "Own pane" }.to_owned());
            }
            if !row.visible {
                parts.push("Off".to_owned());
            }
            parts.join(" · ")
        }
    });
    let meta_color = if failing {
        theme::ORANGE
    } else {
        theme::text_secondary()
    };
    // A failure takes the line; otherwise a script still running is marked with the indicator.
    let working = row.evaluating && !failing;
    let lead = if working {
        META_INDICATOR + META_GAP
    } else {
        0.0
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        meta_text,
        FontId::proportional(kit::size::CAPTION + 0.5),
        meta_color.gamma_multiply(dim),
    );
    job.wrap.max_width = (title_width - lead).max(0.0);
    job.wrap.max_rows = 1;
    let meta = ui.painter().layout_job(job);
    let top = rect.center().y - (title.size().y + 2.0 + meta.size().y) / 2.0;
    let left = badge.max.x + 12.0;
    ui.painter()
        .galley(pos2(left, top), title.clone(), theme::text_primary());
    let meta_top = top + title.size().y + 2.0;
    if working {
        LoadingIndicator::new().size(META_INDICATOR).paint_at(
            ui,
            Rect::from_center_size(
                pos2(left + META_INDICATOR / 2.0, meta_top + meta.size().y / 2.0),
                Vec2::splat(META_INDICATOR),
            ),
        );
    }
    ui.painter()
        .galley(pos2(left + lead, meta_top), meta, meta_color);

    response.context_menu(|ui| {
        let label = if row.visible { "Turn Off" } else { "Turn On" };
        if ui.button(label).clicked() {
            events.push(RowEvent::ToggleVisible(row.id));
            ui.close();
        }
        if ui.button("Settings…").clicked() {
            events.push(RowEvent::Configure(row.id, anchor));
            ui.close();
        }
        // Where the visuals sit against the chart's own candles: only a script drawing on the
        // chart has a choice.
        if row.overlay != Some(false) {
            ui.menu_button("Visual Order", |ui| {
                for (order, label) in [
                    (VisualOrder::Script, "As the Script Declares"),
                    (VisualOrder::Front, "Bring to Front"),
                    (VisualOrder::Behind, "Send to Back"),
                ] {
                    if ui.selectable_label(row.order == order, label).clicked() {
                        events.push(RowEvent::SetOrder(row.id, order));
                        ui.close();
                    }
                }
            });
        }
        if ui.button("Add Another Copy").clicked() {
            events.push(RowEvent::Duplicate(row.id));
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(row.position > 0, egui::Button::new("Move Up"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, row.position - 1));
            ui.close();
        }
        if ui
            .add_enabled(row.position + 1 < row.total, egui::Button::new("Move Down"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, row.position + 1));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new("Remove from Chart").color(theme::syntax::string()))
            .clicked()
        {
            events.push(RowEvent::Remove(row.id));
            ui.close();
        }
    });
}
