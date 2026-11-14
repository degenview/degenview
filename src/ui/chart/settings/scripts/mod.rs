//! The Scripts page: the scripts on this chart and the saved ones to add, with search, counts and
//! the empty state.

use super::{ChartSettingsAction, ChartSettingsContext, ChartSettingsState};
use crate::model::pine::instance::{PineInstance, VisualOrder};
use crate::model::pine::script::{LocalScript, ScriptType};
use crate::pine_studio::drawer;
use crate::ui::chart::card::ChartCard;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::pine::script_catalog;
use crate::ui::settings::card;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Color32, FontId, Id, Layout, Margin, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use rows::{available_row, instance_row, row_card};
use std::sync::Arc;
use std::time::Instant;

mod rows;

/// What one row of the "On this chart" list shows, copied out so drawing needs no borrow of the
/// chart.
struct RowData {
    id: u64,
    title: String,
    is_strategy: bool,
    /// `None` until the script has run.
    overlay: Option<bool>,
    visible: bool,
    failure: Option<String>,
    evaluating: bool,
    order: VisualOrder,
    /// Where the instance sits among all of the chart's scripts, whatever the search shows.
    position: usize,
    total: usize,
}

pub(super) enum RowEvent {
    ToggleVisible(u64),
    Remove(u64),
    MoveTo(u64, usize),
    /// The script's settings, anchored to the control that asked.
    Configure(u64, Rect),
    /// Where the script's visuals sit against the chart's candles.
    SetOrder(u64, VisualOrder),
    /// Turn a saved script on for this chart (its first instance).
    Enable(Arc<LocalScript>),
    /// Apply the instance's script again, with default inputs.
    Duplicate(u64),
}

/// The list gets a search field once it has more rows than this.
const SEARCH_FROM: usize = 5;

pub(super) fn scripts_page(
    ui: &mut Ui,
    state: &mut ChartSettingsState,
    c: &mut ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let saved = script_catalog::scripts(ui.ctx());
    let instances: Vec<PineInstance> = c.card.pine_instances().to_vec();
    if instances.is_empty() && saved.is_empty() {
        card::section_title(ui, "Scripts", Some("Pine scripts running on this chart."));
        empty_state(ui, &mut action);
        return action;
    }
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            card::section_title(
                ui,
                "Scripts",
                Some("Switch a script on to run it on this chart."),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if kit::push_button(ui, Some(Icon::AngleBrackets), "Pine Studio", true)
                .on_hover_text("Write, edit and organise scripts")
                .clicked()
            {
                action = Some(ChartSettingsAction::OpenPineStudio);
            }
        });
    });

    let off_chart: Vec<&Arc<LocalScript>> = saved
        .iter()
        .filter(|script| {
            !instances
                .iter()
                .any(|i| i.script_id.as_deref() == Some(script.id.as_str()))
        })
        .collect();
    if instances.len() + off_chart.len() > SEARCH_FROM {
        ui.add_space(2.0);
        kit::search_field(
            ui,
            Id::new("chart-settings-script-search"),
            &mut state.script_query,
            "Search scripts",
            ui.available_width(),
        );
        ui.add_space(4.0);
    } else {
        // A filter nobody can see must not hide rows.
        state.script_query.clear();
    }
    let query = state.script_query.trim().to_lowercase();
    let mut available = script_catalog::matching(off_chart.iter().copied(), &query);
    if query.is_empty() {
        // Indicators, then strategies; each already by name.
        available.sort_by_key(|script| script.script_type == ScriptType::Strategy);
    }

    // Copy what the rows show out of the chart, so drawing needs no borrow of it.
    let now = Instant::now();
    let rows: Vec<RowData> = instances
        .iter()
        .enumerate()
        .filter(|(_, instance)| query.is_empty() || instance.name.to_lowercase().contains(&query))
        .map(|(position, instance)| {
            let status = c.card.pine_status(instance.id);
            RowData {
                id: instance.id,
                title: instance.name.clone(),
                is_strategy: status.as_ref().is_some_and(|s| s.output.strategy.is_some()),
                overlay: status.as_ref().map(|s| s.output.overlay),
                visible: instance.visible,
                failure: status
                    .as_ref()
                    .and_then(|s| s.error.map(|e| e.message.clone())),
                evaluating: instance.visible
                    && (status.is_none() || c.card.pine_busy(instance.id, now)),
                order: instance.visual_order,
                position,
                total: instances.len(),
            }
        })
        .collect();
    let bars = instances
        .iter()
        .filter_map(|i| c.card.pine_status(i.id))
        .map(|s| s.output.bar_count)
        .max()
        .filter(|bars| *bars > 0);

    let mut events: Vec<RowEvent> = Vec::new();
    if !rows.is_empty() {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(kit::text(
                "On this chart",
                kit::size::SUBHEADLINE + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            count_chip(ui, &rows.len().to_string());
            if let Some(bars) = bars {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status_badge(ui, &format!("{bars} bars"), theme::text_secondary());
                });
            }
        });
        ui.add_space(6.0);
        row_card(ui, rows.len(), |ui, index| {
            instance_row(ui, &rows[index], index, rows.len(), &mut events);
        });
    }
    if !available.is_empty() {
        ui.add_space(if rows.is_empty() { 12.0 } else { 20.0 });
        ui.horizontal(|ui| {
            ui.label(kit::text(
                if rows.is_empty() {
                    "Your scripts"
                } else {
                    "Available"
                },
                kit::size::SUBHEADLINE + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            count_chip(ui, &available.len().to_string());
        });
        ui.add_space(6.0);
        row_card(ui, available.len(), |ui, index| {
            available_row(ui, available[index], index, available.len(), &mut events);
        });
    }
    if rows.is_empty() && available.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(kit::text(
                format!("No scripts match “{}”", state.script_query.trim()),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
    }

    if let Some(asked) = apply_events(c.card, &instances, events) {
        action = Some(asked);
    }

    // One backtest report per applied strategy, so several never hide behind "the first one".
    let strategies: Vec<&PineInstance> = instances
        .iter()
        .filter(|i| {
            c.card
                .pine_status(i.id)
                .is_some_and(|s| s.output.strategy.is_some())
        })
        .collect();
    if !strategies.is_empty() {
        ui.add_space(20.0);
        card::section_title(ui, "Backtest", None);
        for instance in &strategies {
            if strategies.len() > 1 {
                ui.label(kit::text(
                    &instance.name,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::SEMIBOLD,
                    theme::text_secondary(),
                ));
            }
            if let Some(status) = c.card.pine_status(instance.id) {
                drawer::report_content(ui, Some(status.output));
            }
            ui.add_space(8.0);
        }
    }
    action
}

/// Applies what the rows asked for to the chart; returns the sheet-level action, if one asked.
pub(super) fn apply_events(
    card: &mut ChartCard,
    instances: &[PineInstance],
    events: Vec<RowEvent>,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    for event in events {
        match event {
            RowEvent::ToggleVisible(id) => {
                let visible = instances
                    .iter()
                    .find(|i| i.id == id)
                    .is_some_and(|i| i.visible);
                card.set_pine_visible(id, !visible);
            }
            RowEvent::Remove(id) => {
                card.remove_pine_instance(id);
            }
            RowEvent::MoveTo(id, index) => card.move_pine_instance(id, index),
            RowEvent::SetOrder(id, order) => card.set_pine_visual_order(id, order),
            RowEvent::Configure(id, anchor) => {
                action = Some(ChartSettingsAction::ConfigureScript(id, anchor));
            }
            RowEvent::Enable(script) => {
                card.add_pine_instance(Some(script.id.clone()), &script.name, &script.source);
            }
            RowEvent::Duplicate(id) => {
                if let Some(instance) = instances.iter().find(|i| i.id == id) {
                    card.add_pine_instance(
                        instance.script_id.clone(),
                        &instance.name,
                        &instance.source,
                    );
                }
            }
        }
    }
    action
}

fn count_chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::monospace(10.0),
        theme::text_secondary(),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::quaternary(1.0));
    ui.painter().galley(
        pos2(rect.min.x + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_secondary(),
    );
}

fn status_badge(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(10.0), color);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 10.0, theme::with_alpha(color, 0.16));
    ui.painter().galley(
        pos2(rect.min.x + 9.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

/// `ScriptsEmptyState`: a disc glyph, a title, a message and an optional call to action on a
/// dashed card.
fn empty_state(ui: &mut Ui, action: &mut Option<ChartSettingsAction>) {
    ui.add_space(12.0);
    let card = egui::Frame::new()
        .fill(theme::card_fill(false))
        .corner_radius(kit::CARD_RADIUS)
        .inner_margin(Margin::symmetric(24, 36))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.vertical_centered(|ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter()
                    .circle_filled(disc.center(), 24.0, theme::quaternary(1.3));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(disc.center(), Vec2::splat(20.0)),
                    Icon::AngleBrackets,
                    theme::text_secondary(),
                );
                ui.add_space(12.0);
                let (title, message) = (
                    "No scripts yet",
                    "Write one in the Pine Studio and it shows up here.",
                );
                ui.label(kit::text(
                    title,
                    kit::size::HEADLINE + 1.0,
                    kit::BOLD,
                    theme::text_primary(),
                ));
                ui.add_space(3.0);
                ui.label(kit::text(
                    message,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.add_space(12.0);
                if kit::sheet_button(ui, "Open Pine Studio", true, true).clicked() {
                    *action = Some(ChartSettingsAction::OpenPineStudio);
                }
            });
        });
    dashed_rounded_rect(
        ui.painter(),
        card.response.rect,
        kit::CARD_RADIUS,
        Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.9)),
    );
}

/// A rounded rectangle outlined with `[5, 4]` dashes, as `StrokeStyle(dash:)` draws one.
fn dashed_rounded_rect(painter: &egui::Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let rect = rect.shrink(stroke.width / 2.0);
    let corners = [
        (pos2(rect.left() + radius, rect.top() + radius), 180.0_f32),
        (pos2(rect.right() - radius, rect.top() + radius), 270.0),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
    ];
    let mut points = Vec::new();
    for (center, start) in corners {
        for step in 0..=8 {
            let angle = (start + 90.0 * step as f32 / 8.0).to_radians();
            points.push(center + radius * vec2(angle.cos(), angle.sin()));
        }
    }
    points.push(points[0]);
    painter.extend(egui::Shape::dashed_line(&points, stroke, 5.0, 4.0));
}
