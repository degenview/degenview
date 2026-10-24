//! Rows of values: the variables of a stopped script and the results of watches, with the parts of
//! arrays, maps, matrices and objects, and the earlier bars of a series, folding out under them.
//!
//! The rows are flattened to a list first, so only the ones on screen are laid out however many
//! values are open.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::pine::debug::{Group, ValueRef, Variable, VariableGroup};
use crate::script_manager::debug::{PanelAction, Shown};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;

use super::widgets::ROW;

/// One line of the flattened list.
pub enum Line<'a> {
    Heading {
        group: Group,
        count: usize,
    },
    Variable {
        key: String,
        depth: usize,
        variable: &'a Variable,
    },
    More {
        reference: ValueRef,
        depth: usize,
        loaded: usize,
        total: usize,
    },
    Loading {
        depth: usize,
    },
    Failed {
        depth: usize,
        message: &'a str,
    },
}

fn matches(name: &str, filter: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(filter)
}

/// The variables of the frame being looked at, by group, filtered by name.
pub fn variable_lines<'a>(shown: &'a Shown, filter: &str) -> Vec<Line<'a>> {
    let filter = filter.trim().to_lowercase();
    let mut lines = Vec::new();
    for VariableGroup { group, variables } in &shown.variables {
        let wanted: Vec<&Variable> = variables
            .iter()
            .filter(|variable| matches(&variable.name, &filter))
            .collect();
        if wanted.is_empty() {
            continue;
        }
        lines.push(Line::Heading {
            group: *group,
            count: wanted.len(),
        });
        for variable in wanted {
            push_variable(shown, &mut lines, variable.name.clone(), 0, variable);
        }
    }
    lines
}

/// Where the arrow keys take the cursor in the variables tree.
pub struct Navigation {
    pub cursor: Option<String>,
    /// The index among all `lines` to bring into view.
    pub reveal: Option<usize>,
}

/// Up and Down move between rows, Right opens a row (or steps into it), Left closes it (or steps to
/// its parent), Home and End jump, Return toggles. Only the rows that are values take the cursor.
pub fn navigate(
    ui: &Ui,
    lines: &[Line<'_>],
    shown: &Shown,
    cursor: Option<&str>,
    actions: &mut Vec<PanelAction>,
) -> Navigation {
    use eframe::egui::{Key, Modifiers};
    let rows: Vec<(usize, &str, &Variable)> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Line::Variable { key, variable, .. } => Some((index, key.as_str(), *variable)),
            _ => None,
        })
        .collect();
    let mut navigation = Navigation {
        cursor: cursor.map(str::to_owned),
        reveal: None,
    };
    if rows.is_empty() {
        return navigation;
    }
    let pressed = |key: Key| ui.input_mut(|input| input.consume_key(Modifiers::NONE, key));
    let at = cursor.and_then(|key| rows.iter().position(|(_, row, _)| *row == key));
    let mut target = at;
    if pressed(Key::ArrowDown) {
        target = Some(at.map_or(0, |at| (at + 1).min(rows.len() - 1)));
    }
    if pressed(Key::ArrowUp) {
        target = Some(at.map_or(0, |at| at.saturating_sub(1)));
    }
    if pressed(Key::Home) {
        target = Some(0);
    }
    if pressed(Key::End) {
        target = Some(rows.len() - 1);
    }
    if let Some(at) = at {
        let (_, key, variable) = rows[at];
        let open = shown.open.contains(key);
        let toggle = |actions: &mut Vec<PanelAction>| {
            if let Some(reference) = &variable.expand {
                actions.push(PanelAction::ToggleRow {
                    key: key.to_owned(),
                    reference: reference.clone(),
                });
            }
        };
        if pressed(Key::ArrowRight) {
            if variable.expand.is_some() && !open {
                toggle(actions);
            } else if open && at + 1 < rows.len() {
                target = Some(at + 1);
            }
        }
        if pressed(Key::ArrowLeft) {
            if open {
                toggle(actions);
            } else if let Some((parent, _)) = key.rsplit_once('/') {
                target = rows.iter().position(|(_, row, _)| *row == parent);
            }
        }
        if pressed(Key::Enter) || pressed(Key::Space) {
            toggle(actions);
        }
    }
    if let Some(target) = target
        && Some(target) != at
    {
        navigation.cursor = Some(rows[target].1.to_owned());
        navigation.reveal = Some(rows[target].0);
    } else if at.is_none() && target.is_some() {
        navigation.cursor = Some(rows[0].1.to_owned());
        navigation.reveal = Some(rows[0].0);
    }
    navigation
}

/// The parts of a watch's value, when the watch is open.
pub fn watch_children<'a>(shown: &'a Shown, index: usize, variable: &'a Variable) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    push_children(shown, &mut lines, &format!("watch:{index}"), 0, variable);
    lines
}

fn push_variable<'a>(
    shown: &'a Shown,
    lines: &mut Vec<Line<'a>>,
    key: String,
    depth: usize,
    variable: &'a Variable,
) {
    lines.push(Line::Variable {
        key: key.clone(),
        depth,
        variable,
    });
    push_children(shown, lines, &key, depth, variable);
}

fn push_children<'a>(
    shown: &'a Shown,
    lines: &mut Vec<Line<'a>>,
    key: &str,
    depth: usize,
    variable: &'a Variable,
) {
    let Some(reference) = &variable.expand else {
        return;
    };
    if !shown.open.contains(key) {
        return;
    }
    match shown.loaded.get(reference) {
        Some(loaded) => {
            for child in &loaded.items {
                push_variable(
                    shown,
                    lines,
                    format!("{key}/{}", child.name),
                    depth + 1,
                    child,
                );
            }
            if let Some(message) = &loaded.error {
                lines.push(Line::Failed {
                    depth: depth + 1,
                    message,
                });
            } else if loaded.items.len() < loaded.total {
                lines.push(Line::More {
                    reference: reference.clone(),
                    depth: depth + 1,
                    loaded: loaded.items.len(),
                    total: loaded.total,
                });
            }
        }
        None => lines.push(Line::Loading { depth: depth + 1 }),
    }
}

fn value_color(variable: &Variable) -> Color32 {
    if !variable.assigned || variable.is_na {
        return theme::text_tertiary();
    }
    match variable.type_name.as_str() {
        "int" | "float" => theme::syntax::number(),
        "string" => theme::syntax::string(),
        "bool" => theme::syntax::keyword(),
        _ => theme::text_primary(),
    }
}

/// `#RRGGBB` or `#RRGGBBAA`, as the colour it names.
fn swatch(value: &str) -> Option<Color32> {
    let hex = value.strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    match hex.len() {
        6 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(
            byte(0)?,
            byte(2)?,
            byte(4)?,
            byte(6)?,
        )),
        _ => None,
    }
}

/// Draws the lines `range` of `lines`, and collects what was clicked.
pub fn show_rows(
    ui: &mut Ui,
    lines: &[Line<'_>],
    shown: &Shown,
    actions: &mut Vec<PanelAction>,
    allow_watch: bool,
    cursor: Option<&str>,
) {
    for line in lines {
        match line {
            Line::Heading { group, count } => heading(ui, *group, *count),
            Line::Variable {
                key,
                depth,
                variable,
            } => variable_row(
                ui,
                Row {
                    key,
                    depth: *depth,
                    variable,
                    selected: cursor == Some(key.as_str()),
                },
                shown,
                actions,
                allow_watch,
            ),
            Line::More {
                reference,
                depth,
                loaded,
                total,
            } => more_row(ui, reference, *depth, *loaded, *total, actions),
            Line::Loading { depth } => note_row(ui, *depth, "Loading…", theme::text_tertiary()),
            Line::Failed { depth, message } => {
                note_row(ui, *depth, message, theme::syntax::string());
            }
        }
    }
}

fn heading(ui: &mut Ui, group: Group, count: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let galley = kit::galley(
        ui,
        &group.title().to_uppercase(),
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_tertiary(),
        rect.width() - 40.0,
    );
    let end = rect.left() + 12.0 + galley.size().x;
    ui.painter().galley(
        pos2(
            rect.left() + 12.0,
            rect.center().y - galley.size().y / 2.0 + 2.0,
        ),
        galley,
        theme::text_tertiary(),
    );
    kit::paint_count(
        ui,
        end + 6.0,
        rect.center().y + 2.0,
        count,
        theme::text_tertiary(),
        theme::ink(10),
    );
}

fn note_row(ui: &mut Ui, depth: usize, text: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    ui.painter().with_clip_rect(rect).text(
        pos2(rect.left() + 28.0 + depth as f32 * 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(kit::size::SUBHEADLINE),
        color,
    );
}

fn more_row(
    ui: &mut Ui,
    reference: &ValueRef,
    depth: usize,
    loaded: usize,
    total: usize,
    actions: &mut Vec<PanelAction>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::ink(8));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().with_clip_rect(rect).text(
        pos2(rect.left() + 28.0 + depth as f32 * 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("Show more ({} of {total} shown)", loaded),
        FontId::proportional(kit::size::SUBHEADLINE),
        kit::ACCENT,
    );
    if response.clicked() {
        actions.push(PanelAction::LoadMore {
            reference: reference.clone(),
        });
    }
}

/// Paints one value row into `rect`: the fold chevron, the name, the value, and the type.
pub fn paint_variable(
    ui: &Ui,
    rect: Rect,
    depth: usize,
    variable: &Variable,
    open: bool,
    hovered: bool,
    trailing: f32,
) {
    if hovered {
        ui.painter().rect_filled(rect, 0.0, theme::ink(8));
        if variable.expand.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    let indent = rect.left() + 10.0 + depth as f32 * 14.0;
    if variable.expand.is_some() {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(indent + 6.0, rect.center().y), vec2(9.0, 9.0)),
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            theme::text_tertiary(),
        );
    }
    let name_width = (rect.width() * 0.4).clamp(96.0, 240.0);
    let name_clip = Rect::from_min_max(
        pos2(indent + 16.0, rect.top()),
        pos2(
            (rect.left() + name_width).max(indent + 40.0) - 6.0,
            rect.bottom(),
        ),
    );
    let index_name = variable.name.starts_with('[');
    ui.painter().with_clip_rect(name_clip).text(
        pos2(name_clip.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &variable.name,
        if index_name || variable.declaration.is_none() && depth == 0 && variable.name.contains(' ')
        {
            FontId::monospace(11.5)
        } else {
            FontId::proportional(kit::size::BODY - 0.5)
        },
        if index_name {
            theme::text_secondary()
        } else {
            theme::text_primary()
        },
    );

    let type_text = variable.age.as_deref().unwrap_or(&variable.type_name);
    let show_type = rect.width() > 380.0 && !type_text.is_empty();
    let type_width = if show_type { 92.0 } else { 0.0 };
    let value_left = (rect.left() + name_width).max(name_clip.right() + 6.0);
    let value_clip = Rect::from_min_max(
        pos2(value_left, rect.top()),
        pos2(rect.right() - trailing - type_width - 10.0, rect.bottom()),
    );
    if value_clip.width() > 8.0 {
        // The "changed" pill reaches 4 points left of the value; the name's clip ends 6 before it.
        let painter = ui
            .painter()
            .with_clip_rect(value_clip.with_min_x(value_clip.left() - 4.0));
        let mut x = value_clip.left();
        if variable.type_name == "color"
            && let Some(color) = swatch(&variable.value)
        {
            let well = Rect::from_min_size(pos2(x, rect.center().y - 5.0), vec2(10.0, 10.0));
            painter.rect_filled(well, 2.0, color);
            painter.rect_stroke(
                well,
                2.0,
                Stroke::new(1.0, theme::ink(40)),
                egui::StrokeKind::Inside,
            );
            x += 15.0;
        }
        let color = if variable.changed {
            theme::editor_debug_line()
        } else {
            value_color(variable)
        };
        let font = if variable.assigned {
            FontId::monospace(11.5)
        } else {
            FontId::proportional(kit::size::SUBHEADLINE)
        };
        let galley = painter.layout_no_wrap(variable.value.clone(), font, color);
        if variable.changed {
            let pill = Rect::from_min_size(
                pos2(x - 4.0, rect.center().y - 8.0),
                vec2(galley.size().x + 8.0, 16.0),
            );
            painter.rect_filled(
                pill,
                4.0,
                theme::with_alpha(theme::editor_debug_line(), 0.16),
            );
        }
        painter.galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    if show_type {
        ui.painter().with_clip_rect(rect).text(
            pos2(rect.right() - trailing - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            type_text,
            FontId::proportional(kit::size::SUBHEADLINE),
            theme::text_tertiary(),
        );
    }
}

/// What `variable_row` draws.
struct Row<'a> {
    key: &'a str,
    depth: usize,
    variable: &'a Variable,
    selected: bool,
}

fn variable_row(
    ui: &mut Ui,
    row: Row<'_>,
    shown: &Shown,
    actions: &mut Vec<PanelAction>,
    allow_watch: bool,
) {
    let Row {
        key,
        depth,
        variable,
        selected,
    } = row;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let open = shown.open.contains(key);
    if selected {
        ui.painter()
            .rect_filled(rect, 0.0, theme::with_alpha(kit::ACCENT, 0.16));
    }
    paint_variable(ui, rect, depth, variable, open, response.hovered(), 0.0);
    let tip = if variable.assigned {
        format!(
            "{} = {}  ({})",
            variable.name, variable.full, variable.type_name
        )
    } else {
        format!("{}: {}", variable.name, variable.value)
    };
    let response = response.on_hover_text(tip);
    if response.clicked() {
        actions.push(PanelAction::Cursor(key.to_owned()));
        if let Some(reference) = &variable.expand {
            actions.push(PanelAction::ToggleRow {
                key: key.to_owned(),
                reference: reference.clone(),
            });
        }
    }
    // An open row whose parts have not arrived yet asks for them.
    if open
        && let Some(reference) = &variable.expand
        && !shown.loaded.contains_key(reference)
    {
        actions.push(PanelAction::LoadMore {
            reference: reference.clone(),
        });
    }
    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            kit::menu_fit(ui, &["Copy Name", "Copy Value", "Add to Watches"]);
            ui.spacing_mut().item_spacing.y = 0.0;
            if kit::menu_item(ui, Icon::CopyDoc, "Copy Name", true, false) {
                ui.ctx().copy_text(variable.name.clone());
                ui.close();
            }
            if kit::menu_item(ui, Icon::CopyDoc, "Copy Value", variable.assigned, false) {
                ui.ctx().copy_text(variable.full.clone());
                ui.close();
            }
            if allow_watch
                && depth == 0
                && !key.starts_with("watch:")
                && kit::menu_item(ui, Icon::Eye, "Add to Watches", true, false)
            {
                actions.push(PanelAction::AddWatch(variable.name.clone()));
                ui.close();
            }
        });
}
