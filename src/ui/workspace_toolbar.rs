//! The chart window's toolbar, laid out like the Swift one: the timeframe as a segmented
//! control on the left; the layout menu, Portfolio and Script Manager, the watchlist toggle and
//! Add Chart as separate bubbles on the right. Replay and Alerts live on the tool strip.

use crate::model::{ChartSyncSettings, Timeframe};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

pub const HEIGHT: f32 = 48.0;
const BUBBLE: f32 = 30.0;
const GAP: f32 = 10.0;
/// Long layout names are cut so the toolbar keeps its shape.
const NAME_MAX_WIDTH: f32 = 180.0;

pub struct ToolbarInput<'a> {
    pub timeframe: Timeframe,
    pub layout_name: &'a str,
    pub unsaved: bool,
    pub autosave: AutosaveState,
    /// `(id, name)` of every saved layout, and which one is open.
    pub layouts: &'a [(u64, String)],
    pub active_layout: Option<u64>,
    pub watchlist_visible: bool,
    /// What the charts share of horizontal navigation.
    pub sync: ChartSyncSettings,
    /// Whether the layout holds more than one chart to link.
    pub can_link: bool,
    /// Charts that cannot take part in linking, by title.
    pub unlinkable: &'a [String],
}

/// What the layout menu's Autosave row shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct AutosaveState {
    /// The tab is on a saved layout; an unnamed tab has nothing to autosave.
    pub available: bool,
    pub on: bool,
    /// The last autosave did not reach disk.
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    SetTimeframe(Timeframe),
    SaveLayout,
    ToggleAutosave,
    CopyLayout,
    RenameLayout,
    OpenLayout(u64),
    DeleteLayout(u64),
    ToggleWatchlist,
    OpenPortfolio,
    OpenScriptManager,
    AddChart,
    SetSync(ChartSyncSettings),
}

pub fn show(ui: &mut Ui, input: &ToolbarInput) -> Vec<ToolbarAction> {
    let mut actions = Vec::new();
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, 0.0, theme::chrome());
    let y = bar.center().y - BUBBLE / 2.0;

    // Left: the timeframe.
    let mut x = bar.left() + 12.0;
    if let Some(timeframe) = timeframes(ui, pos2(x, y), input.timeframe, &mut x) {
        actions.push(ToolbarAction::SetTimeframe(timeframe));
    }
    // Right, from the edge inwards: Add Chart, the watchlist, the app buttons, the layout.
    let mut right = bar.right() - 12.0;
    let add_label = kit::galley(
        ui,
        "Add Chart",
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        120.0,
    );
    let add = Rect::from_min_max(
        pos2(right - (12.0 + 12.0 + 6.0 + add_label.size().x + 12.0), y),
        pos2(right, y + BUBBLE),
    );
    if crate::ui::accessibility::label(
        bubble_button(ui, add, Id::new("toolbar-add"), false),
        "Add Chart",
    )
    .on_hover_text("Add Chart")
    .clicked()
    {
        actions.push(ToolbarAction::AddChart);
    }
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(add.left() + 12.0, add.center().y - 6.0),
            Vec2::splat(12.0),
        ),
        Icon::Plus,
        theme::text_primary(),
    );
    ui.painter().galley(
        pos2(add.left() + 30.0, add.center().y - add_label.size().y / 2.0),
        add_label,
        theme::text_primary(),
    );
    right = add.left() - GAP;

    let sidebar = Rect::from_min_max(pos2(right - 38.0, y), pos2(right, y + BUBBLE));
    let tip = if input.watchlist_visible {
        "Hide Watchlists"
    } else {
        "Show Watchlists"
    };
    if icon_bubble(
        ui,
        sidebar,
        Id::new("toolbar-watchlist"),
        Icon::SidebarRight,
        tip,
    )
    .clicked()
    {
        actions.push(ToolbarAction::ToggleWatchlist);
    }
    right = sidebar.left() - GAP;

    // Portfolio and Script Manager share one bubble, as the Swift title-bar group does.
    let group = Rect::from_min_max(pos2(right - 76.0, y), pos2(right, y + BUBBLE));
    paint_bubble(ui, group, false, false);
    let halves = [
        (
            Icon::Briefcase,
            "Portfolio Tracker",
            ToolbarAction::OpenPortfolio,
        ),
        (
            Icon::Curlybraces,
            "Script Manager",
            ToolbarAction::OpenScriptManager,
        ),
    ];
    for (index, (icon, tip, action)) in halves.into_iter().enumerate() {
        let half = Rect::from_min_size(
            pos2(group.left() + 2.0 + index as f32 * 36.0, group.top() + 2.0),
            vec2(36.0, BUBBLE - 4.0),
        );
        let response = ui
            .interact(half, Id::new(("toolbar-app", index)), Sense::click())
            .on_hover_text(tip);
        let response = crate::ui::accessibility::label(response, tip);
        if response.hovered() {
            ui.painter().rect_filled(
                half,
                (BUBBLE - 4.0) / 2.0,
                kit::primary(if response.is_pointer_button_down_on() {
                    0.14
                } else {
                    0.08
                }),
            );
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(half.center(), Vec2::splat(15.0)),
            icon,
            theme::text_primary(),
        );
        if response.clicked() {
            actions.push(action);
        }
    }
    right = group.left() - GAP;

    let layout_left = layout_menu(ui, right, y, input, &mut actions);
    link_menu(ui, layout_left - GAP, y, input, &mut actions);
    actions
}

/// The Link charts bubble, its right edge at `right`: a chain while charts are linked, a broken
/// one while not, and a popover with the two options. Not drawn while there is nothing to link.
fn link_menu(
    ui: &mut Ui,
    right: f32,
    y: f32,
    input: &ToolbarInput,
    actions: &mut Vec<ToolbarAction>,
) {
    if !input.can_link {
        return;
    }
    let rect = Rect::from_min_max(pos2(right - 38.0, y), pos2(right, y + BUBBLE));
    let linked = input.sync.is_linked();
    let response = ui.interact(rect, Id::new("toolbar-link"), Sense::click());
    paint_bubble(
        ui,
        rect,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    if linked {
        ui.painter().rect_filled(
            rect,
            rect.height() / 2.0,
            theme::with_alpha(kit::ACCENT, 0.16),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            (rect.height() + 2.0) / 2.0,
            Stroke::new(2.0, kit::ACCENT),
            StrokeKind::Inside,
        );
    }
    let tip = crate::ui::link_charts::summary(input.sync);
    let response = response.on_hover_text(tip);
    let response = crate::ui::accessibility::label(response, tip.to_owned());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
        if linked {
            Icon::ChainLinked
        } else {
            Icon::ChainBroken
        },
        if linked {
            kit::ACCENT
        } else {
            theme::text_primary()
        },
    );
    egui::Popup::from_toggle_button_response(&response)
        .frame(kit::menu_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::BOTTOM_END)
        .gap(6.0)
        .show(|ui| {
            if let Some(next) = crate::ui::link_charts::show(ui, input.sync, input.unlinkable) {
                actions.push(ToolbarAction::SetSync(next));
            }
        });
}

/// The bubble every toolbar control sits in: a faint capsule that lifts on hover.
fn paint_bubble(ui: &Ui, rect: Rect, hovered: bool, pressed: bool) {
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        rect.height() / 2.0,
        kit::primary(if pressed {
            0.16
        } else if hovered {
            0.12
        } else {
            0.07
        }),
    );
    painter.rect_stroke(
        rect,
        rect.height() / 2.0,
        Stroke::new(1.0, kit::primary(0.06)),
        StrokeKind::Inside,
    );
}

fn bubble_button(ui: &mut Ui, rect: Rect, id: Id, active: bool) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    paint_bubble(
        ui,
        rect,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    if active {
        ui.painter().rect_filled(
            rect,
            rect.height() / 2.0,
            theme::with_alpha(kit::ACCENT, 0.16),
        );
    }
    response
}

fn icon_bubble(ui: &mut Ui, rect: Rect, id: Id, icon: Icon, tip: &str) -> egui::Response {
    let response = bubble_button(ui, rect, id, false).on_hover_text(tip);
    let response = crate::ui::accessibility::label(response, tip);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(15.0)),
        icon,
        theme::text_primary(),
    );
    response
}

/// The timeframe picker: one capsule, the chosen segment raised. Advances `x` past it.
fn timeframes(ui: &mut Ui, at: egui::Pos2, current: Timeframe, x: &mut f32) -> Option<Timeframe> {
    let labels: Vec<_> = Timeframe::ALL
        .iter()
        .map(|timeframe| {
            kit::galley(
                ui,
                timeframe.label(),
                12.5,
                if *timeframe == current {
                    SEMIBOLD
                } else {
                    MEDIUM
                },
                theme::text_primary(),
                60.0,
            )
        })
        .collect();
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| (label.size().x + 22.0).max(40.0))
        .collect();
    let track = Rect::from_min_size(at, vec2(widths.iter().sum::<f32>() + 4.0, BUBBLE));
    paint_bubble(ui, track, false, false);
    let mut picked = None;
    let mut left = track.left() + 2.0;
    for ((timeframe, label), width) in Timeframe::ALL.into_iter().zip(labels).zip(widths) {
        let segment = Rect::from_min_size(pos2(left, track.top() + 2.0), vec2(width, BUBBLE - 4.0));
        let response = ui
            .interact(
                segment,
                Id::new(("toolbar-timeframe", timeframe.label())),
                Sense::click(),
            )
            .on_hover_text(timeframe.description());
        let response = crate::ui::accessibility::label(response, timeframe.description());
        let selected = timeframe == current;
        let painter = ui.painter();
        if selected {
            painter.add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: Color32::from_black_alpha(45),
                }
                .as_shape(segment, segment.height() / 2.0),
            );
            painter.rect_filled(
                segment,
                segment.height() / 2.0,
                theme::pick(kit::primary(0.2), Color32::WHITE),
            );
        } else if response.hovered() {
            painter.rect_filled(segment, segment.height() / 2.0, kit::primary(0.06));
        }
        let color = if selected {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        painter.galley(segment.center() - label.size() / 2.0, label, color);
        if response.clicked() && !selected {
            picked = Some(timeframe);
        }
        left += width;
    }
    *x = track.right();
    picked
}

/// The layout bubble, its right edge at `right`: the layout's name, "(unsaved)" while it has
/// changes, and a menu of what can be done with it.
fn layout_menu(
    ui: &mut Ui,
    right: f32,
    y: f32,
    input: &ToolbarInput,
    actions: &mut Vec<ToolbarAction>,
) -> f32 {
    let name = kit::galley(
        ui,
        input.layout_name,
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        NAME_MAX_WIDTH,
    );
    let unsaved = input.unsaved.then(|| {
        kit::galley(
            ui,
            "(unsaved)",
            size::BODY,
            REGULAR,
            theme::text_secondary(),
            90.0,
        )
    });
    let content =
        name.size().x + unsaved.as_ref().map_or(0.0, |label| 4.0 + label.size().x) + 4.0 + 9.0;
    let rect = Rect::from_min_max(pos2(right - content - 24.0, y), pos2(right, y + BUBBLE));
    let help = if input.autosave.failed {
        format!(
            "{} — Autosave could not save; choose Save layout",
            input.layout_name
        )
    } else if input.unsaved {
        format!("{} — unsaved changes", input.layout_name)
    } else {
        input.layout_name.to_owned()
    };
    let response = bubble_button(ui, rect, Id::new("toolbar-layout"), false).on_hover_text(&help);
    let response = crate::ui::accessibility::label(response, format!("Layout: {help}"));
    let mut x = rect.left() + 12.0;
    let name_width = name.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    x += name_width + 4.0;
    if let Some(label) = unsaved {
        let width = label.size().x;
        ui.painter().galley(
            pos2(x, rect.center().y - label.size().y / 2.0),
            label,
            theme::text_secondary(),
        );
        x += width + 4.0;
    }
    icons::paint(
        ui.painter(),
        Rect::from_min_size(pos2(x, rect.center().y - 4.5), Vec2::splat(9.0)),
        Icon::ChevronDown,
        theme::text_secondary(),
    );

    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .align(egui::RectAlign::BOTTOM_END)
        .gap(6.0)
        .show(|ui| {
            ui.set_width(230.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            if kit::menu_item_with_shortcut(
                ui,
                Icon::Download,
                "Save layout",
                Some(&shortcut::SAVE.label()),
                true,
                false,
            ) {
                actions.push(ToolbarAction::SaveLayout);
                ui.close();
            }
            if autosave_row(ui, input.autosave) {
                actions.push(ToolbarAction::ToggleAutosave);
                ui.close();
            }
            kit::menu_separator(ui);
            if kit::menu_item(
                ui,
                Icon::CopyDoc,
                "Make a copy",
                input.active_layout.is_some(),
                false,
            ) {
                actions.push(ToolbarAction::CopyLayout);
                ui.close();
            }
            if kit::menu_item(
                ui,
                Icon::Pencil,
                "Rename",
                input.active_layout.is_some(),
                false,
            ) {
                actions.push(ToolbarAction::RenameLayout);
                ui.close();
            }
            if !input.layouts.is_empty() {
                kit::menu_separator(ui);
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(kit::text(
                        "Layouts",
                        size::CAPTION + 0.5,
                        SEMIBOLD,
                        theme::text_tertiary(),
                    ));
                });
                ui.add_space(4.0);
                for (id, name) in input.layouts {
                    match layout_row(ui, *id, name, input.active_layout == Some(*id)) {
                        Some(LayoutRow::Open) => {
                            actions.push(ToolbarAction::OpenLayout(*id));
                            ui.close();
                        }
                        Some(LayoutRow::Delete) => {
                            actions.push(ToolbarAction::DeleteLayout(*id));
                            ui.close();
                        }
                        None => {}
                    }
                }
            }
        });
    rect.left()
}

/// The Autosave toggle: a check while it is on, dimmed for a tab with no saved layout, and a
/// warning tint when the last autosave failed.
fn autosave_row(ui: &mut Ui, state: AutosaveState) -> bool {
    let width = ui.available_width();
    let sense = if state.available {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let hovered = state.available && response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, kit::ACCENT);
    }
    let color = match (state.available, hovered) {
        (false, _) => theme::text_tertiary(),
        (true, true) => Color32::WHITE,
        (true, false) => theme::text_primary(),
    };
    if state.on {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            Icon::Check,
            color,
        );
    }
    let label = kit::galley(ui, "Autosave", size::BODY, REGULAR, color, width - 70.0);
    let label_width = label.size().x;
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    if state.failed {
        icons::paint(
            painter,
            Rect::from_center_size(
                pos2(rect.left() + 32.0 + label_width + 14.0, rect.center().y),
                Vec2::splat(11.0),
            ),
            Icon::TriangleFill,
            if hovered { Color32::WHITE } else { kit::ORANGE },
        );
    }
    let response = crate::ui::accessibility::label(
        response,
        if state.on {
            "Autosave, on"
        } else {
            "Autosave, off"
        },
    );
    let response = if state.available {
        response.on_hover_text(if state.failed {
            "The last autosave could not be written. Choose Save layout."
        } else {
            "Write changes to this layout a moment after you make them."
        })
    } else {
        response.on_hover_text("Save the layout first, then Autosave can keep it up to date.")
    };
    state.available && response.clicked()
}

enum LayoutRow {
    Open,
    Delete,
}

/// A saved layout in the menu: a check on the open one, the name, and a trash button on hover.
fn layout_row(ui: &mut Ui, id: u64, name: &str, current: bool) -> Option<LayoutRow> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let trash = Rect::from_min_size(
        pos2(rect.right() - 26.0, rect.top() + 2.0),
        Vec2::splat(20.0),
    );
    let row = ui.interact(rect, Id::new(("toolbar-layout-row", id)), Sense::click());
    let delete = ui
        .interact(
            trash,
            Id::new(("toolbar-layout-delete", id)),
            Sense::click(),
        )
        .on_hover_text("Delete this layout");
    let hovered = row.hovered() || delete.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, kit::ACCENT);
    }
    let color = if hovered {
        Color32::WHITE
    } else {
        theme::text_primary()
    };
    if current {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            Icon::Check,
            color,
        );
    }
    let label = kit::galley(ui, name, size::BODY, REGULAR, color, width - 70.0);
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    if hovered {
        if delete.hovered() {
            painter.rect_filled(trash, 5.0, theme::ink(40));
        }
        icons::paint(
            painter,
            Rect::from_center_size(trash.center(), Vec2::splat(11.0)),
            Icon::Trash,
            Color32::WHITE,
        );
    }
    if delete.clicked() {
        Some(LayoutRow::Delete)
    } else if row.clicked() {
        Some(LayoutRow::Open)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbar_draws_headlessly_with_and_without_a_focused_chart() {
        let layouts = vec![(1, "Morning".to_owned()), (2, "Swing trades".to_owned())];
        let ctx = egui::Context::default();
        for (unsaved, active) in [(false, None), (true, Some(2))] {
            let input = ToolbarInput {
                timeframe: Timeframe::Day,
                layout_name: "Morning",
                unsaved,
                autosave: AutosaveState {
                    available: active.is_some(),
                    on: unsaved,
                    failed: false,
                },
                layouts: &layouts,
                active_layout: active,
                watchlist_visible: true,
                sync: ChartSyncSettings {
                    horizontal_position: unsaved,
                    horizontal_zoom: false,
                },
                can_link: active.is_some(),
                unlinkable: &["ETHUSDT (Renko)".to_owned()],
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &input).is_empty(), "nothing was clicked");
            });
            output.textures_delta.clear();
        }
    }
}
