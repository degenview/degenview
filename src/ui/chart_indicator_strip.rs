//! Compact, Swift-style controls for the indicators applied to a chart.

use super::chart_card::ChartCard;
use super::icons::{self, Icon};
use super::kit::{self, size};
use super::popover;
use super::theme;
pub use crate::model::BuiltIn;
use crate::model::{DataSource, EMA_PERIODS, IndicatorConfig};
use eframe::egui::{self, Color32, FontId, Galley, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Kind {
    BuiltIn(BuiltIn),
    Script(u64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Add(BuiltIn),
    ToggleVisible(Kind),
    Remove(BuiltIn),
    EmaPeriod(usize),
    /// Open (or put away) a script's settings popover, pointing at the chip `Rect`.
    ConfigureScript(u64, Rect),
    OpenIndicatorSettings,
    OpenScriptSettings,
    /// Open the Pine logs pane on a script.
    OpenPineLogs(u64),
}

struct Item {
    kind: Kind,
    title: String,
    visible: bool,
    error: bool,
}

fn items(config: &IndicatorConfig, card: &ChartCard, source: DataSource) -> Vec<Item> {
    let mut items: Vec<Item> = BuiltIn::ALL
        .into_iter()
        .filter(|kind| !(source.is_line_chart() && *kind == BuiltIn::Volume))
        .filter(|kind| kind.applied(config))
        .map(|kind| Item {
            kind: Kind::BuiltIn(kind),
            title: kind.chip_title(config),
            visible: !kind.hidden(config),
            error: false,
        })
        .collect();
    if !source.is_line_chart() {
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
    }
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

pub fn has_items(config: &IndicatorConfig, card: &ChartCard, source: DataSource) -> bool {
    !items(config, card, source).is_empty()
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
const ADD_BUTTON: f32 = 20.0;
/// What a hovered chip grows by (its eye, settings and remove controls), held back when
/// deciding how many chips fit so hovering never makes the strip refold.
const CONTROLS_RESERVE: f32 = 4.0 + 3.0 * CONTROL_SIZE + 2.0;
const SPACING: f32 = 4.0;

impl BuiltIn {
    fn icon(self) -> Icon {
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
    CAPSULE_PADDING * 2.0
        + item.title_galley(ui, Color32::WHITE).size().x
        + if item.error { SPACING + 9.0 } else { 0.0 }
}

/// Returns a deferred edit so the workspace can mutate the chart after drawing it.
pub fn show(
    ui: &mut Ui,
    card: &ChartCard,
    config: &IndicatorConfig,
    source: DataSource,
    chart_id: u64,
    header_hovered: bool,
) -> Option<Action> {
    let items = items(config, card, source);
    let mut action = None;
    ui.spacing_mut().item_spacing.x = SPACING;
    let show_add = items.is_empty() || ui.available_width() >= 48.0;

    if !items.is_empty() {
        // Keep room for the add button and one chip's hover controls.
        let budget = ui.available_width()
            - if show_add { ADD_BUTTON + SPACING } else { 0.0 }
            - CONTROLS_RESERVE;
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
                + CAPSULE_PADDING * 2.0
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
                chip(ui, item, config, chart_id, false, &mut action);
            }
            if items.len() > inline {
                let overflow = &items[inline..];
                let label = format!("+{}", overflow.len());
                let pill = count_pill(ui, None, &label, "More indicators");
                overflow_list(ui, &pill, overflow, config, chart_id, &mut action);
            }
        } else {
            let label = items.len().to_string();
            let tooltip = format!("{} indicators", items.len());
            let pill = count_pill(ui, Some(Icon::ChartLine), &label, &tooltip);
            overflow_list(ui, &pill, &items, config, chart_id, &mut action);
        }
    }

    if show_add {
        add_button(
            ui,
            config,
            source,
            items.is_empty(),
            header_hovered,
            &mut action,
        );
    }
    action
}

/// The "+N" overflow pill, or the count pill with a chart glyph that stands in for every chip
/// when they do not fit. The returned response opens the list.
fn count_pill(ui: &mut Ui, icon: Option<Icon>, label: &str, tooltip: &str) -> egui::Response {
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
            CAPSULE_PADDING * 2.0 + glyph + text.size().x,
            CAPSULE_HEIGHT,
        ),
        Sense::click(),
    );
    let response = crate::ui::accessibility::label(response, tooltip)
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().rect_filled(
        rect,
        CAPSULE_HEIGHT / 2.0,
        kit::primary(if response.hovered() { 0.10 } else { 0.06 }),
    );
    let mut x = rect.left() + CAPSULE_PADDING;
    if let Some(icon) = icon {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 4.5, rect.center().y), Vec2::splat(9.0)),
            icon,
            kit::primary(0.7),
        );
        x += glyph;
    }
    ui.painter().galley(
        pos2(x, rect.center().y - text.size().y / 2.0),
        text,
        kit::primary(0.7),
    );
    response
}

/// Every chip in a popover pointing at `anchor`, one per row with its controls always shown. It
/// stays up while a chip's settings popover is open, so the indicators stay in view as they are
/// configured; a press elsewhere or Esc puts it away.
fn overflow_list(
    ui: &Ui,
    anchor: &egui::Response,
    items: &[Item],
    config: &IndicatorConfig,
    chart_id: u64,
    action: &mut Option<Action>,
) {
    const SIDE: f32 = 10.0;
    const INDENT: f32 = 6.0;
    let ctx = ui.ctx().clone();
    let open_id = anchor.id.with("overflow-open");
    let mut open = ctx
        .data(|data| data.get_temp::<bool>(open_id))
        .unwrap_or(false);
    if anchor.clicked() {
        open = !open;
    }
    if open {
        let widest = items
            .iter()
            .map(|item| {
                let controls = control_buttons(item).len() as f32;
                resting_width(ui, item) + SPACING + controls * CONTROL_SIZE + (controls - 1.0)
            })
            .fold(0.0, f32::max);
        let width = (widest + INDENT + 2.0 * SIDE + 2.0).max(170.0);
        let dismissed = popover::show(
            &ctx,
            anchor.id.with("overflow"),
            anchor.rect,
            width,
            SIDE as i8,
            false,
            |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
                ui.horizontal(|ui| {
                    ui.add_space(INDENT);
                    ui.label(kit::text(
                        "Indicators",
                        size::CAPTION + 1.0,
                        kit::SEMIBOLD,
                        theme::text_secondary(),
                    ));
                });
                let mut picked = None;
                for item in items {
                    ui.horizontal(|ui| {
                        ui.add_space(INDENT);
                        chip(ui, item, config, chart_id, true, &mut picked);
                    });
                }
                // A chip's settings hang off the whole list, so the list stays in view.
                *action = match picked {
                    Some(Action::ConfigureScript(id, _)) => Some(Action::ConfigureScript(
                        id,
                        ui.min_rect().expand(SIDE + 1.0),
                    )),
                    other => other.or(*action),
                };
            },
        );
        if dismissed && open_script_popover(&ctx).is_none_or(|(chart, _)| chart != chart_id) {
            open = false;
        }
    }
    ctx.data_mut(|data| data.insert_temp(open_id, open));
}

/// One applied indicator: a quiet name capsule that reveals its eye, settings and remove
/// controls while hovered. Name-only at rest so a full strip stays out of the way.
fn chip(
    ui: &mut Ui,
    item: &Item,
    config: &IndicatorConfig,
    chart_id: u64,
    always_controls: bool,
    action: &mut Option<Action>,
) {
    let id = ui.id().with(("indicator-chip", item.kind));
    let hovered = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    // The settings popup keeps the controls up while the pointer is inside it.
    let popup_open = egui::Popup::is_id_open(ui.ctx(), id.with("period"))
        || matches!(item.kind, Kind::Script(script) if open_script_popover(ui.ctx()) == Some((chart_id, script)));
    let controls = always_controls || hovered || popup_open;
    let opacity = if item.visible { 1.0 } else { 0.55 };

    let name = item.title_galley(ui, name_color(item.visible));
    let buttons = control_buttons(item);
    let controls_width = if controls {
        SPACING + buttons.len() as f32 * CONTROL_SIZE + (buttons.len() - 1) as f32
    } else {
        0.0
    };
    let content = name.size().x + if item.error { SPACING + 9.0 } else { 0.0 } + controls_width;
    let (rect, body) = ui.allocate_exact_size(
        vec2(content + CAPSULE_PADDING * 2.0, CAPSULE_HEIGHT),
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
            let response = ui
                .interact(slot, id.with(("control", index)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let label = format!("{} {}", button.label, item.title);
            let response =
                crate::ui::accessibility::label(response, label.clone()).on_hover_text(label);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(slot.center(), Vec2::splat(CONTROL_GLYPH)),
                button.icon,
                kit::primary(0.85 * opacity),
            );
            if response.clicked() {
                *action = Some(match button.action {
                    // The popover points at the whole chip, not the little button.
                    Action::ConfigureScript(id, _) => Action::ConfigureScript(id, rect),
                    other => other,
                });
            }
        }
    }

    let under_pointer = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pos| rect.contains(pos));
    ui.ctx()
        .data_mut(|data| data.insert_temp(id, under_pointer));
    body.context_menu(|ui| item_menu(ui, item, rect, config, action));
}

struct Control {
    icon: Icon,
    label: &'static str,
    action: Action,
}

/// The eye, the settings and the remove control, as far as this indicator has them.
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
    if let Kind::BuiltIn(kind) = item.kind {
        buttons.push(Control {
            icon: Icon::Xmark,
            label: "Remove",
            action: Action::Remove(kind),
        });
    }
    buttons
}

/// The header's "+": faint on an empty chart, shown only while the header is hovered once
/// there are indicators, so it never competes with the chart's own controls.
fn add_button(
    ui: &mut Ui,
    config: &IndicatorConfig,
    source: DataSource,
    is_empty: bool,
    header_hovered: bool,
    action: &mut Option<Action>,
) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ADD_BUTTON), Sense::click());
    let response = crate::ui::accessibility::label(response, "Add Indicator")
        .on_hover_text("Add Indicator")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered() || egui::Popup::is_id_open(ui.ctx(), response.id.with("add"));
    let opacity = if hovered {
        0.85
    } else if is_empty {
        if header_hovered { 0.6 } else { 0.35 }
    } else if header_hovered {
        0.5
    } else {
        0.0
    };
    if hovered {
        ui.painter()
            .circle_filled(rect.center(), ADD_BUTTON / 2.0, kit::primary(0.10));
    }
    if opacity > 0.0 {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(10.0)),
            Icon::Plus,
            theme::with_alpha(theme::text_primary(), opacity),
        );
    }
    egui::Popup::menu(&response)
        .id(response.id.with("add"))
        .frame(kit::menu_frame())
        .align(egui::RectAlign::BOTTOM_START)
        .gap(4.0)
        .show(|ui| {
            ui.set_width(210.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let addable: Vec<BuiltIn> = BuiltIn::ALL
                .into_iter()
                .filter(|kind| !(source.is_line_chart() && *kind == BuiltIn::Volume))
                .filter(|kind| !kind.applied(config))
                .collect();
            if !addable.is_empty() {
                menu_heading(ui, "Built-in");
                for kind in addable {
                    if kit::menu_item(ui, kind.icon(), kind.menu_title(), true, false) {
                        *action = Some(Action::Add(kind));
                        ui.close();
                    }
                }
                kit::menu_separator(ui);
            }
            if !source.is_line_chart()
                && kit::menu_item(ui, Icon::Curlybraces, "Script Manager…", true, false)
            {
                *action = Some(Action::OpenScriptSettings);
                ui.close();
            }
            if kit::menu_item(ui, Icon::Sliders, "All Indicator Settings…", true, false) {
                *action = Some(Action::OpenIndicatorSettings);
                ui.close();
            }
        });
}

/// A small section label in a menu.
fn menu_heading(ui: &mut Ui, title: &str) {
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
    if let Kind::BuiltIn(kind) = item.kind {
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Trash, "Remove", true, true) {
            *action = Some(Action::Remove(kind));
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let items = items(&config, &card, DataSource::Binance);
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
                        show(ui, card, config, DataSource::Binance, 0, true);
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
    fn chip_titles_and_controls_follow_the_indicator() {
        let item = |kind, visible| Item {
            kind,
            title: "x".into(),
            visible,
            error: false,
        };
        assert_eq!(
            control_buttons(&item(Kind::BuiltIn(BuiltIn::Rsi), true)).len(),
            2
        );
        assert_eq!(
            control_buttons(&item(Kind::BuiltIn(BuiltIn::Ema), true)).len(),
            3
        );
        assert_eq!(control_buttons(&item(Kind::Script(1), true)).len(), 2);
        assert_eq!(
            control_buttons(&item(Kind::Script(1), false))[0].action,
            Action::ToggleVisible(Kind::Script(1))
        );
        assert!(item(Kind::Script(1), true).has_settings());
        assert!(!item(Kind::BuiltIn(BuiltIn::Bollinger), true).has_settings());
    }
}
