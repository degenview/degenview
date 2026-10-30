//! Settings ▸ Script Manager: how the Pine editor looks. The theme (built-in or your own), the
//! typography, and, from "Customize", an editor for every colour the editor paints with the real
//! editor beside it. Everything takes effect as it is changed, in every window; see
//! [`EditorThemeHost`] for how it is kept and saved.
//!
//! The page works on the host directly (as the iCloud and updates pages do on theirs): there is
//! nothing here to apply later and nothing to store, so it returns no actions.

use crate::host::editor_theme_host::{CustomTheme, EditorThemeHost, FontStatus, View};
use crate::model::editor_theme::{
    BuiltInTheme, LineHeight, MAX_FONT_SIZE, MAX_NAME_CHARS, MIN_FONT_SIZE, Palette, SurfaceColor,
    SyntaxRole, ThemeId, ThemeSelection, ThemeSource, clamp_size,
};
use crate::script_manager::theme_preview::ThemePreview;
use crate::service::theme_files;
use crate::ui::icons::{self, Icon};
use crate::ui::settings_font_picker::{self, FontPickerState};
use crate::ui::settings_theme_editor::{self, Editing};
use crate::ui::settings_window::{SettingsSection, page_shell, section, status_message};
use crate::ui::{kit, settings_card, sheet, theme};
use eframe::egui::{
    self, Color32, CornerRadius, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The largest theme file the page reads (`codec::MAX_FILE_BYTES`).
const MAX_IMPORT_BYTES: usize = crate::model::editor_theme::codec::MAX_FILE_BYTES;
const TILE_GAP: f32 = 12.0;
const TILE_MIN_WIDTH: f32 = 124.0;
const PREVIEW_HEIGHT: f32 = 66.0;
const TILE_HEIGHT: f32 = 10.0 + PREVIEW_HEIGHT + 9.0 + 34.0 + 8.0;
const DIALOG_WIDTH: f32 = 420.0;
const MINIATURE_RADIUS: f32 = 8.0;

/// What the page is showing.
#[derive(Default)]
pub(super) enum Mode {
    #[default]
    Overview,
    Editing(Box<Editing>),
}

/// A sheet over the page.
pub(super) enum Dialog {
    Create {
        name: String,
        source: usize,
        focus: bool,
        error: Option<String>,
    },
    Rename {
        id: ThemeId,
        name: String,
        focus: bool,
        error: Option<String>,
    },
    Delete(ThemeId),
    ResetAll(ThemeId),
}

/// A file dialog that is open on another thread.
enum Flow {
    Import(Receiver<Result<Option<String>, String>>),
    Export(Receiver<Result<Option<PathBuf>, String>>, String),
}

/// The size field and the font picker, which the overview and the theme editor share.
#[derive(Default)]
pub(super) struct Controls {
    pub(super) picker: FontPickerState,
    /// What is being typed in the size field, while it has the focus.
    size_text: Option<String>,
}

#[derive(Default)]
pub struct ScriptManagerState {
    pub(super) mode: Mode,
    pub(super) preview: ThemePreview,
    pub(super) dialog: Option<Dialog>,
    pub(super) controls: Controls,
    flow: Option<Flow>,
    /// What the last import, export or refused change said, and whether it is a failure.
    status: Option<(String, bool)>,
}

impl ScriptManagerState {
    /// Back to the overview, when the page is left.
    pub fn leave(&mut self) {
        self.mode = Mode::Overview;
        self.dialog = None;
    }
}

/// One theme as a tile.
struct Tile<'a> {
    selection: ThemeSelection,
    name: String,
    caption: String,
    /// The palettes drawn in it: one, or Dark then Light for the automatic tile.
    palettes: Vec<Palette>,
    custom: Option<&'a CustomTheme>,
}

fn tiles(view: &View) -> Vec<Tile<'_>> {
    let mut tiles = vec![Tile {
        selection: ThemeSelection::Auto,
        name: crate::model::editor_theme::AUTOMATIC_NAME.to_owned(),
        caption: "Follows the app".to_owned(),
        palettes: vec![BuiltInTheme::Dark.palette(), BuiltInTheme::Light.palette()],
        custom: None,
    }];
    tiles.extend(BuiltInTheme::ALL.iter().map(|built_in| Tile {
        selection: ThemeSelection::BuiltIn(*built_in),
        name: built_in.name().to_owned(),
        caption: "Built-in".to_owned(),
        palettes: vec![built_in.palette()],
        custom: None,
    }));
    tiles.extend(view.customs.iter().map(|custom| Tile {
        selection: ThemeSelection::Custom(custom.theme.id),
        name: custom.theme.name.clone(),
        caption: match custom.theme.override_count() {
            0 => "Custom".to_owned(),
            1 => "Custom · 1 change".to_owned(),
            count => format!("Custom · {count} changes"),
        },
        palettes: vec![custom.theme.palette()],
        custom: Some(custom),
    }));
    tiles
}

/// The theme the editor wears, as a source a copy can start from. Automatic is whichever built-in
/// matches the app right now.
pub(super) fn source_of(selection: ThemeSelection) -> ThemeSource {
    match selection {
        ThemeSelection::BuiltIn(built_in) => ThemeSource::BuiltIn(built_in),
        ThemeSelection::Custom(id) => ThemeSource::Custom(id),
        ThemeSelection::Auto => ThemeSource::BuiltIn(if theme::is_light() {
            BuiltInTheme::Light
        } else {
            BuiltInTheme::Dark
        }),
    }
}

/// The page.
pub fn page(
    ui: &mut Ui,
    section: SettingsSection,
    state: &mut ScriptManagerState,
    host: &EditorThemeHost,
) {
    let ctx = ui.ctx().clone();
    poll(state, host);
    let view = host.view();
    // A theme that was deleted from under the editor (an iCloud change, a delete here) ends it.
    if let Mode::Editing(editing) = &state.mode
        && !view
            .customs
            .iter()
            .any(|custom| custom.theme.id == editing.id)
    {
        state.mode = Mode::Overview;
    }
    if matches!(state.mode, Mode::Overview) {
        page_shell(
            ui,
            section,
            "Choose how your Pine Script editor looks.",
            false,
            |ui| overview(ui, state, host, &view),
        );
    } else if let Mode::Editing(editing) = &mut state.mode {
        let done = settings_theme_editor::show(
            ui,
            editing,
            &mut state.preview,
            &mut state.controls,
            &mut state.dialog,
            host,
            &view,
        );
        if done {
            state.mode = Mode::Overview;
        }
    }
    dialogs(&ctx, state, host, &view);
}

// MARK: Overview

fn overview(ui: &mut Ui, state: &mut ScriptManagerState, host: &EditorThemeHost, view: &View) {
    if let Some(why) = &view.unwritable {
        status_message(ui, why, true);
        ui.add_space(12.0);
    }
    if let Some(notice) = &view.notice {
        status_message(ui, notice, true);
        ui.add_space(12.0);
    }
    section(
        ui,
        "Editor Theme",
        Some("The colors of the code, the gutter and the editor around them."),
        |ui| {
            theme_tiles(ui, state, host, view);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if kit::push_button(ui, Some(Icon::Plus), "Create Theme", true).clicked() {
                    state.dialog = Some(Dialog::Create {
                        name: String::new(),
                        source: 0,
                        focus: true,
                        error: None,
                    });
                }
                if crate::platform::has_file_dialogs() {
                    let idle = state.flow.is_none();
                    if kit::push_button(ui, Some(Icon::Download), "Import…", idle).clicked() {
                        state.flow = Some(import_flow(ui.ctx()));
                    }
                }
            });
            if let Some((text, is_error)) = &state.status {
                ui.add_space(10.0);
                status_message(ui, text, *is_error);
            }
        },
    );
    section(ui, "Typography", None, |ui| {
        typography(ui, state, host, view);
    });
    section(
        ui,
        "Theme Customization",
        Some(
            "Change syntax colors, editor backgrounds, selection colors and the other colors the editor paints.",
        ),
        |ui| customization(ui, state, host, view),
    );
    section(
        ui,
        "Live Preview",
        Some(
            "This is the editor itself. Click in it, select text or press ⌘F to see more of the theme.",
        ),
        |ui| {
            let width = ui.available_width();
            let frame = egui::Frame::new()
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(1))
                .show(ui, |ui| {
                    state.preview.show(ui, vec2(width - 2.0, 330.0));
                });
            let _ = frame;
        },
    );
}

fn theme_tiles(ui: &mut Ui, state: &mut ScriptManagerState, host: &EditorThemeHost, view: &View) {
    let tiles = tiles(view);
    let width = ui.available_width();
    let columns = (((width + TILE_GAP) / (TILE_MIN_WIDTH + TILE_GAP)).floor() as usize).max(2);
    let tile_width = (width - TILE_GAP * (columns - 1) as f32) / columns as f32;
    let rows = tiles.len().div_ceil(columns);
    let (area, _) = ui.allocate_exact_size(
        vec2(
            width,
            rows as f32 * TILE_HEIGHT + rows.saturating_sub(1) as f32 * TILE_GAP,
        ),
        Sense::hover(),
    );
    let ids: Vec<Id> = tiles
        .iter()
        .map(|tile| Id::new(("editor-theme-tile", tile.selection.as_text())))
        .collect();
    let mut action = None;
    for (index, tile) in tiles.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(
                area.left() + (index % columns) as f32 * (tile_width + TILE_GAP),
                area.top() + (index / columns) as f32 * (TILE_HEIGHT + TILE_GAP),
            ),
            vec2(tile_width, TILE_HEIGHT),
        );
        if let Some(chosen) = draw_tile(ui, rect, tile, tile.selection == view.selected, ids[index])
        {
            action = Some(chosen);
        }
    }
    arrow_focus(ui, &ids, columns);
    if let Some(action) = action {
        apply(action, state, host, view);
    }
}

/// Moves the focus between tiles with the arrow keys, as radio buttons do.
fn arrow_focus(ui: &Ui, ids: &[Id], columns: usize) {
    let Some(at) = ids.iter().position(|id| ui.memory(|m| m.has_focus(*id))) else {
        return;
    };
    let (left, right, up, down) = ui.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, Key::ArrowLeft),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowRight),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
        )
    });
    let next = if left {
        at.checked_sub(1)
    } else if right {
        Some(at + 1).filter(|next| *next < ids.len())
    } else if up {
        at.checked_sub(columns)
    } else if down {
        Some(at + columns).filter(|next| *next < ids.len())
    } else {
        None
    };
    if let Some(next) = next {
        ui.memory_mut(|memory| memory.request_focus(ids[next]));
    }
}

/// What a tile asked for.
enum TileAction {
    Select(ThemeSelection),
    Customize(ThemeSelection),
    Duplicate(ThemeSource),
    Rename(ThemeId),
    Export(ThemeSource),
    Delete(ThemeId),
}

/// A miniature of the editor in `palette`: a gutter, and lines of code in the theme's colours.
fn paint_miniature(painter: &egui::Painter, rect: Rect, palette: &Palette) {
    let color =
        |c: crate::model::editor_theme::Rgba| Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a);
    let syntax = |role: SyntaxRole| color(palette.syntax[role.index()]);
    let surface = |surface: SurfaceColor| color(palette.surfaces[surface.index()]);
    painter.rect_filled(rect, MINIATURE_RADIUS, surface(SurfaceColor::Background));
    let gutter = Rect::from_min_max(rect.min, pos2(rect.left() + 14.0, rect.bottom()));
    painter.rect_filled(
        gutter,
        CornerRadius {
            nw: MINIATURE_RADIUS as u8,
            sw: MINIATURE_RADIUS as u8,
            ne: 0,
            se: 0,
        },
        surface(SurfaceColor::GutterBackground),
    );
    let lines: [&[(SyntaxRole, f32)]; 5] = [
        &[(SyntaxRole::Comment, 38.0)],
        &[
            (SyntaxRole::Keyword, 16.0),
            (SyntaxRole::Default, 22.0),
            (SyntaxRole::Operator, 5.0),
            (SyntaxRole::Number, 12.0),
        ],
        &[
            (SyntaxRole::UserFunction, 24.0),
            (SyntaxRole::Parameter, 16.0),
            (SyntaxRole::Operator, 6.0),
        ],
        &[
            (SyntaxRole::Namespace, 10.0),
            (SyntaxRole::BuiltinFunction, 22.0),
            (SyntaxRole::String, 20.0),
        ],
        &[
            (SyntaxRole::Type, 14.0),
            (SyntaxRole::BuiltinVariable, 24.0),
        ],
    ];
    let step = (rect.height() - 10.0) / lines.len() as f32;
    for (row, pieces) in lines.iter().enumerate() {
        let y = rect.top() + 7.0 + row as f32 * step;
        painter.rect_filled(
            Rect::from_min_size(pos2(rect.left() + 4.0, y), vec2(6.0, 3.0)),
            1.5,
            surface(SurfaceColor::LineNumber),
        );
        let mut x = rect.left() + 20.0 + if row == 2 || row == 3 { 8.0 } else { 0.0 };
        for (role, length) in pieces.iter() {
            painter.rect_filled(
                Rect::from_min_size(pos2(x, y - 0.5), vec2(*length, 4.0)),
                2.0,
                syntax(*role),
            );
            x += length + 4.0;
        }
    }
}

fn draw_tile(ui: &mut Ui, rect: Rect, tile: &Tile, selected: bool, id: Id) -> Option<TileAction> {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label_as(
        response,
        egui::WidgetType::RadioButton,
        format!("{} editor theme", tile.name),
    );
    let focused = response.has_focus();
    if focused {
        // The arrows move between tiles (see `arrow_focus`), not to whatever egui finds nearest.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    let painter = ui.painter().clone();
    let fill = if selected {
        kit::ACCENT.gamma_multiply(0.1)
    } else if response.hovered() {
        kit::primary(0.07)
    } else {
        kit::primary(0.03)
    };
    painter.rect_filled(rect, 12.0, fill);
    painter.rect_stroke(
        rect,
        12.0,
        if selected {
            Stroke::new(1.5, kit::ACCENT)
        } else {
            Stroke::new(1.0, kit::primary(0.12))
        },
        StrokeKind::Inside,
    );
    if focused {
        painter.rect_stroke(
            rect.expand(2.0),
            14.0,
            Stroke::new(2.0, theme::with_alpha(kit::ACCENT, 0.7)),
            StrokeKind::Outside,
        );
    }
    let preview = Rect::from_min_size(
        rect.min + Vec2::splat(10.0),
        vec2(rect.width() - 20.0, PREVIEW_HEIGHT),
    );
    match tile.palettes.as_slice() {
        [one] => paint_miniature(&painter, preview, one),
        [dark, light] => {
            let (left, right) = preview.split_left_right_at_fraction(0.5);
            paint_miniature(&painter.with_clip_rect(left), preview, dark);
            paint_miniature(&painter.with_clip_rect(right), preview, light);
        }
        _ => {}
    }
    painter.rect_stroke(
        preview,
        MINIATURE_RADIUS,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );

    // The name, its caption and the mark of the selection.
    let text_left = preview.left();
    let name_y = preview.bottom() + 9.0;
    let name = kit::galley(
        ui,
        &tile.name,
        kit::size::SUBHEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        preview.width() - 22.0,
    );
    let name_height = name.size().y;
    painter.galley(pos2(text_left, name_y), name, Color32::PLACEHOLDER);
    let caption = kit::galley(
        ui,
        &tile.caption,
        kit::size::CAPTION,
        kit::REGULAR,
        theme::text_secondary(),
        preview.width() - 22.0,
    );
    painter.galley(
        pos2(text_left, name_y + name_height + 1.0),
        caption,
        Color32::PLACEHOLDER,
    );
    let check = Rect::from_center_size(
        pos2(preview.right() - 7.0, name_y + name_height / 2.0),
        Vec2::splat(14.0),
    );
    if selected {
        icons::paint(&painter, check, Icon::CheckCircleFill, kit::ACCENT);
    } else {
        icons::paint(
            &painter,
            check,
            Icon::Circle,
            theme::text_secondary().gamma_multiply(0.6),
        );
    }

    let mut action = response
        .clicked()
        .then_some(TileAction::Select(tile.selection));
    // The menu: a button over the miniature's corner, and the right-click anywhere on the tile.
    let menu_rect = Rect::from_center_size(
        pos2(preview.right() - 13.0, preview.top() + 13.0),
        Vec2::splat(22.0),
    );
    let menu_response = ui
        .interact(menu_rect, id.with("menu"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let menu_response =
        crate::ui::accessibility::label(menu_response, format!("Actions for {}", tile.name));
    painter.circle_filled(menu_rect.center(), 11.0, Color32::from_black_alpha(110));
    icons::paint(
        &painter,
        Rect::from_center_size(menu_rect.center(), Vec2::splat(13.0)),
        Icon::Ellipsis,
        Color32::WHITE,
    );
    for popup in [
        kit::menu_popup(&menu_response),
        kit::context_popup(&response),
    ] {
        popup.show(|ui| {
            kit::menu_scope(ui, 0.0);
            kit::menu_fit(
                ui,
                if tile.custom.is_some() {
                    &["Edit…", "Duplicate", "Rename…", "Export…", "Delete…"][..]
                } else {
                    &["Duplicate & Customize", "Export…"][..]
                },
            );
            if let Some(chosen) = tile_menu(ui, tile) {
                action = Some(chosen);
            }
        });
    }
    action
}

fn tile_menu(ui: &mut Ui, tile: &Tile) -> Option<TileAction> {
    let mut action = None;
    let row = |ui: &mut Ui, icon: Icon, label: &str, destructive: bool| {
        let clicked = kit::menu_item(ui, icon, label, true, destructive);
        if clicked {
            ui.close();
        }
        clicked
    };
    match (tile.selection, tile.custom) {
        (_, Some(custom)) => {
            let id = custom.theme.id;
            if row(ui, Icon::Pencil, "Edit…", false) {
                action = Some(TileAction::Customize(tile.selection));
            }
            if row(ui, Icon::CopyDoc, "Duplicate", false) {
                action = Some(TileAction::Duplicate(ThemeSource::Custom(id)));
            }
            if row(ui, Icon::TextFormat, "Rename…", false) {
                action = Some(TileAction::Rename(id));
            }
            if row(ui, Icon::Download, "Export…", false) {
                action = Some(TileAction::Export(ThemeSource::Custom(id)));
            }
            kit::menu_separator(ui);
            if row(ui, Icon::Trash, "Delete…", true) {
                action = Some(TileAction::Delete(id));
            }
        }
        (selection, None) => {
            if row(ui, Icon::Palette, "Duplicate & Customize", false) {
                action = Some(TileAction::Customize(selection));
            }
            if selection != ThemeSelection::Auto && row(ui, Icon::Download, "Export…", false) {
                action = Some(TileAction::Export(source_of(selection)));
            }
        }
    }
    action
}

/// Carries out what a tile or a button asked for.
fn apply(action: TileAction, state: &mut ScriptManagerState, host: &EditorThemeHost, view: &View) {
    state.status = None;
    match action {
        TileAction::Select(selection) => host.select(selection),
        TileAction::Customize(selection) => customize(state, host, selection, view),
        TileAction::Duplicate(source) => match host.duplicate(source) {
            Ok(_) => {}
            Err(error) => state.status = Some((error, true)),
        },
        TileAction::Rename(id) => {
            let name = host.theme(id).map(|theme| theme.name).unwrap_or_default();
            state.dialog = Some(Dialog::Rename {
                id,
                name,
                focus: true,
                error: None,
            });
        }
        TileAction::Export(source) => match host.export(source) {
            Ok((stem, text)) => state.flow = Some(export_flow(&stem, text)),
            Err(error) => state.status = Some((error, true)),
        },
        TileAction::Delete(id) => state.dialog = Some(Dialog::Delete(id)),
    }
}

/// Opens a theme for editing: a custom theme as it is, anything else as a copy of its own.
fn customize(
    state: &mut ScriptManagerState,
    host: &EditorThemeHost,
    selection: ThemeSelection,
    _view: &View,
) {
    let id = match selection {
        ThemeSelection::Custom(id) => {
            host.select(selection);
            id
        }
        other => match host.duplicate(source_of(other)) {
            Ok(id) => id,
            Err(error) => {
                state.status = Some((error, true));
                return;
            }
        },
    };
    if let Some(theme) = host.theme(id) {
        state.mode = Mode::Editing(Box::new(Editing::new(theme)));
    }
}

// MARK: Typography

/// What the three typography controls changed this frame.
#[derive(Default)]
pub(super) struct TypographyChange {
    pub(super) family: Option<Option<String>>,
    pub(super) size: Option<f32>,
    pub(super) line_height: Option<LineHeight>,
    /// A trailing reset was clicked: family, size, line height.
    pub(super) reset: [bool; 3],
}

/// What the typography controls show.
pub(super) struct TypographyShown<'a> {
    pub(super) family: Option<&'a str>,
    pub(super) size: f32,
    pub(super) line_height: LineHeight,
    /// Which of the three a theme sets itself: each gets a reset beside it.
    pub(super) resettable: [bool; 3],
    pub(super) status: &'a FontStatus,
    pub(super) fonts: &'a [String],
    pub(super) fonts_ready: bool,
}

/// The three typography rows: font, size and line height.
pub(super) fn typography_controls(
    ui: &mut Ui,
    controls: &mut Controls,
    salt: &str,
    shown: &TypographyShown,
) -> TypographyChange {
    let mut change = TypographyChange::default();
    ui.spacing_mut().item_spacing.y = 8.0;
    let hint = match shown.status {
        FontStatus::Missing(name) => {
            format!("“{name}” is not installed on this Mac, so the default is used until it is.")
        }
        FontStatus::Failed(name, why) => format!("“{name}” could not be loaded: {why}"),
        FontStatus::Loading(name) => format!("Loading “{name}”…"),
        _ => "Built in: JetBrains Mono. Or any monospaced font installed on this Mac.".to_owned(),
    };
    settings_card::card_reserving(ui, Icon::TextFormat, "Font", Some(&hint), 250.0, |ui| {
        reset_button(
            ui,
            shown.resettable[0],
            "Use the editor's own font",
            &mut change.reset[0],
        );
        change.family = settings_font_picker::show(
            ui,
            &mut controls.picker,
            salt,
            shown.family,
            shown.status,
            shown.fonts,
            shown.fonts_ready,
            true,
        );
    });
    settings_card::card_reserving(
        ui,
        Icon::Sliders,
        "Font size",
        Some(&format!("{MIN_FONT_SIZE} to {MAX_FONT_SIZE} points.")),
        120.0,
        |ui| {
            reset_button(
                ui,
                shown.resettable[1],
                "Use the editor's own size",
                &mut change.reset[1],
            );
            change.size = size_field(ui, controls, salt, shown.size);
        },
    );
    settings_card::card_reserving(
        ui,
        Icon::List,
        "Line height",
        Some("The space each line of code gets."),
        290.0,
        |ui| {
            reset_button(
                ui,
                shown.resettable[2],
                "Use the editor's own line height",
                &mut change.reset[2],
            );
            let mut value = shown.line_height;
            let options: Vec<(LineHeight, &str)> = LineHeight::ALL
                .iter()
                .map(|height| (*height, height.label()))
                .collect();
            kit::segmented(
                ui,
                Id::new(("editor-line-height", salt)),
                &options,
                &mut value,
            );
            if value != shown.line_height {
                change.line_height = Some(value);
            }
        },
    );
    change
}

/// A small "back to the editor's own" button, drawn only for a value a theme sets.
fn reset_button(ui: &mut Ui, shown: bool, tooltip: &str, clicked: &mut bool) {
    if shown && icons::button(ui, Icon::Undo, 22.0, theme::text_secondary(), tooltip).clicked() {
        *clicked = true;
    }
}

/// The size: a number to type and a stepper. Returns the size when it changes. A number that is
/// out of range is clamped, one that is not a number is dropped and the field shows the size again.
fn size_field(ui: &mut Ui, controls: &mut Controls, salt: &str, size: f32) -> Option<f32> {
    let id = Id::new(("editor-font-size", salt));
    let mut result = None;
    let stepped = kit::stepper(
        ui,
        id.with("step"),
        size.round() as i64,
        MIN_FONT_SIZE as i64..=MAX_FONT_SIZE as i64,
        1,
    );
    if let Some(next) = stepped {
        controls.size_text = None;
        result = Some(next as f32);
    }
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut text = match (&controls.size_text, focused) {
        (Some(typed), true) => typed.clone(),
        _ => format_size(size),
    };
    let frame = kit::field_frame(focused).inner_margin(egui::Margin::symmetric(8, 3));
    let response = frame
        .show(ui, |ui| {
            kit::plain_field(ui, id, &mut text, "", kit::size::BODY, kit::REGULAR, 34.0)
        })
        .inner;
    let response = crate::ui::accessibility::label(response, "Font size in points");
    if response.changed() {
        controls.size_text = Some(text.clone());
    }
    let enter = focused && ui.input(|input| input.key_pressed(Key::Enter));
    if response.lost_focus() || enter {
        if let Some(typed) = controls.size_text.take()
            && let Some(parsed) = parse_size(&typed)
            && (parsed - size).abs() > f32::EPSILON
        {
            result = Some(parsed);
        }
        if enter {
            response.surrender_focus();
        }
    }
    result
}

/// "13", "13.5" or "13 pt", as a size in range; anything else is not a size.
pub(super) fn parse_size(text: &str) -> Option<f32> {
    let number = text.trim().trim_end_matches("pt").trim().replace(',', ".");
    number
        .parse::<f32>()
        .ok()
        .filter(|size| size.is_finite())
        .map(clamp_size)
}

pub(super) fn format_size(size: f32) -> String {
    if (size - size.round()).abs() < f32::EPSILON {
        format!("{}", size as i32)
    } else {
        format!("{size:.1}")
    }
}

fn typography(ui: &mut Ui, state: &mut ScriptManagerState, host: &EditorThemeHost, view: &View) {
    // A theme that sets one of these says so, and the field it sets is the theme's to change.
    let set_by = match view.selected {
        ThemeSelection::Custom(id) => view
            .customs
            .iter()
            .find(|custom| custom.theme.id == id)
            .filter(|custom| !custom.theme.typography.is_empty())
            .map(|custom| custom.theme.name.as_str()),
        _ => None,
    };
    let shown = TypographyShown {
        family: view.effective.font_family.as_deref(),
        size: view.effective.font_size,
        line_height: view.effective.line_height,
        resettable: [false; 3],
        status: &view.font,
        fonts: &view.fonts,
        fonts_ready: view.fonts_ready,
    };
    let change = typography_controls(ui, &mut state.controls, "editor", &shown);
    let mut typography = view.typography.clone();
    let mut changed = false;
    if let Some(family) = change.family {
        typography.font_family = family;
        changed = true;
    }
    if let Some(size) = change.size {
        typography.font_size = size;
        changed = true;
    }
    if let Some(height) = change.line_height {
        typography.line_height = height;
        changed = true;
    }
    if changed {
        host.set_typography(typography);
    }
    if let Some(name) = set_by {
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(kit::text(
                format!("“{name}” sets some of these itself. Change them under Edit ▸ Typography."),
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
    }
}

// MARK: Customization

fn customization(ui: &mut Ui, state: &mut ScriptManagerState, host: &EditorThemeHost, view: &View) {
    let custom = matches!(view.selected, ThemeSelection::Custom(_));
    let (title, hint, button) = if custom {
        (
            "Edit this theme",
            "Change any color it paints, with the editor beside you.",
            "Edit Current Theme",
        )
    } else {
        (
            "Make your own",
            "Built-in themes can't be changed. Duplicate & Customize makes a copy that is yours to edit.",
            "Duplicate & Customize",
        )
    };
    settings_card::card_reserving(ui, Icon::Palette, title, Some(hint), 190.0, |ui| {
        if kit::push_button(
            ui,
            Some(if custom { Icon::Pencil } else { Icon::CopyDoc }),
            button,
            true,
        )
        .clicked()
        {
            apply(TileAction::Customize(view.selected), state, host, view);
        }
    });
}

// MARK: Files

fn import_flow(ctx: &egui::Context) -> Flow {
    let (sender, receiver) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = sender.send(theme_files::choose_and_read(MAX_IMPORT_BYTES));
        ctx.request_repaint();
    });
    Flow::Import(receiver)
}

fn export_flow(stem: &str, text: String) -> Flow {
    let (sender, receiver) = channel();
    let name = stem.to_owned();
    // The dialog is asked from the frame that clicked, so the repaint comes from the thread.
    let stem = stem.to_owned();
    std::thread::spawn(move || {
        let _ = sender.send(theme_files::save(&stem, &text));
    });
    Flow::Export(receiver, name)
}

/// Takes the answer of a file dialog that has finished.
fn poll(state: &mut ScriptManagerState, host: &EditorThemeHost) {
    let Some(flow) = state.flow.take() else {
        return;
    };
    match flow {
        Flow::Import(receiver) => match receiver.try_recv() {
            Ok(Ok(Some(text))) => {
                state.status = Some(match host.import(&text) {
                    Ok(imported) => {
                        let skipped = match imported.skipped.len() {
                            0 => String::new(),
                            1 => format!(
                                " One value in the file was not usable and was skipped: {}",
                                imported.skipped[0]
                            ),
                            count => format!(
                                " {count} values in the file were not usable and were skipped, such as: {}",
                                imported.skipped[0]
                            ),
                        };
                        (format!("Imported “{}”.{skipped}", imported.name), false)
                    }
                    Err(error) => (error, true),
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => state.status = Some((error, true)),
            Err(TryRecvError::Empty) => state.flow = Some(Flow::Import(receiver)),
            Err(TryRecvError::Disconnected) => {}
        },
        Flow::Export(receiver, name) => match receiver.try_recv() {
            Ok(Ok(Some(path))) => {
                let file = path
                    .file_name()
                    .map(|file| file.to_string_lossy().into_owned())
                    .unwrap_or(name);
                state.status = Some((format!("Exported “{file}”."), false));
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => state.status = Some((error, true)),
            Err(TryRecvError::Empty) => state.flow = Some(Flow::Export(receiver, name)),
            Err(TryRecvError::Disconnected) => {}
        },
    }
}

// MARK: Dialogs

fn dialogs(
    ctx: &egui::Context,
    state: &mut ScriptManagerState,
    host: &EditorThemeHost,
    view: &View,
) {
    let Some(dialog) = state.dialog.take() else {
        return;
    };
    state.dialog = match dialog {
        Dialog::Create {
            mut name,
            mut source,
            mut focus,
            mut error,
        } => {
            let sources = create_sources(view);
            let problem = name_problem(host, &name, None);
            match name_dialog(
                ctx,
                "editor-theme-create",
                "Create Editor Theme",
                "Start from a theme you like, then change what you want.",
                &mut name,
                &mut focus,
                problem.as_deref().or(error.as_deref()),
                "Create Theme",
                Some((&mut source, &sources)),
            ) {
                Outcome::Confirm => {
                    match host.create(&name, sources[source.min(sources.len() - 1)].1) {
                        Ok(id) => {
                            if let Some(theme) = host.theme(id) {
                                state.mode = Mode::Editing(Box::new(Editing::new(theme)));
                            }
                            None
                        }
                        Err(message) => {
                            error = Some(message);
                            Some(Dialog::Create {
                                name,
                                source,
                                focus,
                                error,
                            })
                        }
                    }
                }
                Outcome::Cancel => None,
                Outcome::Stay => Some(Dialog::Create {
                    name,
                    source,
                    focus,
                    error: None,
                }),
            }
        }
        Dialog::Rename {
            id,
            mut name,
            mut focus,
            mut error,
        } => {
            let problem = name_problem(host, &name, Some(id));
            match name_dialog(
                ctx,
                "editor-theme-rename",
                "Rename Theme",
                "Names are for you: any characters, up to 48 of them.",
                &mut name,
                &mut focus,
                problem.as_deref().or(error.as_deref()),
                "Rename",
                None,
            ) {
                Outcome::Confirm => match host.rename(id, &name) {
                    Ok(()) => {
                        // The editor's snapshot keeps the old name; the name is not a change to undo.
                        if let Mode::Editing(editing) = &mut state.mode {
                            editing.renamed(&name);
                        }
                        None
                    }
                    Err(message) => {
                        error = Some(message);
                        Some(Dialog::Rename {
                            id,
                            name,
                            focus,
                            error,
                        })
                    }
                },
                Outcome::Cancel => None,
                Outcome::Stay => Some(Dialog::Rename {
                    id,
                    name,
                    focus,
                    error: None,
                }),
            }
        }
        Dialog::Delete(id) => {
            let name = host.theme(id).map(|theme| theme.name).unwrap_or_default();
            let message = format!(
                "“{name}” will be deleted. {}",
                if view.selected == ThemeSelection::Custom(id) {
                    "The editor goes back to following the app."
                } else {
                    "Nothing else changes."
                }
            );
            match kit::confirm(
                ctx,
                Id::new("editor-theme-delete"),
                &kit::Confirm {
                    icon: Icon::Trash,
                    title: "Delete this theme?",
                    message: &message,
                    destructive: "Delete Theme",
                },
            ) {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    state.status = host.delete(id).err().map(|error| (error, true));
                    None
                }
                Some(kit::ConfirmOutcome::Cancelled) => None,
                None => Some(Dialog::Delete(id)),
            }
        }
        Dialog::ResetAll(id) => {
            let name = host.theme(id).map(|theme| theme.name).unwrap_or_default();
            let message = format!(
                "Every color and the typography of “{name}” go back to the theme it started from. Reset Changes will bring them back until you press Done."
            );
            match kit::confirm(
                ctx,
                Id::new("editor-theme-reset"),
                &kit::Confirm {
                    icon: Icon::Undo,
                    title: "Reset this theme?",
                    message: &message,
                    destructive: "Reset Theme",
                },
            ) {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    state.status = host.reset_all(id).err().map(|error| (error, true));
                    None
                }
                Some(kit::ConfirmOutcome::Cancelled) => None,
                None => Some(Dialog::ResetAll(id)),
            }
        }
    };
}

/// The themes a new one can start from, with their names.
fn create_sources(view: &View) -> Vec<(String, ThemeSource)> {
    BuiltInTheme::ALL
        .iter()
        .map(|built_in| (built_in.name().to_owned(), ThemeSource::BuiltIn(*built_in)))
        .chain(view.customs.iter().map(|custom| {
            (
                custom.theme.name.clone(),
                ThemeSource::Custom(custom.theme.id),
            )
        }))
        .collect()
}

/// Why `name` cannot be used, once something has been typed.
fn name_problem(host: &EditorThemeHost, name: &str, renaming: Option<ThemeId>) -> Option<String> {
    (!name.trim().is_empty())
        .then(|| host.name_problem(name, renaming))
        .flatten()
}

/// A small label over a field of a sheet.
fn caption(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::MEDIUM,
        theme::text_secondary(),
    ));
    ui.add_space(6.0);
}

/// A choice from a short list, drawn as the sheet's text fields are (same frame, same height) and
/// as wide as they are, with the choices in a menu under it. Returns the index chosen this frame.
fn field_select(
    ui: &mut Ui,
    id: Id,
    icon: Icon,
    labels: &[&str],
    selected: usize,
) -> Option<usize> {
    let current = labels.get(selected).copied().unwrap_or_default();
    let shown = kit::field_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::inline(ui, icon, 13.0, theme::text_secondary());
            ui.label(kit::text(
                current,
                kit::size::BODY,
                kit::REGULAR,
                theme::text_primary(),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                icons::inline(ui, Icon::ChevronUpDown, 12.0, theme::text_secondary());
            });
        });
    });
    let rect = shown.response.rect;
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, format!("Based on: {current}"));
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            10.0,
            Stroke::new(2.0, theme::with_alpha(kit::ACCENT, 0.7)),
            StrokeKind::Outside,
        );
    }
    let mut picked = None;
    kit::menu_popup(&response).show(|ui| {
        kit::menu_scope(ui, rect.width() - 16.0);
        for (index, label) in labels.iter().enumerate() {
            let row = kit::menu_row(
                ui,
                kit::MenuRow {
                    current: index == selected,
                    reserve_icon: false,
                    ..kit::MenuRow::new(label)
                },
            );
            if row.clicked() {
                picked = Some(index);
                ui.close();
            }
        }
    });
    picked
}

enum Outcome {
    Confirm,
    Cancel,
    Stay,
}

#[allow(clippy::too_many_arguments)]
fn name_dialog(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    subtitle: &str,
    name: &mut String,
    focus: &mut bool,
    problem: Option<&str>,
    action: &str,
    based_on: Option<(&mut usize, &[(String, ThemeSource)])>,
) -> Outcome {
    let valid = !name.trim().is_empty() && problem.is_none();
    let mut outcome = Outcome::Stay;
    sheet::show(ctx, Id::new(id), DIALOG_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(ui, Icon::Palette, title, subtitle, theme::ACCENT);
            ui.add_space(20.0);
            caption(ui, "Name");
            crate::ui::chart_templates::text_field(
                ui,
                Id::new((id, "name")),
                Icon::Tag,
                name,
                "Theme name",
                focus,
                MAX_NAME_CHARS,
            );
            if let Some(problem) = problem {
                ui.add_space(8.0);
                ui.label(kit::text(
                    problem,
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    kit::ORANGE,
                ));
            }
            if let Some((selected, sources)) = based_on {
                ui.add_space(18.0);
                caption(ui, "Based on");
                let labels: Vec<&str> = sources.iter().map(|(label, _)| label.as_str()).collect();
                if let Some(picked) = field_select(
                    ui,
                    Id::new((id, "based-on")),
                    Icon::Palette,
                    &labels,
                    *selected,
                ) {
                    *selected = picked;
                }
            }
            ui.add_space(22.0);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, action, true, valid).clicked() {
                outcome = Outcome::Confirm;
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        outcome = Outcome::Cancel;
    } else if matches!(outcome, Outcome::Stay)
        && valid
        && ctx.input(|input| input.key_pressed(Key::Enter))
    {
        outcome = Outcome::Confirm;
    }
    outcome
}

#[cfg(test)]
mod tests;
