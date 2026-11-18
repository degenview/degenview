//! The vertical tool strip down the left edge of a chart window (Swift `ToolSidebar`): the
//! drawing tools at the top; Paper Trading and the alerts bell at the bottom.

use crate::model::{ChartSyncSettings, MagnetMode, Tool, ToolGroup};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, shortcut, theme};
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
    /// The Zoom In mode is on: a drag on a plot selects the rectangle to zoom to.
    pub zoom_armed: bool,
    /// The focused chart has a zoom to go back from, so Zoom out is offered under Zoom in.
    pub can_zoom_out: bool,
    /// The tool the grouped brushes, arrows and shapes tile shows and arms: the last one used.
    pub group_tool: Tool,
    /// The tool the Lines tile shows and arms: the last one used from its menu.
    pub line_tool: Tool,
    /// The tool the text and notes tile shows and arms: the last one used from its menu.
    pub note_tool: Tool,
    /// The tool the chart patterns, Elliott waves and cycles tile shows and arms: the last one
    /// used from its menu.
    pub pattern_tool: Tool,
    /// The tool the forecasting, volume and measurement tile shows and arms: the last one used
    /// from its menu.
    pub forecast_tool: Tool,
    /// The tool the Fibonacci and Gann tile shows and arms: the last one used from its menu.
    pub fib_tool: Tool,
    pub magnet: MagnetMode,
    pub keep_drawing: bool,
    pub hide_all: bool,
    /// Every drawing on the tab is locked (and there is at least one).
    pub all_locked: bool,
    /// There is at least one drawing to lock or hide.
    pub has_drawings: bool,
    pub paper_open: bool,
    /// Zoom In needs a market chart to zoom.
    pub has_market: bool,
    /// Alerts fired since the Alerts window was last looked at.
    pub unseen_alerts: usize,
    /// What the charts share of horizontal navigation.
    pub sync: ChartSyncSettings,
    /// The layout holds more than one chart to link; the Link charts tile only shows then.
    pub can_link: bool,
    /// Charts that cannot take part in linking, by title.
    pub unlinkable: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSidebarAction {
    /// Arm `tool`, or disarm it when it already is (back to the crosshair).
    Select(Tool),
    /// Arm `tool` (from the grouped menu), even when it already is.
    Arm(Tool),
    /// Arm the Zoom In mode, or put it down when it is armed.
    ToggleZoomIn,
    /// Go back to the view the last zoom came from.
    ZoomOut,
    SetMagnet(MagnetMode),
    ToggleKeepDrawing,
    ToggleLockAll,
    ToggleHideAll,
    TogglePaper,
    OpenAlerts,
    SetSync(ChartSyncSettings),
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
    /// The colour of a lit tile; `None` is the neutral grey of every other selected surface.
    tint: Option<Color32>,
    enabled: bool,
    badge: usize,
    /// Identity of the tile when its tooltip changes with its state; empty uses the tooltip.
    key: &'a str,
    /// A small corner mark showing the tile opens a menu.
    notch: bool,
    /// A one-letter caption in the corner (the magnet's strength).
    mark: Option<&'a str>,
}

/// A tile at the top of the strip: one tool, or a tile that opens a menu of them.
#[derive(Clone, Copy)]
enum Slot {
    Tool(Tool, Icon, &'static str),
    Menu(Menu),
}

/// The menus a tile opens: the lines, channels and pitchforks; the Fibonacci and Gann tools; the
/// chart patterns, Elliott waves and cycles; the forecasting, volume and measurement tools; the
/// brushes, arrows and shapes; and the text and notes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Menu {
    Lines,
    Fib,
    Patterns,
    Forecast,
    Shapes,
    Notes,
}

impl Menu {
    fn groups(self) -> &'static [ToolGroup] {
        match self {
            Self::Lines => &ToolGroup::LINE_MENU,
            Self::Fib => &ToolGroup::FIB_MENU,
            Self::Patterns => &ToolGroup::PATTERN_MENU,
            Self::Forecast => &ToolGroup::FORECAST_MENU,
            Self::Shapes => &ToolGroup::MENU,
            Self::Notes => &ToolGroup::NOTE_MENU,
        }
    }

    fn contains(self, tool: Tool) -> bool {
        self.groups().contains(&tool.spec().group)
    }

    /// The tool the tile shows and arms.
    fn tool(self, input: &ToolSidebarInput) -> Tool {
        match self {
            Self::Lines => input.line_tool,
            Self::Fib => input.fib_tool,
            Self::Patterns => input.pattern_tool,
            Self::Forecast => input.forecast_tool,
            Self::Shapes => input.group_tool,
            Self::Notes => input.note_tool,
        }
    }

    fn width(self) -> f32 {
        match self {
            Self::Lines => 268.0,
            Self::Fib => 268.0,
            // Wide enough for "Elliott triple combo wave (W·X·Y·X·Z)".
            Self::Patterns => 322.0,
            // Wide enough for "Fixed range volume profile".
            Self::Forecast => 268.0,
            Self::Shapes => 224.0,
            Self::Notes => 224.0,
        }
    }

    /// The height of the whole list: a heading for each group and a row for each tool.
    fn height(self) -> f32 {
        let groups = self.groups();
        let rows: usize = groups
            .iter()
            .map(|group| Tool::in_group(*group).count())
            .sum();
        rows as f32 * ROW_HEIGHT
            + groups.len() as f32 * 20.0
            + (groups.len() - 1) as f32 * 6.0
            + 4.0
    }

    fn key(self) -> &'static str {
        match self {
            Self::Lines => "lines",
            Self::Fib => "fib",
            Self::Patterns => "patterns",
            Self::Forecast => "forecast",
            Self::Shapes => "group",
            Self::Notes => "notes",
        }
    }

    fn corner_label(self) -> &'static str {
        match self {
            Self::Lines => "More lines, channels and pitchforks",
            Self::Fib => "More Fibonacci and Gann tools",
            Self::Patterns => "More chart patterns, Elliott waves and cycles",
            Self::Forecast => "More forecasting, volume and measurement tools",
            Self::Shapes => "More brushes, arrows and shapes",
            Self::Notes => "More text and notes",
        }
    }
}

const SLOTS: [Slot; 8] = [
    Slot::Tool(Tool::Cursor, Icon::Plus, "Crosshair"),
    Slot::Menu(Menu::Lines),
    Slot::Menu(Menu::Fib),
    Slot::Menu(Menu::Patterns),
    Slot::Menu(Menu::Forecast),
    Slot::Menu(Menu::Shapes),
    Slot::Menu(Menu::Notes),
    Slot::Tool(Tool::Ruler, Icon::Ruler, "Ruler"),
];

/// The height of one row of a menu.
const ROW_HEIGHT: f32 = 30.0;

/// The glyph a tool has on the strip and in the grouped menu.
pub fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Cursor => Icon::Plus,
        Tool::TrendLine => Icon::LineTrend,
        Tool::FibRetracement => Icon::FibRetracement,
        Tool::FibExtension => Icon::FibExtension,
        Tool::FibChannel => Icon::FibChannel,
        Tool::FibTimeZone => Icon::FibTimeZone,
        Tool::FibSpeedFan => Icon::FibSpeedFan,
        Tool::FibTrendTime => Icon::FibTrendTime,
        Tool::FibCircles => Icon::FibCircles,
        Tool::FibSpiral => Icon::FibSpiral,
        Tool::FibSpeedArcs => Icon::FibSpeedArcs,
        Tool::FibWedge => Icon::FibWedge,
        Tool::Pitchfan => Icon::Pitchfan,
        Tool::GannBox => Icon::GannBox,
        Tool::GannSquareFixed => Icon::GannSquareFixed,
        Tool::GannSquare => Icon::GannSquare,
        Tool::GannFan => Icon::GannFan,
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
        Tool::Ray => Icon::LineRay,
        Tool::InfoLine => Icon::LineInfo,
        Tool::ExtendedLine => Icon::LineExtended,
        Tool::TrendAngle => Icon::LineAngle,
        Tool::HorizontalLine => Icon::LineHorizontal,
        Tool::HorizontalRay => Icon::LineHorizontalRay,
        Tool::VerticalLine => Icon::LineVertical,
        Tool::Crossline => Icon::LineCross,
        Tool::ParallelChannel => Icon::ChannelParallel,
        Tool::RegressionTrend => Icon::ChannelRegression,
        Tool::FlatChannel => Icon::ChannelFlat,
        Tool::DisjointChannel => Icon::ChannelDisjoint,
        Tool::Pitchfork => Icon::ForkAndrews,
        Tool::SchiffPitchfork => Icon::ForkSchiff,
        Tool::ModifiedSchiffPitchfork => Icon::ForkModifiedSchiff,
        Tool::InsidePitchfork => Icon::ForkInside,
        Tool::Text => Icon::TextTool,
        Tool::Comment => Icon::CommentTool,
        Tool::PriceLabel => Icon::PriceLabelTool,
        Tool::FlagMark => Icon::FlagTool,
        Tool::Pin => Icon::PinTool,
        Tool::Note => Icon::NoteTool,
        Tool::PriceNote => Icon::PriceNoteTool,
        Tool::Callout => Icon::CalloutTool,
        Tool::Signpost => Icon::SignpostTool,
        Tool::Table => Icon::TableTool,
        Tool::Image => Icon::ImageTool,
        Tool::Post => Icon::XLogo,
        Tool::XabcdPattern => Icon::PatternXabcd,
        Tool::CypherPattern => Icon::PatternCypher,
        Tool::HeadAndShoulders => Icon::PatternHeadShoulders,
        Tool::AbcdPattern => Icon::PatternAbcd,
        Tool::TrianglePattern => Icon::PatternTriangle,
        Tool::ThreeDrivesPattern => Icon::PatternThreeDrives,
        Tool::ElliottImpulse => Icon::WaveImpulse,
        Tool::ElliottCorrection => Icon::WaveCorrection,
        Tool::ElliottTriangle => Icon::WaveTriangle,
        Tool::ElliottDoubleCombo => Icon::WaveDouble,
        Tool::ElliottTripleCombo => Icon::WaveTriple,
        Tool::CyclicLines => Icon::CycleLines,
        Tool::TimeCycles => Icon::CycleArcs,
        Tool::SineLine => Icon::CycleSine,
        Tool::LongPosition => Icon::PositionLong,
        Tool::ShortPosition => Icon::PositionShort,
        Tool::PositionForecast => Icon::PositionForecast,
        Tool::BarsPattern => Icon::BarsPatternTool,
        Tool::GhostFeed => Icon::GhostFeedTool,
        Tool::Sector => Icon::SectorTool,
        Tool::AnchoredVwap => Icon::VwapTool,
        Tool::FixedRangeVolumeProfile => Icon::ProfileFixed,
        Tool::AnchoredVolumeProfile => Icon::ProfileAnchored,
        Tool::PriceRange => Icon::RangePrice,
        Tool::DateRange => Icon::RangeDate,
        Tool::DateAndPriceRange => Icon::RangeDatePrice,
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
                    tint: None,
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
            Slot::Menu(menu) => menu_tile(ui, rect, input, strip.right(), &mut actions, menu),
        }
        y += BUTTON + SPACING;
    }

    // Zoom in sits under the ruler, as TradingView's does. It is a mode of the pointer rather
    // than a drawing tool, so it is lit from its own state. Zoom out appears under it only while
    // the focused chart has a zoom to go back from, and the options below make room.
    let zoom = |icon, label, active| Tile {
        icon,
        label,
        tooltip: label,
        active,
        tint: None,
        enabled: input.has_market,
        badge: 0,
        key: "",
        notch: false,
        mark: None,
    };
    // Like the options below, they give way to the bottom group on a short window.
    let limit = strip.bottom() - 135.0 - 10.0;
    let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    let lit = if input.zoom_armed {
        Active::Filled
    } else {
        Active::Off
    };
    if rect.bottom() <= limit
        && tile_button(ui, rect, &zoom(Icon::ZoomIn, "Zoom in", lit), strip.right()).clicked()
    {
        actions.push(ToolSidebarAction::ToggleZoomIn);
    }
    y += BUTTON + SPACING;
    if input.can_zoom_out && y + BUTTON <= limit {
        let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
        let tile = zoom(Icon::ZoomOut, "Zoom out", Active::Off);
        if tile_button(ui, rect, &tile, strip.right()).clicked() {
            actions.push(ToolSidebarAction::ZoomOut);
        }
        y += BUTTON + SPACING;
    }

    // The drawing options: magnet, keep drawing, lock and hide. They take the room the bottom
    // group leaves; on a short window the last ones are dropped (their shortcuts and the plot's
    // menu still reach them).
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
        tint: None,
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
    let mut all_options_fit = true;
    for (tile, action) in options {
        if y + BUTTON > limit {
            all_options_fit = false;
            break;
        }
        let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
        if tile_button(ui, rect, &tile, strip.right()).clicked() {
            actions.push(action);
        }
        y += BUTTON + SPACING;
    }

    // Link charts sits under the options behind its own rule, and only while there is more than
    // one chart to link. On a short window it is dropped with the options.
    if input.can_link && all_options_fit && y + 2.0 + SPACING / 2.0 + 2.0 + BUTTON <= limit {
        y += 2.0;
        ui.painter().rect_filled(
            Rect::from_center_size(pos2(strip.center().x, y - SPACING / 2.0), vec2(22.0, 1.0)),
            0.5,
            theme::separator(),
        );
        y += SPACING / 2.0 + 2.0;
        let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
        link_tile(ui, rect, input, strip.right(), &mut actions);
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
            tint: None,
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
            tint: None,
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

/// A tile that opens a menu of tools: it shows the last tool used from the menu, arms it on a
/// click, and opens the menu from its corner, a right-click or the keyboard.
/// The Link charts tile: a chain while charts are linked (a grey wash, like every other selected
/// surface), a broken one while not. It opens the popover with the two options to the right.
fn link_tile(
    ui: &mut Ui,
    rect: Rect,
    input: &ToolSidebarInput,
    strip_right: f32,
    actions: &mut Vec<ToolSidebarAction>,
) {
    let linked = input.sync.is_linked();
    let tip = crate::ui::workspace::link_charts::summary(input.sync);
    let response = tile_button(
        ui,
        rect,
        &Tile {
            icon: if linked {
                Icon::ChainLinked
            } else {
                Icon::ChainBroken
            },
            label: tip,
            tooltip: tip,
            active: if linked { Active::Tinted } else { Active::Off },
            tint: None,
            enabled: true,
            badge: 0,
            key: "link",
            notch: false,
            mark: None,
        },
        strip_right,
    );
    egui::Popup::from_toggle_button_response(&response)
        .frame(kit::menu_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(RectAlign::RIGHT_START)
        .align_alternatives(&[RectAlign::RIGHT_END])
        .gap(10.0)
        .show(|ui| {
            if let Some(next) =
                crate::ui::workspace::link_charts::show(ui, input.sync, &input.unlinkable)
            {
                actions.push(ToolSidebarAction::SetSync(next));
            }
        });
}

fn menu_tile(
    ui: &mut Ui,
    rect: Rect,
    input: &ToolSidebarInput,
    strip_right: f32,
    actions: &mut Vec<ToolSidebarAction>,
    menu: Menu,
) {
    let tool = menu.tool(input);
    let label = tool.label();
    let tile = Tile {
        icon: tool_icon(tool),
        label,
        tooltip: label,
        active: if menu.contains(input.active_tool) {
            Active::Filled
        } else {
            Active::Off
        },
        tint: None,
        enabled: true,
        badge: 0,
        key: menu.key(),
        notch: true,
        mark: None,
    };
    let response = tile_button(ui, rect, &tile, strip_right);
    if response.clicked() {
        actions.push(ToolSidebarAction::Select(tool));
    }
    // The corner opens the menu instead of arming the tool.
    let corner = Rect::from_min_size(rect.right_bottom() - vec2(16.0, 16.0), vec2(16.0, 16.0));
    let corner_response = ui.interact(
        corner,
        Id::new(("tool-sidebar-menu", menu.key())),
        Sense::click(),
    );
    let corner_response = crate::ui::accessibility::label(corner_response, menu.corner_label());
    let wants_open = corner_response.clicked()
        || response.secondary_clicked()
        || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::ArrowRight)));
    let popup = egui::Popup::menu(&response)
        .open_memory(wants_open.then_some(SetOpenCommand::Toggle))
        .align(RectAlign::RIGHT_START)
        .align_alternatives(&[RectAlign::RIGHT_END])
        .gap(10.0)
        .frame(kit::menu_frame())
        .width(menu.width());
    popup.show(|ui| tool_menu(ui, input, actions, menu));
}

/// A menu of tools: a heading for each group and one row per tool. Rows are focusable, so the
/// arrow keys move through them and Return picks one.
fn tool_menu(
    ui: &mut Ui,
    input: &ToolSidebarInput,
    actions: &mut Vec<ToolSidebarAction>,
    menu: Menu,
) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    // Room from where the menu starts down to the window's foot.
    let room = (ui.ctx().content_rect().bottom() - ui.next_widget_position().y - 16.0).max(160.0);
    let max_height = room.min(menu.height());
    // A popup's area starts out short; the menu says how tall it wants to be.
    ui.set_max_height(max_height);
    ui.set_min_height(max_height);
    let mut first = true;
    let seen = Id::new(("tool-sidebar-menu-focused", menu.key()));
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for group in menu.groups() {
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
                for tool in Tool::in_group(*group) {
                    let selected = input.active_tool == tool;
                    let response = menu_row(ui, tool, selected);
                    // Land the keyboard on the armed tool (or the first row) when it opens.
                    let landed = ui.data(|data| data.get_temp::<bool>(seen).unwrap_or(false));
                    if !landed
                        && (selected
                            || (input.active_tool == Tool::Cursor && tool == menu.tool(input)))
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
        ui.data_mut(|data| data.remove::<bool>(seen));
    }
}

/// One row of a menu: glyph, name, the shortcut that arms the tool (if it has one) and a check on
/// the armed tool.
fn menu_row(ui: &mut Ui, tool: Tool, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, tool.label());
    let highlighted = response.hovered() || response.has_focus();
    let painter = ui.painter();
    if highlighted {
        painter.rect_filled(rect, 6.0, theme::selected_tint(true));
    }
    let color = theme::text_primary();
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(17.0)),
        tool_icon(tool),
        color,
    );
    // The shortcut sits between the name and the check.
    let hint = shortcut::tool(tool).map(|shortcut| shortcut.label());
    let hint_width = if hint.is_some() { 56.0 } else { 0.0 };
    let galley = kit::galley(
        ui,
        tool.label(),
        kit::size::BODY,
        kit::REGULAR,
        color,
        width - 72.0 - hint_width,
    );
    ui.painter().galley(
        pos2(rect.left() + 38.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if let Some(hint) = hint {
        let tint = if highlighted {
            theme::text_secondary()
        } else {
            theme::text_tertiary()
        };
        let galley = kit::galley(
            ui,
            &hint,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            tint,
            hint_width,
        );
        ui.painter().galley(
            pos2(
                rect.right() - 32.0 - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            tint,
        );
    }
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
    let lit = tile.active == Active::Filled && tile.tint.is_some();
    let grey_lit = tile.active != Active::Off && tile.tint.is_none();
    let (fill, glyph) = match (tile.active, tile.tint) {
        (Active::Off, _) => (
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
        (_, None) => (
            theme::selected_tint(hovered || pressed),
            theme::text_primary(),
        ),
        (Active::Filled, Some(tint)) => (
            if pressed {
                tint.gamma_multiply(0.85)
            } else {
                tint
            },
            Color32::WHITE,
        ),
        (Active::Tinted, Some(tint)) => (
            theme::with_alpha(
                tint,
                if pressed {
                    0.26
                } else if hovered {
                    0.22
                } else {
                    0.16
                },
            ),
            tint,
        ),
    };
    if let (true, Some(tint)) = (lit, tile.tint) {
        painter.add(
            egui::Shadow {
                offset: [0, 2],
                blur: 10,
                spread: 0,
                color: theme::with_alpha(tint, 0.35),
            }
            .as_shape(shape, 10.0),
        );
    }
    painter.rect_filled(shape, 10.0, fill);
    if grey_lit {
        painter.rect_stroke(shape, 10.0, theme::selected_stroke(), StrokeKind::Inside);
    }
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
            Color32::from_rgb(33, 40, 44),
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
    fn the_strip_draws_headlessly_in_every_state() {
        let ctx = egui::Context::default();
        for (tool, paper, replay, market, unseen) in [
            (Tool::Cursor, false, false, true, 0),
            (Tool::FibRetracement, true, true, true, 3),
            (Tool::Ruler, false, false, false, 120),
        ] {
            let state = ToolSidebarInput {
                zoom_armed: false,
                can_zoom_out: false,
                active_tool: tool,
                group_tool: Tool::Brush,
                line_tool: Tool::TrendLine,
                note_tool: Tool::Text,
                pattern_tool: Tool::XabcdPattern,
                forecast_tool: Tool::LongPosition,
                fib_tool: Tool::FibRetracement,
                magnet: MagnetMode::Weak,
                keep_drawing: true,
                hide_all: false,
                all_locked: false,
                has_drawings: true,
                paper_open: paper,
                has_market: market,
                unseen_alerts: unseen,
                sync: ChartSyncSettings {
                    horizontal_position: replay,
                    horizontal_zoom: false,
                },
                can_link: market,
                unlinkable: vec!["ETHUSDT (Renko)".to_owned()],
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &state).is_empty(), "nothing was clicked");
            });
            output.textures_delta.clear();
        }
    }

    fn state() -> ToolSidebarInput {
        ToolSidebarInput {
            zoom_armed: false,
            can_zoom_out: false,
            active_tool: Tool::Cursor,
            group_tool: Tool::Brush,
            line_tool: Tool::TrendLine,
            note_tool: Tool::Text,
            pattern_tool: Tool::XabcdPattern,
            forecast_tool: Tool::LongPosition,
            fib_tool: Tool::FibRetracement,
            magnet: MagnetMode::Off,
            keep_drawing: true,
            hide_all: false,
            all_locked: false,
            has_drawings: true,
            paper_open: false,
            has_market: true,
            unseen_alerts: 0,
            sync: ChartSyncSettings::default(),
            can_link: true,
            unlinkable: Vec::new(),
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

    /// Clicks down the strip's left half in fresh contexts and returns the `y` values at which a
    /// popup opened (no tile opens one from a plain click except Link charts).
    fn popups_opened_by_plain_clicks(input: &ToolSidebarInput, height: f32) -> Vec<f32> {
        let size = vec2(900.0, height);
        (0..(height as i32 / 4))
            .map(|row| row as f32 * 4.0)
            .filter(|y| {
                let ctx = egui::Context::default();
                let mut time = 0.0;
                for _ in 0..3 {
                    frame(&ctx, &mut time, size, Vec::new(), input);
                }
                click(&ctx, &mut time, size, pos2(10.0, *y), input);
                frame(&ctx, &mut time, size, Vec::new(), input).1.is_some()
            })
            .collect()
    }

    #[test]
    fn link_charts_sits_under_the_options_and_only_shows_with_more_than_one_chart() {
        let mut input = state();
        input.can_link = false;
        assert!(
            popups_opened_by_plain_clicks(&input, 900.0).is_empty(),
            "no Link charts tile with a single chart"
        );

        input.can_link = true;
        let opened = popups_opened_by_plain_clicks(&input, 900.0);
        assert!(!opened.is_empty(), "the tile opens its popover");
        // Under the hide-all eye: the options are the four tiles above it.
        let tile_top = opened.first().copied().unwrap();
        assert!(tile_top > 300.0, "below the tool tiles: {tile_top}");

        // A short window drops it with the options instead of drawing over the bottom group.
        assert!(popups_opened_by_plain_clicks(&input, 330.0).is_empty());
    }

    /// The grouped tile's corner, where the menu opens from.
    const CORNER: Pos2 = pos2(31.0, 236.0);

    /// The patterns tile's corner, where its menu opens from.
    const PATTERNS_CORNER: Pos2 = pos2(31.0, 156.0);

    /// The forecasting, volume and measurement tile's corner.
    const FORECAST_CORNER: Pos2 = pos2(31.0, 196.0);

    #[test]
    fn the_grouped_menu_opens_from_the_corner_lists_every_tool_and_arms_the_pick() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        // A click on the tile itself arms its tool and opens nothing.
        let actions = click(&ctx, &mut time, size, pos2(14.0, 220.0), &input);
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

    /// The Lines tile's corner, where its menu opens from.
    const LINES_CORNER: Pos2 = pos2(31.0, 76.0);
    /// The corner notch of the Fibonacci and Gann tile, below the Lines tile.
    const FIB_CORNER: Pos2 = pos2(31.0, 116.0);

    /// Opens the menu from `corner` on a tall window and returns the tools its rows arm, top to
    /// bottom, each by clicking it in a fresh strip.
    fn tools_armed_by_the_menu(corner: Pos2) -> (Rect, Vec<Tool>) {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 1100.0));
        let input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        click(&ctx, &mut time, size, corner, &input);
        let mut menu = None;
        for _ in 0..3 {
            menu = frame(&ctx, &mut time, size, Vec::new(), &input).1;
        }
        let menu = menu.expect("open");
        let mut armed = Vec::new();
        let mut y = menu.top() + 40.0;
        while y < menu.bottom() - 10.0 {
            let ctx = egui::Context::default();
            let mut time = 0.0;
            for _ in 0..3 {
                frame(&ctx, &mut time, size, Vec::new(), &input);
            }
            click(&ctx, &mut time, size, corner, &input);
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
        (menu, armed)
    }

    #[test]
    fn picking_a_row_arms_that_tool_and_closes_the_menu() {
        let (_, armed) = tools_armed_by_the_menu(CORNER);
        let expected: Vec<Tool> = ToolGroup::MENU
            .iter()
            .flat_map(|group| Tool::in_group(*group))
            .collect();
        assert_eq!(armed, expected, "every row, in order");
        assert_eq!(armed.len(), 16);
    }

    #[test]
    fn the_lines_menu_lists_lines_channels_and_pitchforks_in_order() {
        let (menu, armed) = tools_armed_by_the_menu(LINES_CORNER);
        let expected: Vec<Tool> = ToolGroup::LINE_MENU
            .iter()
            .flat_map(|group| Tool::in_group(*group))
            .collect();
        assert_eq!(armed, expected, "every row, in order");
        assert_eq!(armed.len(), 17);
        assert_eq!(armed[0], Tool::TrendLine);
        assert_eq!(armed[9], Tool::ParallelChannel);
        assert_eq!(armed[13], Tool::Pitchfork);
        assert!(menu.left() >= WIDTH && menu.width() >= Menu::Lines.width() - 1.0);
    }

    #[test]
    fn the_fibonacci_menu_lists_fibonacci_then_gann_in_order() {
        let (menu, armed) = tools_armed_by_the_menu(FIB_CORNER);
        let expected: Vec<Tool> = ToolGroup::FIB_MENU
            .iter()
            .flat_map(|group| Tool::in_group(*group))
            .collect();
        assert_eq!(armed, expected, "every row, in order");
        assert_eq!(armed.len(), Tool::fib_tools().count());
        assert_eq!(armed[0], Tool::FibRetracement);
        assert!(menu.left() >= WIDTH && menu.width() >= Menu::Fib.width() - 1.0);
    }

    #[test]
    fn the_patterns_menu_lists_patterns_waves_and_cycles_in_order() {
        let (menu, armed) = tools_armed_by_the_menu(PATTERNS_CORNER);
        let expected: Vec<Tool> = ToolGroup::PATTERN_MENU
            .iter()
            .flat_map(|group| Tool::in_group(*group))
            .collect();
        assert_eq!(armed, expected, "every row, in order");
        assert_eq!(armed.len(), 14);
        assert_eq!(armed[0], Tool::XabcdPattern);
        assert_eq!(armed[6], Tool::ElliottImpulse);
        assert_eq!(armed[11], Tool::CyclicLines);
        assert!(menu.left() >= WIDTH && menu.width() >= Menu::Patterns.width() - 1.0);
    }

    #[test]
    fn the_forecast_menu_lists_forecasting_volume_and_measurement_in_order() {
        let (menu, armed) = tools_armed_by_the_menu(FORECAST_CORNER);
        let expected: Vec<Tool> = ToolGroup::FORECAST_MENU
            .iter()
            .flat_map(|group| Tool::in_group(*group))
            .collect();
        assert_eq!(armed, expected, "every row, in order");
        assert_eq!(armed.len(), 12);
        assert_eq!(armed[0], Tool::LongPosition);
        assert_eq!(armed[6], Tool::AnchoredVwap);
        assert_eq!(armed[9], Tool::PriceRange);
        assert!(menu.left() >= WIDTH && menu.width() >= Menu::Forecast.width() - 1.0);
    }

    #[test]
    fn the_forecast_tile_shows_its_last_tool_and_a_click_arms_it() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let mut input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        let actions = click(&ctx, &mut time, size, pos2(14.0, 180.0), &input);
        assert_eq!(actions, vec![ToolSidebarAction::Select(Tool::LongPosition)]);
        input.forecast_tool = Tool::DateAndPriceRange;
        let actions = click(&ctx, &mut time, size, pos2(14.0, 180.0), &input);
        assert_eq!(
            actions,
            vec![ToolSidebarAction::Select(Tool::DateAndPriceRange)]
        );
        assert!(Menu::Forecast.contains(Tool::AnchoredVolumeProfile));
        assert!(!Menu::Forecast.contains(Tool::SineLine) && !Menu::Patterns.contains(Tool::Sector));
    }

    #[test]
    fn the_patterns_tile_shows_its_last_tool_and_a_click_arms_it() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let mut input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        let actions = click(&ctx, &mut time, size, pos2(14.0, 140.0), &input);
        assert_eq!(actions, vec![ToolSidebarAction::Select(Tool::XabcdPattern)]);
        input.pattern_tool = Tool::SineLine;
        let actions = click(&ctx, &mut time, size, pos2(14.0, 140.0), &input);
        assert_eq!(actions, vec![ToolSidebarAction::Select(Tool::SineLine)]);
        assert!(Menu::Patterns.contains(Tool::ElliottTripleCombo));
        assert!(!Menu::Patterns.contains(Tool::Arc) && !Menu::Shapes.contains(Tool::SineLine));
    }

    #[test]
    fn zoom_in_sits_under_the_ruler_and_a_click_toggles_the_mode() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let mut input = state();
        input.has_market = true;
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        // The ruler is the ninth row's neighbour above: rows are 40 points apart from y = 10.
        let ruler = 10.0 + 40.0 * 7.0 + BUTTON / 2.0;
        let zoom = ruler + 40.0;
        let actions = click(&ctx, &mut time, size, pos2(22.0, ruler), &input);
        assert_eq!(actions, vec![ToolSidebarAction::Select(Tool::Ruler)]);
        let actions = click(&ctx, &mut time, size, pos2(22.0, zoom), &input);
        assert_eq!(actions, vec![ToolSidebarAction::ToggleZoomIn]);
        // The mode is its own state: lit from it, whatever drawing tool is armed.
        input.zoom_armed = true;
        let actions = click(&ctx, &mut time, size, pos2(22.0, zoom), &input);
        assert_eq!(actions, vec![ToolSidebarAction::ToggleZoomIn]);
    }

    #[test]
    fn zoom_out_appears_under_zoom_in_only_with_a_zoom_to_go_back_from() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let mut input = state();
        input.has_market = true;
        let zoom_out = 10.0 + 40.0 * 9.0 + BUTTON / 2.0;
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        // Nothing there: the row below Zoom in is the magnet, whose click is not Zoom out.
        let actions = click(&ctx, &mut time, size, pos2(22.0, zoom_out), &input);
        assert!(!actions.contains(&ToolSidebarAction::ZoomOut));
        input.can_zoom_out = true;
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        let actions = click(&ctx, &mut time, size, pos2(22.0, zoom_out), &input);
        assert_eq!(actions, vec![ToolSidebarAction::ZoomOut]);
    }

    #[test]
    fn on_a_short_window_zoom_out_gives_way_to_the_bottom_group() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 520.0));
        let mut input = state();
        input.has_market = true;
        input.can_zoom_out = true;
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        let zoom_out = 10.0 + 40.0 * 9.0 + BUTTON / 2.0;
        let actions = click(&ctx, &mut time, size, pos2(22.0, zoom_out), &input);
        assert!(!actions.contains(&ToolSidebarAction::ZoomOut));
        // Zoom in itself still fits there.
        let zoom = zoom_out - 40.0;
        let actions = click(&ctx, &mut time, size, pos2(22.0, zoom), &input);
        assert_eq!(actions, vec![ToolSidebarAction::ToggleZoomIn]);
    }

    #[test]
    fn zoom_in_is_inert_without_a_market_chart() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let mut input = state();
        input.has_market = false;
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        let zoom = 10.0 + 40.0 * 8.0 + BUTTON / 2.0;
        assert!(click(&ctx, &mut time, size, pos2(22.0, zoom), &input).is_empty());
    }

    #[test]
    fn the_lines_tile_shows_its_last_tool_and_a_click_arms_it() {
        let ctx = egui::Context::default();
        let (mut time, size) = (0.0, vec2(900.0, 900.0));
        let mut input = state();
        for _ in 0..3 {
            frame(&ctx, &mut time, size, Vec::new(), &input);
        }
        let actions = click(&ctx, &mut time, size, pos2(14.0, 60.0), &input);
        assert_eq!(actions, vec![ToolSidebarAction::Select(Tool::TrendLine)]);
        input.line_tool = Tool::SchiffPitchfork;
        let actions = click(&ctx, &mut time, size, pos2(14.0, 60.0), &input);
        assert_eq!(
            actions,
            vec![ToolSidebarAction::Select(Tool::SchiffPitchfork)]
        );
        // The Fibonacci tile is the next one down, and arming its tool lights neither of the others.
        let actions = click(&ctx, &mut time, size, pos2(14.0, 100.0), &input);
        assert_eq!(
            actions,
            vec![ToolSidebarAction::Select(Tool::FibRetracement)]
        );
        assert!(Menu::Fib.contains(Tool::FibRetracement));
        assert!(
            !Menu::Lines.contains(Tool::FibRetracement)
                && !Menu::Shapes.contains(Tool::FibRetracement)
        );
        assert!(Menu::Lines.contains(Tool::Pitchfork) && !Menu::Shapes.contains(Tool::Pitchfork));
        assert!(Menu::Shapes.contains(Tool::Arc) && !Menu::Lines.contains(Tool::Arc));
    }

    #[test]
    fn a_row_prints_the_shortcut_of_the_tool_it_arms() {
        for tool in Tool::GROUPED {
            let hint = shortcut::tool(tool).map(|shortcut| shortcut.label());
            assert_eq!(hint.is_some(), tool.hotkey().is_some(), "{tool:?}");
        }
        // The tallest menu fits its rows exactly.
        assert!(Menu::Lines.height() > Menu::Shapes.height());
    }

    /// The strip around Zoom in: resting, hovered, armed, and with Zoom out offered, in both
    /// themes. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_zoom_in() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::{self, AppTheme};
        if gallery_dir().is_none() {
            return;
        }
        let hover = pos2(22.0, 10.0 + 40.0 * 8.0 + BUTTON / 2.0);
        for (theme_name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for (name, armed, zoom_out, pointer) in [
                ("rest", false, false, None),
                ("hover", false, false, Some(hover)),
                ("armed", true, false, None),
                ("zoom-out", false, true, Some(hover + vec2(0.0, 40.0))),
            ] {
                let mut screen = Screen::new(vec2(300.0, 640.0));
                theme::apply(&screen.ctx, appearance);
                let mut input = state();
                input.has_market = true;
                input.zoom_armed = armed;
                input.can_zoom_out = zoom_out;
                let mut last = None;
                for _ in 0..5 {
                    theme::sync(&screen.ctx);
                    let events = pointer
                        .map(|at| vec![egui::Event::PointerMoved(at)])
                        .unwrap_or_default();
                    last = Some(screen.run(events, |ui| {
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 640.0))),
                            |ui| {
                                show(ui, &input);
                            },
                        );
                    }));
                }
                screen.save(
                    &format!("zoom-in-{name}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }

    /// Both menus as a picture. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_menus() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        for (name, corner) in [
            ("menu-lines", LINES_CORNER),
            ("menu-fib", FIB_CORNER),
            ("menu-patterns", PATTERNS_CORNER),
            ("menu-forecast", FORECAST_CORNER),
            ("menu-shapes", CORNER),
        ] {
            let mut screen = Screen::new(vec2(420.0, 700.0));
            let input = state();
            let run = |screen: &mut Screen, events: Vec<egui::Event>| {
                screen.run(events, |ui| {
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 700.0))),
                        |ui| {
                            show(ui, &input);
                        },
                    );
                })
            };
            let button = |pressed| egui::Event::PointerButton {
                pos: corner,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            for _ in 0..3 {
                run(&mut screen, Vec::new());
            }
            run(&mut screen, vec![egui::Event::PointerMoved(corner)]);
            run(&mut screen, vec![button(true)]);
            run(&mut screen, vec![button(false)]);
            let mut last = None;
            for _ in 0..4 {
                last = Some(run(&mut screen, Vec::new()));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}
