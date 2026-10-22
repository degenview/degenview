//! The vertical tool strip down the left edge of a chart window (Swift `ToolSidebar`): the
//! drawing tools at the top; Paper Trading, Replay and the alerts bell at the bottom.

use crate::model::{MagnetMode, Tool, ToolGroup};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Color32, Id, Pos2, Rect, RectAlign, Sense, SetOpenCommand, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};
use std::time::Duration;

pub const WIDTH: f32 = 44.0;
const BUTTON: f32 = 34.0;
const ICON: f32 = 18.0;
const SPACING: f32 = 6.0;
/// How long the pointer rests on a tile before its label appears beside the strip.
const TOOLTIP_DELAY: Duration = Duration::from_millis(250);

pub struct ToolSidebarInput {
    pub active_tool: Tool,
    /// The tool the grouped brushes, arrows and shapes tile shows and arms: the last one used.
    pub group_tool: Tool,
    pub magnet: MagnetMode,
    pub keep_drawing: bool,
    pub hide_all: bool,
    /// Every drawing on the tab is locked (and there is at least one).
    pub all_locked: bool,
    /// There is at least one drawing to lock or hide.
    pub has_drawings: bool,
    pub paper_open: bool,
    pub replay_open: bool,
    /// Replay needs a market chart to step through.
    pub has_market: bool,
    /// Alerts fired since the Alerts window was last looked at.
    pub unseen_alerts: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSidebarAction {
    /// Arm `tool`, or disarm it when it already is (back to the crosshair).
    Select(Tool),
    /// Arm `tool` (from the grouped menu), even when it already is.
    Arm(Tool),
    SetMagnet(MagnetMode),
    ToggleKeepDrawing,
    ToggleLockAll,
    ToggleHideAll,
    TogglePaper,
    ToggleReplay,
    OpenAlerts,
}

/// How an active tile is drawn: a lit accent tile for an armed tool, or a tinted glyph on a
/// soft wash for a toggle (a panel open, replay running).
#[derive(Clone, Copy, Eq, PartialEq)]
enum Active {
    Off,
    Filled,
    Tinted,
}

struct Tile<'a> {
    icon: Icon,
    label: &'a str,
    tooltip: &'a str,
    active: Active,
    tint: Color32,
    enabled: bool,
    badge: usize,
    /// Identity of the tile when its tooltip changes with its state; empty uses the tooltip.
    key: &'a str,
    /// A small corner mark showing the tile opens a menu.
    notch: bool,
    /// A one-letter caption in the corner (the magnet's strength).
    mark: Option<&'a str>,
}

/// A tile at the top of the strip: one tool, or the grouped brushes, arrows and shapes.
#[derive(Clone, Copy)]
enum Slot {
    Tool(Tool, Icon, &'static str),
    Group,
}

const SLOTS: [Slot; 5] = [
    Slot::Tool(Tool::Cursor, Icon::Plus, "Crosshair"),
    Slot::Tool(Tool::TrendLine, Icon::LineDiagonal, "Trend line"),
    Slot::Tool(Tool::Fib, Icon::FibPoints, "Fib Retracement"),
    Slot::Group,
    Slot::Tool(Tool::Ruler, Icon::Ruler, "Ruler"),
];

/// The glyph a tool has on the strip and in the grouped menu.
pub fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Cursor => Icon::Plus,
        Tool::TrendLine => Icon::LineDiagonal,
        Tool::Fib => Icon::FibPoints,
        Tool::Brush => Icon::Paintbrush,
        Tool::Ruler => Icon::Ruler,
        Tool::Highlighter => Icon::Highlighter,
        Tool::ArrowMarker => Icon::ArrowMarkerTool,
        Tool::Arrow => Icon::ArrowTool,
        Tool::ArrowMarkUp => Icon::ArrowMarkUp,
        Tool::ArrowMarkDown => Icon::ArrowMarkDown,
        Tool::Rectangle => Icon::ShapeRectangle,
        Tool::RotatedRectangle => Icon::ShapeRotatedRect,
        Tool::Path => Icon::ShapePath,
        Tool::Circle => Icon::ShapeCircle,
        Tool::Ellipse => Icon::ShapeEllipse,
        Tool::Polyline => Icon::ShapePolyline,
        Tool::Triangle => Icon::ShapeTriangle,
        Tool::Arc => Icon::ShapeArc,
        Tool::Curve => Icon::ShapeCurve,
        Tool::DoubleCurve => Icon::ShapeDoubleCurve,
        // Drawn properly with the menus.
        _ => Icon::LineDiagonal,
    }
}

/// Whether `tool` lives in the grouped menu.
fn in_group_menu(tool: Tool) -> bool {
    ToolGroup::MENU.contains(&tool.spec().group)
}

/// The tooltip a tile shows, separate from its accessibility label where Swift's differs.
pub fn replay_tooltip(replay_open: bool, has_market: bool) -> &'static str {
    if replay_open {
        "Close Replay"
    } else if has_market {
        "Historical Replay"
    } else {
        "Historical Replay — add a market chart first"
    }
}

pub fn show(ui: &mut Ui, input: &ToolSidebarInput) -> Vec<ToolSidebarAction> {
    let mut actions = Vec::new();
    let strip = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(strip, 0.0, theme::chrome());

    let x = strip.center().x - BUTTON / 2.0;
    let mut y = strip.top() + 10.0;
    for slot in SLOTS {
        let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
        match slot {
            Slot::Tool(tool, icon, label) => {
                let tile = Tile {
                    icon,
                    label,
                    tooltip: label,
                    // The crosshair is the resting state, so like Swift's default nothing is lit
                    // until a drawing tool is armed; clicking it disarms whichever one is.
                    active: if input.active_tool == tool && tool != Tool::Cursor {
                        Active::Filled
                    } else {
                        Active::Off
                    },
                    tint: kit::ACCENT,
                    enabled: true,
                    badge: 0,
                    key: "",
                    notch: false,
                    mark: None,
                };
                if tile_button(ui, rect, &tile, strip.right()).clicked() {
                    actions.push(ToolSidebarAction::Select(tool));
                }
            }
            Slot::Group => group_tile(ui, rect, input, strip.right(), &mut actions),
        }
        y += BUTTON + SPACING;
    }

    // The drawing options: magnet, keep drawing, lock and hide. They take the room the bottom
    // group leaves; on a short window the last ones are dropped (their shortcuts and the plot's
    // menu still reach them).
    let limit = strip.bottom() - 135.0 - 10.0;
    y += 2.0;
    ui.painter().rect_filled(
        Rect::from_center_size(pos2(strip.center().x, y - SPACING / 2.0), vec2(22.0, 1.0)),
        0.5,
        theme::separator(),
    );
    y += SPACING / 2.0 + 2.0;
    let toggle = |icon, label, tooltip, key, active_on: bool, enabled, mark| Tile {
        icon,
        label,
        tooltip,
        active: if active_on {
            Active::Tinted
        } else {
            Active::Off
        },
        tint: kit::ACCENT,
        enabled,
        badge: 0,
        key,
        notch: false,
        mark,
    };
    let options = [
        (
            toggle(
                Icon::Magnet,
                "Magnet",
                input.magnet.label(),
                "magnet",
                input.magnet != MagnetMode::Off,
                true,
                match input.magnet {
                    MagnetMode::Off => None,
                    MagnetMode::Weak => Some("W"),
                    MagnetMode::Strong => Some("S"),
                },
            ),
            ToolSidebarAction::SetMagnet(input.magnet.next()),
        ),
        (
            toggle(
                Icon::KeepDrawing,
                "Keep drawing",
                if input.keep_drawing {
                    "Keep drawing: on (stay on the tool)"
                } else {
                    "Keep drawing: off (back to the crosshair)"
                },
                "keep",
                input.keep_drawing,
                true,
                None,
            ),
            ToolSidebarAction::ToggleKeepDrawing,
        ),
        (
            toggle(
                if input.all_locked {
                    Icon::LockFill
                } else {
                    Icon::LockOpen
                },
                "Lock all drawings",
                if input.all_locked {
                    "Unlock all drawings"
                } else {
                    "Lock all drawings"
                },
                "lock",
                input.all_locked,
                input.has_drawings,
                None,
            ),
            ToolSidebarAction::ToggleLockAll,
        ),
        (
            toggle(
                if input.hide_all {
                    Icon::EyeSlash
                } else {
                    Icon::Eye
                },
                "Hide all drawings",
                if input.hide_all {
                    "Show all drawings"
                } else {
                    "Hide all drawings"
                },
                "hide",
                input.hide_all,
                input.has_drawings || input.hide_all,
                None,
            ),
            ToolSidebarAction::ToggleHideAll,
        ),
    ];
    for (tile, action) in options {
        if y + BUTTON > limit {
            break;
        }
        let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
        if tile_button(ui, rect, &tile, strip.right()).clicked() {
            actions.push(action);
        }
        y += BUTTON + SPACING;
    }

    // The bottom group, laid out upwards from the strip's foot.
    let mut y = strip.bottom() - 10.0 - BUTTON;
    let bell = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    let bell_label = if input.unseen_alerts > 0 {
        format!("View Price Alerts, {} new", input.unseen_alerts)
    } else {
        "View Price Alerts".to_owned()
    };
    if tile_button(
        ui,
        bell,
        &Tile {
            icon: Icon::Bell,
            label: &bell_label,
            tooltip: "View Price Alerts",
            active: Active::Off,
            tint: kit::ACCENT,
            enabled: true,
            badge: input.unseen_alerts,
            key: "",
            notch: false,
            mark: None,
        },
        strip.right(),
    )
    .clicked()
    {
        actions.push(ToolSidebarAction::OpenAlerts);
    }

    // A short inset rule between the groups.
    y -= SPACING + 1.0 + 2.0;
    ui.painter().rect_filled(
        Rect::from_center_size(pos2(strip.center().x, y + 0.5), vec2(22.0, 1.0)),
        0.5,
        theme::separator(),
    );
    y -= 2.0 + SPACING + BUTTON;

    let replay = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    if tile_button(
        ui,
        replay,
        &Tile {
            icon: Icon::ClockArrow,
            label: if input.replay_open {
                "Close historical bar replay"
            } else {
                "Historical bar replay"
            },
            tooltip: replay_tooltip(input.replay_open, input.has_market),
            active: if input.replay_open {
                Active::Tinted
            } else {
                Active::Off
            },
            tint: theme::REPLAY,
            enabled: input.has_market || input.replay_open,
            badge: 0,
            key: "",
            notch: false,
            mark: None,
        },
        strip.right(),
    )
    .clicked()
    {
        actions.push(ToolSidebarAction::ToggleReplay);
    }
    y -= SPACING + BUTTON;

    let paper = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    if tile_button(
        ui,
        paper,
        &Tile {
            icon: Icon::ArrowLeftRight,
            label: if input.paper_open {
                "Hide Paper Trading panel"
            } else {
                "Show Paper Trading panel"
            },
            tooltip: if input.paper_open {
                "Hide Paper Trading"
            } else {
                "Paper Trading"
            },
            active: if input.paper_open {
                Active::Tinted
            } else {
                Active::Off
            },
            tint: kit::ACCENT,
            enabled: true,
            badge: 0,
            key: "",
            notch: false,
            mark: None,
        },
        strip.right(),
    )
    .clicked()
    {
        actions.push(ToolSidebarAction::TogglePaper);
    }

    actions
}

/// The tile for the brushes, arrows and shapes: it shows the last tool used from them, arms it
/// on a click, and opens the grouped menu from its corner, a right-click or the keyboard.
fn group_tile(
    ui: &mut Ui,
    rect: Rect,
    input: &ToolSidebarInput,
    strip_right: f32,
    actions: &mut Vec<ToolSidebarAction>,
) {
    let tool = input.group_tool;
    let label = tool.label();
    let tile = Tile {
        icon: tool_icon(tool),
        label,
        tooltip: label,
        active: if in_group_menu(input.active_tool) {
            Active::Filled
        } else {
            Active::Off
        },
        tint: kit::ACCENT,
        enabled: true,
        badge: 0,
        key: "group",
        notch: true,
        mark: None,
    };
    let response = tile_button(ui, rect, &tile, strip_right);
    if response.clicked() {
        actions.push(ToolSidebarAction::Select(tool));
    }
    // The corner opens the menu instead of arming the tool.
    let corner = Rect::from_min_size(rect.right_bottom() - vec2(16.0, 16.0), vec2(16.0, 16.0));
    let corner_response = ui.interact(corner, Id::new("tool-sidebar-group-menu"), Sense::click());
    let corner_response =
        crate::ui::accessibility::label(corner_response, "More brushes, arrows and shapes");
    let wants_open = corner_response.clicked()
        || response.secondary_clicked()
        || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::ArrowRight)));
    let popup = egui::Popup::menu(&response)
        .open_memory(wants_open.then_some(SetOpenCommand::Toggle))
        .align(RectAlign::RIGHT_START)
        .align_alternatives(&[RectAlign::RIGHT_END])
        .gap(10.0)
        .frame(kit::menu_frame())
        .width(MENU_WIDTH);
    popup.show(|ui| group_menu(ui, input, actions));
}

const MENU_WIDTH: f32 = 224.0;
/// The height of the whole list: three headings and sixteen rows.
const MENU_HEIGHT: f32 = 556.0;

/// The grouped menu: BRUSHES, ARROWS and SHAPES, one row per tool. Rows are focusable, so the
/// arrow keys move through them and Return picks one.
fn group_menu(ui: &mut Ui, input: &ToolSidebarInput, actions: &mut Vec<ToolSidebarAction>) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    // Room from where the menu starts down to the window's foot.
    let room = (ui.ctx().content_rect().bottom() - ui.next_widget_position().y - 16.0).max(160.0);
    let max_height = room.min(MENU_HEIGHT);
    // A popup's area starts out short; the menu says how tall it wants to be.
    ui.set_max_height(max_height);
    ui.set_min_height(max_height);
    let mut first = true;
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for group in ToolGroup::MENU {
                if !first {
                    ui.add_space(6.0);
                }
                first = false;
                ui.add_space(4.0);
                let heading = group.title().to_uppercase();
                ui.label(kit::text(
                    heading,
                    kit::size::CAPTION,
                    kit::SEMIBOLD,
                    theme::text_tertiary(),
                ));
                ui.add_space(3.0);
                for tool in Tool::in_group(group) {
                    let selected = input.active_tool == tool;
                    let response = menu_row(ui, tool, selected);
                    // Land the keyboard on the armed tool (or the first row) when it opens.
                    let seen = Id::new("tool-sidebar-menu-focused");
                    let landed = ui.data(|data| data.get_temp::<bool>(seen).unwrap_or(false));
                    if !landed
                        && (selected
                            || (input.active_tool == Tool::Cursor && tool == input.group_tool))
                    {
                        response.request_focus();
                        ui.data_mut(|data| data.insert_temp(seen, true));
                    }
                    if response.clicked() {
                        actions.push(ToolSidebarAction::Arm(tool));
                        ui.data_mut(|data| data.remove::<bool>(seen));
                        ui.close();
                    }
                }
            }
        });
    if !ui.ctx().memory(|memory| memory.focused().is_some()) {
        // Nothing took the focus: allow the next opening to place it again.
        ui.data_mut(|data| data.remove::<bool>(Id::new("tool-sidebar-menu-focused")));
    }
}

/// One row of the grouped menu: glyph, name and a check on the armed tool.
fn menu_row(ui: &mut Ui, tool: Tool, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let response = crate::ui::accessibility::label(response, tool.label());
    let highlighted = response.hovered() || response.has_focus();
    let painter = ui.painter();
    if highlighted {
        painter.rect_filled(rect, 6.0, kit::ACCENT);
    }
    let color = if highlighted {
        Color32::WHITE
    } else {
        theme::text_primary()
    };
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(17.0)),
        tool_icon(tool),
        color,
    );
    let galley = kit::galley(
        ui,
        tool.label(),
        kit::size::BODY,
        kit::REGULAR,
        color,
        width - 72.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 38.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if selected {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(rect.right() - 16.0, rect.center().y),
                Vec2::splat(12.0),
            ),
            Icon::Check,
            color,
        );
    }
    response
}

/// The tile every strip control shares: glyph, hover wash, press feedback, active styles, the
/// red count badge and the label that slides out beside the strip.
fn tile_button(ui: &mut Ui, rect: Rect, tile: &Tile, strip_right: f32) -> egui::Response {
    let id = Id::new((
        "tool-sidebar",
        if tile.key.is_empty() {
            tile.tooltip
        } else {
            tile.key
        },
    ));
    let sense = if tile.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let response = crate::ui::accessibility::label(response, tile.label);
    let hovered = response.hovered() && tile.enabled;
    let pressed = response.is_pointer_button_down_on() && tile.enabled;

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.expand(8.0)));
    if !tile.enabled {
        child.multiply_opacity(0.35);
    }
    let painter = child.painter();
    // Pressed tiles shrink a touch, like the Swift `scaleEffect(0.94)`.
    let shape = if pressed {
        Rect::from_center_size(rect.center(), rect.size() * 0.94)
    } else {
        rect
    };
    let lit = tile.active == Active::Filled;
    let (fill, glyph) = match tile.active {
        Active::Filled => (
            if pressed {
                tile.tint.gamma_multiply(0.85)
            } else {
                tile.tint
            },
            Color32::WHITE,
        ),
        Active::Tinted => (
            theme::with_alpha(
                tile.tint,
                if pressed {
                    0.26
                } else if hovered {
                    0.22
                } else {
                    0.16
                },
            ),
            tile.tint,
        ),
        Active::Off => (
            if pressed {
                kit::primary(0.14)
            } else if hovered {
                kit::primary(0.08)
            } else {
                Color32::TRANSPARENT
            },
            if hovered || pressed {
                theme::text_primary()
            } else {
                kit::primary(0.75)
            },
        ),
    };
    if lit {
        painter.add(
            egui::Shadow {
                offset: [0, 2],
                blur: 10,
                spread: 0,
                color: theme::with_alpha(tile.tint, 0.35),
            }
            .as_shape(shape, 10.0),
        );
    }
    painter.rect_filled(shape, 10.0, fill);
    if lit {
        // A faint rim, brightest along the top edge, like the Swift gradient stroke.
        painter.rect_stroke(
            shape,
            10.0,
            Stroke::new(1.0, Color32::from_white_alpha(18)),
            StrokeKind::Inside,
        );
        painter.hline(
            (shape.left() + 7.0)..=(shape.right() - 7.0),
            shape.top() + 0.5,
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
    }
    icons::paint(
        painter,
        Rect::from_center_size(
            shape.center(),
            Vec2::splat(ICON * if pressed { 0.94 } else { 1.0 }),
        ),
        tile.icon,
        glyph,
    );
    if tile.notch {
        // A small corner triangle: this tile opens a menu.
        let corner = shape.right_bottom() - vec2(4.0, 4.0);
        painter.add(egui::Shape::convex_polygon(
            vec![corner, corner - vec2(5.0, 0.0), corner - vec2(0.0, 5.0)],
            glyph.gamma_multiply(0.7),
            Stroke::NONE,
        ));
    }
    if let Some(mark) = tile.mark {
        painter.text(
            shape.right_bottom() - vec2(5.0, 4.0),
            egui::Align2::RIGHT_BOTTOM,
            mark,
            egui::FontId::proportional(9.0),
            glyph,
        );
    }
    if tile.badge > 0 {
        badge(
            &child,
            rect.right_top() + vec2(5.0 - 8.0, -5.0 + 8.0),
            tile.badge,
        );
    }

    tooltip(ui, id, &response, rect, strip_right, tile.tooltip);
    response
}

/// The red count bubble, centred on `at` (the tile's top-right corner, nudged outwards).
fn badge(ui: &Ui, at: Pos2, count: usize) {
    let text = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let galley = kit::digits(
        ui,
        &text,
        10.0,
        kit::SEMIBOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    let size = vec2((galley.size().x + 8.0).max(16.0), 16.0);
    let bubble = Rect::from_center_size(at, size);
    let painter = ui.painter();
    painter.rect_filled(bubble.expand(1.5), 9.5, theme::chrome());
    painter.rect_filled(bubble, 8.0, kit::RED);
    painter.galley(
        bubble.center() - galley.size() / 2.0,
        galley,
        Color32::WHITE,
    );
}

/// The label that appears beside the strip once the pointer has rested on a tile.
fn tooltip(ui: &Ui, id: Id, response: &egui::Response, tile: Rect, strip_right: f32, label: &str) {
    let since = id.with("hover-since");
    let now = ui.input(|input| input.time);
    if !response.hovered() {
        ui.ctx().data_mut(|data| data.remove::<f64>(since));
        return;
    }
    let started = ui
        .ctx()
        .data_mut(|data| *data.get_temp_mut_or_insert_with(since, || now));
    let waited = now - started;
    if waited < TOOLTIP_DELAY.as_secs_f64() {
        ui.ctx().request_repaint_after(Duration::from_secs_f64(
            TOOLTIP_DELAY.as_secs_f64() - waited,
        ));
        return;
    }
    let galley = kit::galley(
        ui,
        label,
        kit::size::BODY - 1.0,
        kit::REGULAR,
        theme::text_primary(),
        360.0,
    );
    let size = galley.size() + vec2(24.0, 14.0);
    let rect = Rect::from_min_size(
        pos2(strip_right + 8.0, tile.center().y - size.y / 2.0),
        size,
    );
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("tooltip")));
    painter.add(
        egui::Shadow {
            offset: [0, 3],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(36),
        }
        .as_shape(rect, 8.0),
    );
    painter.rect_filled(
        rect,
        8.0,
        theme::pick(
            Color32::from_rgb(48, 57, 62),
            Color32::from_rgb(250, 250, 251),
        ),
    );
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(0.5, kit::primary(0.12)),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(12.0, 7.0), galley, theme::text_primary());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_tooltips_say_what_a_click_does() {
        assert_eq!(replay_tooltip(true, true), "Close Replay");
        assert_eq!(replay_tooltip(false, true), "Historical Replay");
        assert_eq!(
            replay_tooltip(false, false),
            "Historical Replay — add a market chart first"
        );
    }

    #[test]
    fn the_strip_draws_headlessly_in_every_state() {
        let ctx = egui::Context::default();
        for (tool, paper, replay, market, unseen) in [
            (Tool::Cursor, false, false, true, 0),
            (Tool::Fib, true, true, true, 3),
            (Tool::Ruler, false, false, false, 120),
        ] {
            let state = ToolSidebarInput {
                active_tool: tool,
                group_tool: Tool::Brush,
                magnet: MagnetMode::Weak,
                keep_drawing: true,
                hide_all: false,
                all_locked: false,
                has_drawings: true,
                paper_open: paper,
                replay_open: replay,
                has_market: market,
                unseen_alerts: unseen,
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &state).is_empty(), "nothing was clicked");
            });
            output.textures_delta.clear();
        }
    }

    fn state() -> ToolSidebarInput {
        ToolSidebarInput {
            active_tool: Tool::Cursor,
            group_tool: Tool::Brush,
            magnet: MagnetMode::Off,
            keep_drawing: true,
            hide_all: false,
            all_locked: false,
            has_drawings: true,
            paper_open: false,
            replay_open: false,
            has_market: true,
            unseen_alerts: 0,
        }
    }

    /// Runs the strip in a screen of `size`, feeding it `events`, and returns what it asked for
    /// and the grouped menu's rectangle if it is open.
    fn frame(
        ctx: &egui::Context,
        time: &mut f64,
        size: Vec2,
        events: Vec<egui::Event>,
        input: &ToolSidebarInput,
    ) -> (Vec<ToolSidebarAction>, Option<Rect>) {
        *time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(*time),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, size.y))),
                |ui| actions = show(ui, input),
            );
        });
        output.textures_delta.clear();
        let menu = egui::Popup::is_any_open(ctx)
            .then(|| {
                ctx.memory(|memory| {
                    memory
                        .layer_ids()
                        .filter(|layer| layer.order == egui::Order::Foreground)
                        .find_map(|layer| memory.area_rect(layer.id))
                })
            })
            .flatten();
        (actions, menu)
    }

    fn click(
        ctx: &egui::Context,
        time: &mut f64,
        size: Vec2,
        at: Pos2,
        input: &ToolSidebarInput,
    ) -> Vec<ToolSidebarAction> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, time, size, vec![egui::Event::PointerMoved(at)], input);
        frame(ctx, time, size, vec![button(true)], input);
        frame(ctx, time, size, vec![button(false)], input).0
    }

    /// The grouped tile's corner, where the menu opens from.
    const CORNER: Pos2 = pos2(31.0, 156.0);

    #[test]
    fn the_grouped_menu_opens_from_the_corner_lists_every_tool_and_arms_the_pick() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        // A click on the tile itself arms its tool and opens nothing.
        let actions = click(&ctx, &mut time, size, pos2(14.0, 140.0), &input);
        assert_eq!(actions, vec![ToolSidebarAction::Select(Tool::Brush)]);
        assert!(frame(&ctx, &mut time, size, Vec::new(), &input).1.is_none());

        // The corner opens the menu, beside the strip and whole on a tall window.
        let actions = click(&ctx, &mut time, size, CORNER, &input);
        assert!(actions.is_empty());
        let mut menu = None;
        for _ in 0..3 {
            menu = frame(&ctx, &mut time, size, Vec::new(), &input).1;
        }
        let menu = menu.expect("the menu is open");
        assert!(
            menu.left() >= WIDTH,
            "beside the strip, not over it: {menu:?}"
        );
        assert!(
            menu.height() > 540.0,
            "every row fits on a tall window: {menu:?}"
        );
        assert!(menu.bottom() <= size.y, "{menu:?}");
    }

    #[test]
    fn the_grouped_menu_scrolls_on_a_short_window_and_stays_on_screen() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 420.0));
        let input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        click(&ctx, &mut time, size, CORNER, &input);
        let mut menu = None;
        for _ in 0..3 {
            menu = frame(&ctx, &mut time, size, Vec::new(), &input).1;
        }
        let menu = menu.expect("open");
        assert!(
            menu.top() >= 0.0 && menu.bottom() <= size.y + 0.5,
            "{menu:?}"
        );
    }

    #[test]
    fn picking_a_row_arms_that_tool_and_closes_the_menu() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        click(&ctx, &mut time, size, CORNER, &input);
        let mut menu = None;
        for _ in 0..3 {
            menu = frame(&ctx, &mut time, size, Vec::new(), &input).1;
        }
        let menu = menu.expect("open");
        // Every row of the menu, top to bottom, names a different tool.
        let mut armed = Vec::new();
        let mut y = menu.top() + 40.0;
        while y < menu.bottom() - 10.0 {
            let ctx = egui::Context::default();
            let mut time = 0.0;
            for _ in 0..3 {
                frame(&ctx, &mut time, size, Vec::new(), &input);
            }
            click(&ctx, &mut time, size, CORNER, &input);
            for _ in 0..3 {
                frame(&ctx, &mut time, size, Vec::new(), &input);
            }
            let actions = click(&ctx, &mut time, size, pos2(menu.left() + 60.0, y), &input);
            if let [ToolSidebarAction::Arm(tool)] = actions[..] {
                assert!(Tool::GROUPED.contains(&tool));
                if !armed.contains(&tool) {
                    armed.push(tool);
                }
                // It closed.
                let after = frame(&ctx, &mut time, size, Vec::new(), &input).1;
                assert!(after.is_none(), "the menu stays open after {tool:?}");
            }
            y += 10.0;
        }
        assert_eq!(armed.len(), Tool::GROUPED.len(), "{armed:?}");
    }
}
