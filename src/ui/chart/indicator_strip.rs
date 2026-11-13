//! Compact, Swift-style controls for the indicators applied to a chart.

use super::card::ChartCard;
pub use crate::model::BuiltIn;
use crate::model::overlay::{OverlayCommand, OverlayId};
use crate::model::{EMA_PERIODS, IndicatorConfig};
use crate::pine_studio::indicators::BundledIndicator;
use crate::pine_studio::model::LocalScript;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::popover;
use crate::ui::kit::{self, size};
use crate::ui::theme;
use eframe::egui::{self, Color32, FontId, Galley, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Kind {
    BuiltIn(BuiltIn),
    Script(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Add(BuiltIn),
    /// Apply a saved script to the chart, after the ones already there.
    AddScript(Arc<LocalScript>),
    /// Apply an indicator bundled with the app to the chart, after the ones already there.
    AddBundled(&'static BundledIndicator),
    ToggleVisible(Kind),
    Remove(BuiltIn),
    /// Take a script off the chart (undoable; the saved script stays).
    RemoveScript(u64),
    EmaPeriod(usize),
    /// Open (or put away) a script's settings popover, pointing at the chip `Rect`.
    ConfigureScript(u64, Rect),
    OpenIndicatorSettings,
    /// Open the Pine logs pane on a script.
    OpenPineLogs(u64),
    /// Open (or put away) a compared market's settings popover, pointing at its row `Rect` on
    /// the chart's overlay card.
    ConfigureOverlay(OverlayId, Rect),
    /// Change a compared market.
    Overlay(OverlayCommand),
}

struct Item {
    kind: Kind,
    title: String,
    visible: bool,
    error: bool,
}

fn items(config: &IndicatorConfig, card: &ChartCard) -> Vec<Item> {
    let mut items: Vec<Item> = BuiltIn::ALL
        .into_iter()
        .filter(|kind| kind.applied(config))
        .map(|kind| Item {
            kind: Kind::BuiltIn(kind),
            title: kind.chip_title(config),
            visible: !kind.hidden(config),
            error: false,
        })
        .collect();
    items.extend(card.pine_instances().iter().map(|instance| {
        Item {
            kind: Kind::Script(instance.id),
            title: instance.name.clone(),
            visible: instance.visible,
            error: card
                .pine_status(instance.id)
                .is_some_and(|status| status.error.is_some()),
        }
    }));
    items
}

/// Which script's settings popover is up, as (chart id, instance id), so its chip keeps its
/// controls showing meanwhile. Instance ids are per chart, so the chart has to be part of it.
fn open_script_popover_id() -> egui::Id {
    egui::Id::new("open-script-popover")
}

pub fn set_open_script_popover(ctx: &egui::Context, script: Option<(u64, u64)>) {
    ctx.data_mut(|data| data.insert_temp(open_script_popover_id(), script));
}

fn open_script_popover(ctx: &egui::Context) -> Option<(u64, u64)> {
    ctx.data(|data| data.get_temp::<Option<(u64, u64)>>(open_script_popover_id()))
        .flatten()
}

pub fn has_items(config: &IndicatorConfig, card: &ChartCard) -> bool {
    !items(config, card).is_empty()
}

// MARK: View

/// The strip shows this many chips before folding the rest into a "+N" pill.
const VISIBLE_CHIP_LIMIT: usize = 3;
/// Capsule height and the padding either side of its content (Swift: `.frame(height: 20)`,
/// `.padding(.horizontal, 8)`).
const CAPSULE_HEIGHT: f32 = 20.0;
const CAPSULE_PADDING: f32 = 8.0;
const CHIP_TEXT_SIZE: f32 = 10.0;
const CHIP_NAME_MAX_WIDTH: f32 = 110.0;
/// A hover control is a 16pt square with a 10pt glyph, 1pt apart.
const CONTROL_SIZE: f32 = 16.0;
const CONTROL_GLYPH: f32 = 10.0;
/// What a hovered chip grows by (its eye, settings and remove controls), held back when
/// deciding how many chips fit so hovering never makes the strip refold.
const CONTROLS_RESERVE: f32 = 4.0 + 2.0 * CONTROL_SIZE + 1.0;
/// The x at a chip's right end: its slot, the glyph in it, the gap before it and the padding after.
const REMOVE_GLYPH: f32 = 8.0;
const REMOVE_PADDING: f32 = 4.0;
/// The overflow pill's chevron: its size, the room it takes after the label, the padding after it.
const PILL_CHEVRON: f32 = 8.0;
const PILL_CHEVRON_ROOM: f32 = SPACING + PILL_CHEVRON;
const PILL_END_PADDING: f32 = 6.0;
const REMOVE_ROOM: f32 = SPACING + CONTROL_SIZE;
/// The overflow list's rows, controls and size.
const LIST_WIDTH: f32 = 256.0;
const LIST_ROW: f32 = 32.0;
const LIST_BUTTON: f32 = 22.0;
const LIST_SCROLLS_FROM: usize = 8;
const LIST_MAX_HEIGHT: f32 = 300.0;
const SPACING: f32 = 4.0;

impl BuiltIn {
    pub(in crate::ui) fn icon(self) -> Icon {
        match self {
            Self::Volume => Icon::BarChart,
            Self::Rsi => Icon::Waveform,
            Self::Ema => Icon::ChartUptrend,
            Self::Bollinger => Icon::Bands,
            Self::TrendFlips => Icon::Cycle,
        }
    }
}

impl Item {
    /// An item with a title and nothing else to say.
    #[cfg(test)]
    fn bare(kind: Kind, title: &str, visible: bool) -> Self {
        Self {
            kind,
            title: title.to_owned(),
            visible,
            error: false,
        }
    }

    fn has_settings(&self) -> bool {
        matches!(self.kind, Kind::Script(_) | Kind::BuiltIn(BuiltIn::Ema))
    }

    fn title_galley(&self, ui: &Ui, visible_color: Color32) -> Arc<Galley> {
        kit::galley(
            ui,
            &self.title,
            CHIP_TEXT_SIZE,
            kit::MEDIUM,
            visible_color,
            CHIP_NAME_MAX_WIDTH,
        )
    }
}

/// `Color.primary.opacity(0.7)` for a showing indicator, secondary for a hidden one.
fn name_color(visible: bool) -> Color32 {
    if visible {
        kit::primary(0.7)
    } else {
        theme::text_secondary()
    }
}

/// The resting width of a chip: its name, a warning if it has one, and the capsule padding.
fn resting_width(ui: &Ui, item: &Item) -> f32 {
    CAPSULE_PADDING
        + item.title_galley(ui, Color32::WHITE).size().x
        + if item.error { SPACING + 9.0 } else { 0.0 }
        + REMOVE_ROOM
        + REMOVE_PADDING
}

/// Returns a deferred edit so the workspace can mutate the chart after drawing it.
pub fn show(
    ui: &mut Ui,
    card: &ChartCard,
    config: &IndicatorConfig,
    chart_id: u64,
) -> Option<Action> {
    let items = items(config, card);
    let mut action = None;
    ui.spacing_mut().item_spacing.x = SPACING;

    if !items.is_empty() {
        // Keep room for one chip's hover controls.
        let budget = ui.available_width() - CONTROLS_RESERVE;
        let inline = items.len().min(VISIBLE_CHIP_LIMIT);
        let pill_width = |label: &str| {
            ui.painter()
                .layout_no_wrap(
                    label.to_owned(),
                    FontId::proportional(CHIP_TEXT_SIZE),
                    Color32::WHITE,
                )
                .size()
                .x
                + CAPSULE_PADDING
                + PILL_CHEVRON_ROOM
                + PILL_END_PADDING
        };
        let widths: Vec<f32> = items.iter().map(|item| resting_width(ui, item)).collect();
        let chips_fit = {
            let chips: f32 =
                widths.iter().take(inline).sum::<f32>() + SPACING * inline.saturating_sub(1) as f32;
            let overflow = items.len() - inline;
            let pill = if overflow > 0 {
                SPACING + pill_width(&format!("+{overflow}"))
            } else {
                0.0
            };
            chips + pill <= budget
        };
        if chips_fit {
            for item in items.iter().take(inline) {
                chip(ui, item, config, chart_id, &mut action);
            }
            if items.len() > inline {
                let overflow = &items[inline..];
                let label = format!("+{}", overflow.len());
                let tooltip = format!(
                    "{} more indicator{}",
                    overflow.len(),
                    if overflow.len() == 1 { "" } else { "s" }
                );
                let pill = count_pill(ui, None, &label, &tooltip);
                overflow_list(ui, &pill, overflow, config, chart_id, &mut action);
            }
        } else {
            let label = items.len().to_string();
            let tooltip = format!("{} indicators", items.len());
            let pill = count_pill(ui, Some(Icon::ChartLine), &label, &tooltip);
            overflow_list(ui, &pill, &items, config, chart_id, &mut action);
        }
    }

    action
}

/// The "+N" overflow pill, or the count pill with a chart glyph that stands in for every chip
/// when they do not fit. It ends in a chevron that points up while the list is open. The returned
/// response opens the list.
fn count_pill(ui: &mut Ui, icon: Option<Icon>, label: &str, tooltip: &str) -> egui::Response {
    // The folded pill (with a chart glyph) already says what it is and has the least room.
    let chevron = icon.is_none();
    let text = kit::galley(
        ui,
        label,
        CHIP_TEXT_SIZE,
        kit::MEDIUM,
        kit::primary(0.7),
        f32::INFINITY,
    );
    let glyph = if icon.is_some() { 9.0 + SPACING } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            CAPSULE_PADDING
                + glyph
                + text.size().x
                + if chevron {
                    PILL_CHEVRON_ROOM + PILL_END_PADDING
                } else {
                    CAPSULE_PADDING
                },
            CAPSULE_HEIGHT,
        ),
        Sense::click(),
    );
    let open = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(response.id.with("overflow-open")))
        .unwrap_or(false);
    let response = crate::ui::accessibility::label(response, tooltip)
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if open {
        0.15
    } else if response.hovered() {
        0.10
    } else {
        0.06
    };
    ui.painter()
        .rect_filled(rect, CAPSULE_HEIGHT / 2.0, kit::primary(fill));
    let ink = kit::primary(if open || response.hovered() { 0.9 } else { 0.7 });
    let mut x = rect.left() + CAPSULE_PADDING;
    if let Some(icon) = icon {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 4.5, rect.center().y), Vec2::splat(9.0)),
            icon,
            ink,
        );
        x += glyph;
    }
    ui.painter().galley(
        pos2(x, rect.center().y - text.size().y / 2.0),
        text.clone(),
        ink,
    );
    if chevron {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(
                    x + text.size().x + SPACING + PILL_CHEVRON / 2.0,
                    rect.center().y,
                ),
                Vec2::splat(PILL_CHEVRON),
            ),
            if open {
                Icon::ChevronUp
            } else {
                Icon::ChevronDown
            },
            theme::with_alpha(ink, 0.8),
        );
    }
    response
}

/// How an [`icon_button`] is drawn: its icon and the icon's size, the colour at rest, and whether
/// it is destructive.
struct Look {
    icon: Icon,
    glyph: f32,
    tint: Color32,
    danger: bool,
}

/// A small round icon button: `glyph` points of `icon` in `tint`, with a wash behind it under the
/// pointer (red for a `danger` button). `slot` is the hit area.
fn icon_button(ui: &mut Ui, slot: Rect, id: egui::Id, look: Look, label: &str) -> egui::Response {
    let Look {
        icon,
        glyph,
        tint,
        danger,
    } = look;
    let response = ui
        .interact(slot, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, label).on_hover_text(label);
    let hovered = response.hovered();
    if hovered {
        let wash = if danger {
            kit::RED.gamma_multiply(0.18)
        } else {
            kit::primary(0.12)
        };
        ui.painter()
            .circle_filled(slot.center(), slot.width().min(slot.height()) / 2.0, wash);
    }
    let ink = match (danger, hovered) {
        (true, true) => kit::RED,
        (_, true) => kit::primary(0.95),
        _ => tint,
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(slot.center(), Vec2::splat(glyph)),
        icon,
        ink,
    );
    response
}

/// What removes `item` from the chart.
fn remove_action(item: &Item) -> Action {
    match item.kind {
        Kind::BuiltIn(kind) => Action::Remove(kind),
        Kind::Script(id) => Action::RemoveScript(id),
    }
}

/// Every chip in a popover pointing at `anchor`: a header with the count, then one row per
/// indicator, built-ins before scripts, each with its controls. It stays up while a chip's
/// settings popover is open, so the indicators stay in view as they are configured; a press
/// elsewhere or Esc puts it away.
fn overflow_list(
    ui: &Ui,
    anchor: &egui::Response,
    items: &[Item],
    config: &IndicatorConfig,
    chart_id: u64,
    action: &mut Option<Action>,
) {
    const SIDE: f32 = 6.0;
    let ctx = ui.ctx().clone();
    let open_id = anchor.id.with("overflow-open");
    let mut open = ctx
        .data(|data| data.get_temp::<bool>(open_id))
        .unwrap_or(false);
    if anchor.clicked() {
        open = !open;
    }
    if open {
        let dismissed = popover::show(
            &ctx,
            anchor.id.with("overflow"),
            anchor.rect,
            LIST_WIDTH + 2.0 * SIDE,
            SIDE as i8,
            false,
            |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                list_header(ui, "Indicators", items.len());
                let mut picked = None;
                let rows = |ui: &mut Ui, picked: &mut Option<Action>| {
                    let group_of = |item: &Item| match item.kind {
                        Kind::BuiltIn(_) => 0,
                        Kind::Script(_) => 1,
                    };
                    let groups = (0..2)
                        .filter(|group| items.iter().any(|i| group_of(i) == *group))
                        .count();
                    for (heading, wanted) in [("Built-in", 0), ("Scripts", 1)] {
                        let group: Vec<&Item> =
                            items.iter().filter(|i| group_of(i) == wanted).collect();
                        if group.is_empty() {
                            continue;
                        }
                        if groups > 1 {
                            kit::menu_heading(ui, heading);
                        }
                        for item in group {
                            list_row(ui, item, config, chart_id, picked);
                        }
                    }
                };
                if items.len() > LIST_SCROLLS_FROM {
                    egui::ScrollArea::vertical()
                        .max_height(LIST_MAX_HEIGHT)
                        .min_scrolled_height(0.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| rows(ui, &mut picked));
                } else {
                    rows(ui, &mut picked);
                }
                // A chip's settings hang off the whole list, so the list stays in view.
                *action = match picked {
                    Some(Action::ConfigureScript(id, _)) => Some(Action::ConfigureScript(
                        id,
                        ui.min_rect().expand(SIDE + 1.0),
                    )),
                    other => other.or_else(|| action.take()),
                };
            },
        );
        if dismissed && open_script_popover(&ctx).is_none_or(|(chart, _)| chart != chart_id) {
            open = false;
        }
    }
    ctx.data_mut(|data| data.insert_temp(open_id, open));
}

/// The list's title and how many rows it has.
fn list_header(ui: &mut Ui, title: &str, count: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let title = kit::galley(
        ui,
        title,
        size::CAPTION + 2.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        rect.width() - 60.0,
    );
    let left = rect.left() + 14.0;
    ui.painter().galley(
        pos2(left, rect.center().y - title.size().y / 2.0),
        title.clone(),
        theme::text_primary(),
    );
    let number = kit::galley(
        ui,
        &count.to_string(),
        CHIP_TEXT_SIZE,
        kit::MEDIUM,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let badge = Rect::from_min_size(
        pos2(left + title.size().x + 6.0, rect.center().y - 8.0),
        vec2(number.size().x + 12.0, 16.0),
    );
    ui.painter().rect_filled(badge, 8.0, kit::primary(0.08));
    ui.painter().galley(
        pos2(
            badge.center().x - number.size().x / 2.0,
            badge.center().y - number.size().y / 2.0,
        ),
        number,
        theme::text_secondary(),
    );
}

/// One row of the overflow list: a glyph tile, the name, a warning if the script failed, and the
/// eye, settings and remove controls. Clicking the name does what clicking the chip's does.
fn list_row(
    ui: &mut Ui,
    item: &Item,
    config: &IndicatorConfig,
    chart_id: u64,
    action: &mut Option<Action>,
) {
    let id = ui.id().with(("indicator-row", item.kind));
    let (rect, body) = ui.allocate_exact_size(vec2(ui.available_width(), LIST_ROW), Sense::click());
    let dim = if item.visible { 1.0 } else { 0.5 };
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("period"))
        || matches!(item.kind, Kind::Script(script) if open_script_popover(ui.ctx()) == Some((chart_id, script)));
    if body.hovered() || open {
        ui.painter().rect_filled(rect, 7.0, kit::primary(0.07));
    }

    // Controls, right to left: remove, settings, eye.
    let mut x = rect.right() - 6.0;
    let mut place = |ui: &mut Ui, index: usize, icon: Icon, label: String, danger: bool| {
        let slot = Rect::from_center_size(
            pos2(x - LIST_BUTTON / 2.0, rect.center().y),
            Vec2::splat(LIST_BUTTON),
        );
        x -= LIST_BUTTON + 2.0;
        icon_button(
            ui,
            slot,
            id.with(("control", index)),
            Look {
                icon,
                glyph: 12.0,
                tint: theme::with_alpha(theme::text_secondary(), if danger { 0.7 } else { dim }),
                danger,
            },
            &label,
        )
    };
    if place(ui, 2, Icon::Xmark, format!("Remove {}", item.title), true).clicked() {
        *action = Some(remove_action(item));
    }
    for control in control_buttons(item).into_iter().rev() {
        let response = place(
            ui,
            usize::from(control.label == "Settings"),
            control.icon,
            format!("{} {}", control.label, item.title),
            false,
        );
        if response.clicked() {
            *action = Some(match control.action {
                Action::ConfigureScript(id, _) => Action::ConfigureScript(id, rect),
                other => other,
            });
        }
    }
    let text_right = x - 2.0;

    // The glyph tile and the name.
    let tile = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(22.0));
    ui.painter()
        .rect_filled(tile, 6.0, kit::primary(0.06 * dim.max(0.6)));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(tile.center(), Vec2::splat(12.0)),
        match item.kind {
            Kind::BuiltIn(kind) => kind.icon(),
            Kind::Script(_) => Icon::AngleBrackets,
        },
        theme::with_alpha(theme::text_secondary(), dim.max(0.6)),
    );
    let name_left = tile.right() + 9.0;
    let warning = if item.error { 9.0 + SPACING } else { 0.0 };
    let name = kit::galley(
        ui,
        &item.title,
        size::BODY - 0.5,
        kit::MEDIUM,
        theme::with_alpha(theme::text_primary(), dim),
        (text_right - name_left - warning).max(40.0),
    );
    let name_width = name.size().x;
    ui.painter().galley(
        pos2(name_left, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    if item.error {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(name_left + name_width + SPACING + 4.5, rect.center().y),
                Vec2::splat(9.0),
            ),
            Icon::TriangleFill,
            kit::ORANGE,
        );
    }

    let body = crate::ui::accessibility::label(
        body,
        if item.has_settings() {
            format!("{} settings", item.title)
        } else {
            format!(
                "{}, {}",
                item.title,
                if item.visible { "shown" } else { "hidden" }
            )
        },
    );
    if body.clicked() {
        match item.kind {
            Kind::BuiltIn(BuiltIn::Ema) => {}
            Kind::Script(script) => *action = Some(Action::ConfigureScript(script, rect)),
            Kind::BuiltIn(_) => *action = Some(Action::ToggleVisible(item.kind)),
        }
    }
    if item.kind == Kind::BuiltIn(BuiltIn::Ema) {
        egui::Popup::menu(&body)
            .id(id.with("period"))
            .frame(kit::menu_frame())
            .gap(6.0)
            .show(|ui| ema_menu(ui, config, action));
    }
    item_context_menu(ui, &body, item, rect, config, action);
}

/// One applied indicator: a quiet name capsule with a small x at its right end that removes it,
/// and an eye and settings control that appear while it is hovered. A full strip stays calm.
fn chip(
    ui: &mut Ui,
    item: &Item,
    config: &IndicatorConfig,
    chart_id: u64,
    action: &mut Option<Action>,
) {
    let id = ui.id().with(("indicator-chip", item.kind));
    let hovered = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    // The settings popup keeps the controls up while the pointer is inside it.
    let popup_open = egui::Popup::is_id_open(ui.ctx(), id.with("period"))
        || matches!(item.kind, Kind::Script(script) if open_script_popover(ui.ctx()) == Some((chart_id, script)));
    let controls = hovered || popup_open;
    let opacity = if item.visible { 1.0 } else { 0.55 };

    let name = item.title_galley(ui, name_color(item.visible));
    let buttons = control_buttons(item);
    let controls_width = if controls {
        SPACING + buttons.len() as f32 * CONTROL_SIZE + (buttons.len() - 1) as f32
    } else {
        0.0
    };
    let content =
        name.size().x + if item.error { SPACING + 9.0 } else { 0.0 } + controls_width + REMOVE_ROOM;
    let (rect, body) = ui.allocate_exact_size(
        vec2(CAPSULE_PADDING + content + REMOVE_PADDING, CAPSULE_HEIGHT),
        Sense::click(),
    );
    let fill = if hovered || popup_open { 0.10 } else { 0.06 };
    ui.painter()
        .rect_filled(rect, CAPSULE_HEIGHT / 2.0, kit::primary(fill * opacity));

    // The name: opens the settings when there are any, otherwise it just shows or hides.
    let name_rect = Rect::from_min_size(
        pos2(rect.left() + CAPSULE_PADDING, rect.top()),
        vec2(name.size().x, CAPSULE_HEIGHT),
    );
    let name_response = ui
        .interact(name_rect, id.with("name"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let name_response = crate::ui::accessibility::label(
        name_response,
        if item.has_settings() {
            format!("{} settings", item.title)
        } else {
            format!(
                "{}, {}",
                item.title,
                if item.visible { "shown" } else { "hidden" }
            )
        },
    );
    let name_response = if item.has_settings() {
        name_response.on_hover_text(format!("Edit {}", item.title))
    } else {
        name_response
    };
    let mut x = name_rect.right();
    ui.painter().galley(
        pos2(name_rect.left(), rect.center().y - name.size().y / 2.0),
        name,
        name_color(item.visible),
    );
    if name_response.clicked() {
        match item.kind {
            Kind::BuiltIn(BuiltIn::Ema) => {}
            Kind::Script(script) => *action = Some(Action::ConfigureScript(script, rect)),
            Kind::BuiltIn(_) => *action = Some(Action::ToggleVisible(item.kind)),
        }
    }
    if item.kind == Kind::BuiltIn(BuiltIn::Ema) {
        egui::Popup::menu(&name_response)
            .id(id.with("period"))
            .frame(kit::menu_frame())
            .gap(6.0)
            .show(|ui| ema_menu(ui, config, action));
    }
    if item.error {
        x += SPACING;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 4.5, rect.center().y), Vec2::splat(9.0)),
            Icon::TriangleFill,
            kit::ORANGE,
        );
        x += 9.0;
    }
    if controls {
        x += SPACING;
        for (index, button) in buttons.iter().enumerate() {
            let slot = Rect::from_min_size(
                pos2(
                    x + index as f32 * (CONTROL_SIZE + 1.0),
                    rect.center().y - CONTROL_SIZE / 2.0,
                ),
                Vec2::splat(CONTROL_SIZE),
            );
            let label = format!("{} {}", button.label, item.title);
            let response = icon_button(
                ui,
                slot,
                id.with(("control", index)),
                Look {
                    icon: button.icon,
                    glyph: CONTROL_GLYPH,
                    tint: kit::primary(0.85 * opacity),
                    danger: false,
                },
                &label,
            );
            if response.clicked() {
                *action = Some(match button.action.clone() {
                    // The popover points at the whole chip, not the little button.
                    Action::ConfigureScript(id, _) => Action::ConfigureScript(id, rect),
                    other => other,
                });
            }
        }
    }

    // The x: faint at rest so a strip of chips stays quiet, firmer while the chip is hovered.
    let remove = Rect::from_center_size(
        pos2(
            rect.right() - REMOVE_PADDING - CONTROL_SIZE / 2.0,
            rect.center().y,
        ),
        Vec2::splat(CONTROL_SIZE),
    );
    let strength = if controls { 0.6 } else { 0.32 };
    let label = format!("Remove {}", item.title);
    if icon_button(
        ui,
        remove,
        id.with("remove"),
        Look {
            icon: Icon::Xmark,
            glyph: REMOVE_GLYPH,
            tint: kit::primary(strength * opacity),
            danger: true,
        },
        &label,
    )
    .clicked()
    {
        *action = Some(remove_action(item));
    }

    let under_pointer = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pos| rect.contains(pos));
    ui.ctx()
        .data_mut(|data| data.insert_temp(id, under_pointer));
    item_context_menu(ui, &body, item, rect, config, action);
}

struct Control {
    icon: Icon,
    label: &'static str,
    action: Action,
}

/// The eye and, where the indicator has settings, the settings control. Removing is the x.
fn control_buttons(item: &Item) -> Vec<Control> {
    let mut buttons = vec![Control {
        icon: if item.visible {
            Icon::Eye
        } else {
            Icon::EyeSlash
        },
        label: if item.visible { "Hide" } else { "Show" },
        action: Action::ToggleVisible(item.kind),
    }];
    match item.kind {
        Kind::BuiltIn(BuiltIn::Ema) => buttons.push(Control {
            icon: Icon::Sliders,
            label: "Settings",
            action: Action::OpenIndicatorSettings,
        }),
        Kind::Script(id) => buttons.push(Control {
            icon: Icon::Sliders,
            label: "Settings",
            action: Action::ConfigureScript(id, Rect::NOTHING),
        }),
        Kind::BuiltIn(_) => {}
    }
    buttons
}

/// A small section label in a menu.
pub(crate) fn menu_heading(ui: &mut Ui, title: &str) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(kit::text(
            title,
            size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_tertiary(),
        ));
    });
    ui.add_space(2.0);
}

fn ema_menu(ui: &mut Ui, config: &IndicatorConfig, action: &mut Option<Action>) {
    ui.set_width(130.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    menu_heading(ui, "EMA period");
    for period in EMA_PERIODS {
        if kit::menu_item_marked(
            ui,
            Icon::Check,
            &period.to_string(),
            config.ema_period == period,
        ) {
            *action = Some(Action::EmaPeriod(period));
            ui.close();
        }
    }
}

/// The right-click menu of a chip or list row.
fn item_context_menu(
    _ui: &Ui,
    body: &egui::Response,
    item: &Item,
    chip: Rect,
    config: &IndicatorConfig,
    action: &mut Option<Action>,
) {
    body.context_menu(|ui| item_menu(ui, item, chip, config, action));
}

fn item_menu(
    ui: &mut Ui,
    item: &Item,
    chip: Rect,
    config: &IndicatorConfig,
    action: &mut Option<Action>,
) {
    ui.set_width(180.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let (icon, label) = if item.visible {
        (Icon::EyeSlash, "Hide")
    } else {
        (Icon::Eye, "Show")
    };
    if kit::menu_item(ui, icon, label, true, false) {
        *action = Some(Action::ToggleVisible(item.kind));
        ui.close();
    }
    match item.kind {
        Kind::BuiltIn(BuiltIn::Ema) => {
            kit::menu_separator(ui);
            ema_menu(ui, config, action);
        }
        Kind::Script(id) => {
            if kit::menu_item(ui, Icon::Sliders, "Settings…", true, false) {
                *action = Some(Action::ConfigureScript(id, chip));
                ui.close();
            }
            if kit::menu_item(ui, Icon::DocText, "Pine logs", true, false) {
                *action = Some(Action::OpenPineLogs(id));
                ui.close();
            }
        }
        Kind::BuiltIn(_) => {}
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Remove", true, true) {
        *action = Some(remove_action(item));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    #[test]
    fn built_in_visibility_keeps_the_applied_chip() {
        let mut config = IndicatorConfig {
            show_ema: true,
            ..IndicatorConfig::default()
        };
        BuiltIn::Ema.toggle_visible(&mut config);
        assert!(BuiltIn::Ema.applied(&config));
        assert!(!config.drawn().show_ema);
        BuiltIn::Ema.toggle_visible(&mut config);
        assert!(config.drawn().show_ema);
    }

    #[test]
    fn strip_contains_hidden_built_ins_and_applied_scripts() {
        let config = IndicatorConfig {
            show_ema: true,
            hide_ema: true,
            ..IndicatorConfig::default()
        };
        let mut card = ChartCard::default();
        let script_id = card.add_pine_instance(None, "My script", "plot(close)");
        let items = items(&config, &card);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "EMA 20");
        assert!(!items[0].visible);
        assert_eq!(items[1].kind, Kind::Script(script_id));
    }

    #[test]
    fn older_indicator_configs_default_to_visible() {
        let json = r#"{"show_volume":false,"show_rsi":false,"show_ema":true,"ema_period":50,"show_bollinger":false,"show_trend_flips":false}"#;
        let config: IndicatorConfig = serde_json::from_str(json).unwrap();
        assert!(config.drawn().show_ema);
        assert!(!config.hide_ema);
    }

    /// Draws the strip in a row `width` wide and returns how wide it came out.
    fn drawn_width(config: &IndicatorConfig, card: &ChartCard, width: f32) -> f32 {
        let ctx = egui::Context::default();
        let mut used = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.allocate_ui_with_layout(
                    vec2(width, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let start = ui.cursor().left();
                        show(ui, card, config, 0);
                        used = ui.min_rect().right() - start;
                    },
                );
            });
            output.textures_delta.clear();
        }
        used
    }

    #[test]
    fn the_strip_draws_empty_full_folded_and_squeezed_and_never_overflows_its_row() {
        let empty = IndicatorConfig::default();
        let full = IndicatorConfig {
            show_volume: true,
            show_rsi: true,
            show_ema: true,
            show_bollinger: true,
            show_trend_flips: true,
            hide_rsi: true,
            ..IndicatorConfig::default()
        };
        let mut card = ChartCard::default();
        card.add_pine_instance(None, "A very long script name indeed", "plot(close)");
        for (config, width) in [
            (&empty, 200.0),
            (&full, 520.0),
            (&full, 260.0),
            (&full, 120.0),
            (&full, 64.0),
        ] {
            let used = drawn_width(config, &card, width);
            assert!(used <= width + 0.5, "{used} overflows a {width} row");
        }
    }

    #[test]
    fn the_x_removes_a_built_in_or_a_script_and_every_chip_has_one() {
        let item = |kind| Item::bare(kind, "x", true);
        assert_eq!(
            remove_action(&item(Kind::BuiltIn(BuiltIn::Rsi))),
            Action::Remove(BuiltIn::Rsi)
        );
        assert_eq!(
            remove_action(&item(Kind::Script(7))),
            Action::RemoveScript(7)
        );
        // The x is part of a chip's resting width, so the strip's fit maths leaves room for it.
        let ctx = egui::Context::default();
        let mut width = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            width = resting_width(ui, &item(Kind::Script(1)));
            let name = item(Kind::Script(1))
                .title_galley(ui, Color32::WHITE)
                .size()
                .x;
            assert!(width >= name + CAPSULE_PADDING + REMOVE_ROOM);
        });
        output.textures_delta.clear();
        assert!(width > 0.0);
    }

    #[test]
    fn clicking_the_count_pill_opens_a_list_with_every_indicator_and_its_controls() {
        use crate::ui::render_tests::Screen;
        let config = IndicatorConfig {
            show_volume: true,
            show_rsi: true,
            show_ema: true,
            show_bollinger: true,
            show_trend_flips: true,
            ..IndicatorConfig::default()
        };
        let mut card = ChartCard::default();
        card.add_pine_instance(Some("a".into()), "Squeeze Momentum", "plot(close)");
        let mut screen = Screen::new(egui::vec2(560.0, 420.0));
        let mut picked = None;
        let mut frame = |screen: &mut Screen, events: Vec<egui::Event>| {
            screen.run(events, |ui| {
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    if let Some(action) = show(ui, &card, &config, 1) {
                        picked = Some(action);
                    }
                });
            })
        };
        let last = frame(&mut screen, Vec::new());
        let pill = text_pos(&last, "+3").expect("three chips fold into a pill");
        let click = |pressed| egui::Event::PointerButton {
            pos: pill,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut screen, vec![egui::Event::PointerMoved(pill)]);
        frame(&mut screen, vec![click(true)]);
        frame(&mut screen, vec![click(false)]);
        let mut last = frame(&mut screen, Vec::new());
        for _ in 0..3 {
            last = frame(&mut screen, Vec::new());
        }
        for wanted in ["Indicators", "BB", "Trend Flips", "Squeeze Momentum"] {
            assert!(text_pos(&last, wanted).is_some(), "the list shows {wanted}");
        }
        assert_eq!(picked, None, "opening the list changes nothing");
    }

    #[test]
    fn chip_titles_and_controls_follow_the_indicator() {
        let item = |kind, visible| Item::bare(kind, "x", visible);
        assert_eq!(
            control_buttons(&item(Kind::BuiltIn(BuiltIn::Rsi), true)).len(),
            1
        );
        assert_eq!(
            control_buttons(&item(Kind::BuiltIn(BuiltIn::Ema), true)).len(),
            2
        );
        assert_eq!(control_buttons(&item(Kind::Script(1), true)).len(), 2);
        assert_eq!(
            control_buttons(&item(Kind::Script(1), false))[0].action,
            Action::ToggleVisible(Kind::Script(1))
        );
        assert!(item(Kind::Script(1), true).has_settings());
        assert!(!item(Kind::BuiltIn(BuiltIn::Bollinger), true).has_settings());
    }

    fn text_pos(output: &egui::FullOutput, wanted: &str) -> Option<egui::Pos2> {
        fn walk(shape: &egui::Shape, wanted: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| walk(shape, wanted)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|clipped| walk(&clipped.shape, wanted))
    }

    /// The strip at rest, with a chip hovered, and with the overflow list open, as pictures. Run by
    /// hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_strip() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let config = IndicatorConfig {
            show_volume: true,
            show_rsi: true,
            show_ema: true,
            show_bollinger: true,
            show_trend_flips: true,
            hide_rsi: true,
            ..IndicatorConfig::default()
        };
        let mut card = ChartCard::default();
        card.add_pine_instance(Some("a".into()), "Squeeze Momentum", "plot(close)");
        card.add_pine_instance(Some("b".into()), "Liquidity Sweeps Pro", "plot(close)");
        let mut screen = Screen::new(egui::vec2(560.0, 420.0));
        let frame = |screen: &mut Screen, events: Vec<egui::Event>| {
            screen.run(events, |ui| {
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    show(ui, &card, &config, 1);
                });
            })
        };
        let mut last = frame(&mut screen, Vec::new());
        for _ in 0..3 {
            last = frame(&mut screen, Vec::new());
        }
        screen.save("strip-rest", last);

        let mut last = frame(&mut screen, Vec::new());
        let first = text_pos(&last, "Vol").expect("first chip");
        last = frame(&mut screen, vec![egui::Event::PointerMoved(first)]);
        for _ in 0..3 {
            last = frame(&mut screen, Vec::new());
        }
        screen.save("strip-hover", last);

        let mut last = frame(
            &mut screen,
            vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0))],
        );
        let pill = text_pos(&last, "+4").expect("overflow pill");
        let click = |pressed| egui::Event::PointerButton {
            pos: pill,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut screen, vec![egui::Event::PointerMoved(pill)]);
        frame(&mut screen, vec![click(true)]);
        frame(&mut screen, vec![click(false)]);
        for _ in 0..4 {
            last = frame(&mut screen, Vec::new());
        }
        screen.save("strip-overflow", last);
    }

    fn card_comparing(symbols: &[&str]) -> ChartCard {
        use crate::model::overlay::OverlayList;
        use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
        let main = crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let mut list = OverlayList::new();
        for symbol in symbols {
            list.add(
                &main,
                crate::model::InstrumentId::new(DataSource::Binance, *symbol),
                None,
            );
        }
        let feeds = (0..list.len())
            .map(|_| OverlayFeed::empty(OverlayStatus::Ready))
            .collect();
        let mut card = ChartCard::default();
        card.set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
        card
    }

    #[test]
    fn compared_markets_are_not_chips() {
        // They are rows of the chart's overlay card (with the eye, settings and remove), so the
        // strip lists only the indicators and scripts however many markets are compared.
        let config = IndicatorConfig {
            show_ema: true,
            ..IndicatorConfig::default()
        };
        let symbols: Vec<String> = (0..60).map(|n| format!("S{n}USDT")).collect();
        let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let mut card = card_comparing(&refs);
        let script = card.add_pine_instance(None, "My script", "plot(close)");
        let kinds: Vec<_> = items(&config, &card).iter().map(|item| item.kind).collect();
        assert_eq!(
            kinds,
            vec![Kind::BuiltIn(BuiltIn::Ema), Kind::Script(script)]
        );
        for width in [600.0, 140.0, 64.0] {
            let used = drawn_width(&config, &card, width);
            assert!(used <= width + 0.5, "{used} overflows a {width} row");
        }
    }
}
