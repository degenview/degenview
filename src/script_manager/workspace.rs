//! The detail area: the empty states, and for a selected script the split between the code
//! editor and the preview pane. Ported from `ScriptWorkspaceView` and the detail part of
//! `ScriptManagerView`.

use super::debug::DebugPanel;
use super::debug_panel;
use super::drawer;
use super::editor;
use super::prefs::{ChartPosition, DrawerTab, LayoutPrefs};
use super::profile_marks::ProfileColumn;
use super::profile_ui::{self, TabView};
use super::profiler::{Profile, Status};
use super::{Cmd, OpenScript, ScriptManager};
use crate::pine::token::Diagnostic;
use crate::ui::icons::{self, Icon};
use crate::ui::split::{self, Axis, DefaultLength, SplitSpec, SplitState};
use crate::ui::theme;
use eframe::egui::{
    self, Align, Color32, FontId, Id, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use std::cell::{Cell, RefCell};
use std::time::Instant;

pub use super::preview::PreviewState;

/// What the preview pane reads of the open script.
pub(crate) struct PreviewInput {
    pub name: String,
    pub script_type: super::model::ScriptType,
    pub source: String,
    pub diagnostics: Vec<Diagnostic>,
    /// `diagnostics` and the script's runtime failures: what the Problems tab lists and counts.
    pub problems: Vec<Diagnostic>,
    /// The source changed after `diagnostics` were made; the preview waits for them to catch up.
    pub compile_pending: bool,
}

impl ScriptManager {
    /// How the script of `source`, whose compile result is `compile`, failed when it ran: on the
    /// chart or in the profile, and only if that run was of this very text.
    pub(super) fn runtime_problems(&self, source: &str, compile: &[Diagnostic]) -> Vec<Diagnostic> {
        super::problems::runtime_problems(
            compile,
            self.preview.runtime_error_for(source),
            self.profiler.failure(self.profiler_now.as_ref()),
        )
    }

    pub(super) fn detail_ui(&mut self, ui: &mut Ui) {
        if self.open.is_some() {
            self.workspace_ui(ui);
        } else if self.loaded && self.scripts.is_empty() {
            self.empty_library(ui);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Select a script")
                        .size(13.0)
                        .color(theme::text_secondary()),
                );
            });
        }
    }

    fn empty_library(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let mut column = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(Align::Center)),
        );
        column.add_space((rect.height() / 2.0 - 120.0).max(24.0));
        column.spacing_mut().item_spacing.y = 12.0;
        column.label(crate::ui::kit::text(
            "No scripts yet",
            13.0,
            crate::ui::kit::SEMIBOLD,
            theme::text_primary(),
        ));
        column.label(
            RichText::new("Write a Pine Script indicator or strategy and run it on your charts.")
                .size(13.0)
                .color(theme::text_secondary()),
        );
        column.add_space(4.0);
        if prominent_button(&mut column, Icon::Plus, "Create Your First Script").clicked() {
            self.cmds.push(Cmd::NewScript);
        }
        if bordered_button(&mut column, Icon::Download, "Add Example Scripts").clicked() {
            self.cmds.push(Cmd::AddExamples);
        }
        column.add_space(4.0);
        // Plain accent links, as Swift's `Link`s draw.
        column.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let links = [
                ("Pine Script documentation", DOCS_URL),
                ("Community scripts", super::sidebar::COMMUNITY_URL),
            ];
            let widths: Vec<f32> = links
                .iter()
                .map(|(label, _)| {
                    crate::ui::kit::galley(
                        ui,
                        label,
                        13.0,
                        crate::ui::kit::REGULAR,
                        theme::text_primary(),
                        300.0,
                    )
                    .size()
                    .x
                })
                .collect();
            let total = widths.iter().sum::<f32>() + 16.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for (label, url) in links {
                let response = ui
                    .add(
                        egui::Label::new(crate::ui::kit::text(
                            label,
                            13.0,
                            crate::ui::kit::REGULAR,
                            crate::ui::kit::ACCENT,
                        ))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(url);
                if response.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        });
    }

    fn workspace_ui(&mut self, ui: &mut Ui) {
        let position = self.prefs.chart_position;
        let horizontal = position == ChartPosition::Left;
        let spec = if horizontal {
            SplitSpec {
                axis: Axis::Horizontal,
                secondary_first: true,
                min_primary: 320.0,
                min_secondary: 320.0,
                max_secondary: f32::MAX,
                default: DefaultLength::Fraction(0.5),
            }
        } else {
            SplitSpec {
                axis: Axis::Vertical,
                secondary_first: position == ChartPosition::Top,
                min_primary: 160.0,
                min_secondary: 220.0,
                max_secondary: f32::MAX,
                default: DefaultLength::Fraction(0.5),
            }
        };
        let mut state = SplitState {
            length: if horizontal {
                self.prefs.chart_width
            } else {
                self.prefs.chart_height
            },
            visible: self.prefs.chart_visible,
        };
        let input = self.open.as_ref().map(|open| PreviewInput {
            name: open.name.clone(),
            script_type: open.script_type,
            source: open.editor.source.clone(),
            diagnostics: open.editor.diagnostics.clone(),
            problems: Vec::new(),
            compile_pending: open.editor.compile_pending(),
        });
        let Some(mut input) = input else {
            return;
        };
        // One list of problems for the tab, its count and the editor: what the compiler said plus
        // how the script failed when it ran, on the chart or in the profile.
        let runtime = self.runtime_problems(&input.source, &input.diagnostics);
        input.problems = super::problems::combined(&input.diagnostics, &runtime);
        if let Some(open) = self.open.as_mut() {
            open.editor.set_runtime_problems(runtime);
        }

        let cmds = RefCell::new(Vec::new());
        let changed = Cell::new(false);
        let profile_column = self.profiler.column(self.profiler_now.as_ref());
        let profile_status = self.profiler.status(self.profiler_now.as_ref());
        let profile_lines_match = self.profiler.lines_match(self.profiler_now.as_ref());
        {
            let open = &mut self.open;
            let preview = &mut self.preview;
            let copied_at = &mut self.problems_copied;
            let logs = &mut self.logs.view;
            // The editor draws the debugger's panel and the drawer shows its logs, so both reach the
            // layout preferences and the debugger through one cell.
            let shared = RefCell::new((&mut self.prefs, &mut self.debug));
            let profile = ProfileTab {
                status: profile_status,
                profile: self.profiler.profile(),
                lines_match: profile_lines_match,
            };
            let side = Side {
                input: &input,
                profile: &profile,
            };
            split::show(
                ui,
                Id::new(("script-workspace", horizontal)),
                spec,
                &mut state,
                |ui| editor_pane(ui, open, &shared, &cmds, &changed, profile_column.clone()),
                |ui| preview_pane(ui, preview, &shared, copied_at, &side, logs, &cmds),
            );
        }
        if horizontal {
            self.prefs.chart_width = state.length;
        } else {
            self.prefs.chart_height = state.length;
        }
        self.cmds.extend(cmds.into_inner());
        if changed.get()
            && let Some(open) = self.open.as_mut()
        {
            // Compiled once typing pauses (`end_frame`), so a name still being typed isn't an error.
            open.editor.request_compile();
        }
    }
}

type Shared<'a> = RefCell<(&'a mut LayoutPrefs, &'a mut DebugPanel)>;

/// What the drawer reads of the open script and of its profile.
struct Side<'a> {
    input: &'a PreviewInput,
    profile: &'a ProfileTab<'a>,
}

/// What the drawer's Profile tab shows.
struct ProfileTab<'a> {
    status: Status,
    profile: Option<&'a Profile>,
    lines_match: bool,
}

/// The editor, and under it the debugger's panel when that is shown.
fn editor_pane(
    ui: &mut Ui,
    open: &mut Option<OpenScript>,
    shared: &Shared<'_>,
    cmds: &RefCell<Vec<Cmd>>,
    changed: &Cell<bool>,
    profile_column: Option<ProfileColumn>,
) {
    let Some(open) = open.as_mut() else {
        return;
    };
    let marks = shared.borrow_mut().1.marks();
    open.editor.set_debug_marks(marks);
    open.editor.set_profile_column(profile_column);
    let (shown, height) = {
        let guard = shared.borrow();
        (guard.0.debug_open, guard.0.debug_height)
    };
    let open = RefCell::new(open);
    let draw_editor = |ui: &mut Ui| {
        let output = editor::show(ui, &mut open.borrow_mut().editor);
        if output.changed {
            changed.set(true);
        }
        if output.save {
            cmds.borrow_mut().push(Cmd::Save);
        }
        for event in output.gutter {
            cmds.borrow_mut().push(Cmd::Gutter(event));
        }
    };
    if !shown {
        draw_editor(ui);
        return;
    }
    let spec = SplitSpec {
        axis: Axis::Vertical,
        secondary_first: false,
        min_primary: 140.0,
        min_secondary: 150.0,
        max_secondary: f32::MAX,
        default: DefaultLength::Points(280.0),
    };
    let mut state = SplitState {
        length: height,
        visible: true,
    };
    split::show(
        ui,
        Id::new("script-debug-split"),
        spec,
        &mut state,
        draw_editor,
        |ui| {
            let source = open.borrow().editor.source.clone();
            let mut guard = shared.borrow_mut();
            let (prefs, debug) = &mut *guard;
            let out_of_date = debug.is_out_of_date(&source);
            for action in debug_panel::show(ui, debug, prefs, &source, out_of_date) {
                cmds.borrow_mut().push(Cmd::DebugPanel(action));
            }
        },
    );
    shared.borrow_mut().0.debug_height = state.length;
}

/// The preview header, the chart, and the drawer with its tab bar.
fn preview_pane(
    ui: &mut Ui,
    preview: &mut PreviewState,
    shared: &Shared<'_>,
    copied_at: &mut Option<Instant>,
    side: &Side<'_>,
    logs: &mut crate::ui::pine_logs_view::LogsState,
    cmds: &RefCell<Vec<Cmd>>,
) {
    let (input, profile) = (side.input, side.profile);
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, theme::background());
    let bar_rect = super::bottom_bar::rect(full);
    let top = super::preview::header(ui, preview, full);
    let middle = egui::Rect::from_min_max(pos2(full.min.x, top), pos2(full.max.x, bar_rect.min.y));

    let (drawer_height, drawer_open, drawer_tab) = {
        let guard = shared.borrow();
        (
            guard.0.drawer_height,
            guard.0.drawer_open,
            guard.0.drawer_tab,
        )
    };
    let mut drawer_state = SplitState {
        length: drawer_height,
        visible: drawer_open,
    };
    let spec = SplitSpec {
        axis: Axis::Vertical,
        secondary_first: false,
        min_primary: 140.0,
        min_secondary: 90.0,
        max_secondary: f32::MAX,
        default: DefaultLength::Fraction(0.38),
    };
    let preview = RefCell::new(preview);
    let mut middle_ui = ui.new_child(egui::UiBuilder::new().max_rect(middle));
    middle_ui.set_clip_rect(middle.intersect(ui.clip_rect()));
    let copied = RefCell::new(*copied_at);
    let tab = drawer_tab;
    let problems = input.problems.clone();
    split::show(
        &mut middle_ui,
        Id::new("script-preview-drawer"),
        spec,
        &mut drawer_state,
        |ui| super::preview::chart(ui, &mut preview.borrow_mut(), input, cmds),
        |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, theme::background());
            match tab {
                DrawerTab::Inputs => {
                    let edits = {
                        let preview = preview.borrow();
                        super::inputs::show(ui, &preview.input_schema, &preview.input_values)
                    };
                    for edit in edits {
                        preview.borrow_mut().apply_input_edit(edit);
                    }
                }
                DrawerTab::Report => drawer::report(ui, preview.borrow().visual()),
                DrawerTab::Problems => {
                    let mut at = copied.borrow_mut();
                    if let Some(diagnostic) = drawer::problems(ui, &problems, &mut at) {
                        cmds.borrow_mut().push(Cmd::Reveal(diagnostic));
                    }
                }
                DrawerTab::Logs => {
                    let preview = preview.borrow();
                    let guard = shared.borrow();
                    super::logs::tab(ui, &preview, guard.1, logs, &input.name, cmds);
                }
                DrawerTab::Profile => {
                    let view = TabView {
                        status: &profile.status,
                        profile: profile.profile,
                        lines_match: profile.lines_match,
                    };
                    for action in profile_ui::tab(ui, &view) {
                        cmds.borrow_mut().push(Cmd::from(action));
                    }
                }
            }
        },
    );
    shared.borrow_mut().0.drawer_height = drawer_state.length;
    *copied_at = copied.into_inner();

    let status = preview.borrow().status_text();
    let output = drawer::bar(
        ui,
        bar_rect,
        drawer_tab,
        drawer_open,
        &input.problems,
        &status,
    );
    if let Some(tab) = output.tab {
        cmds.borrow_mut().push(Cmd::SetDrawerTab(tab));
    }
    if output.toggle {
        cmds.borrow_mut().push(Cmd::ToggleDrawer);
    }
}

fn prominent_button(ui: &mut Ui, icon: Icon, label: &str) -> egui::Response {
    sized_button(ui, icon, label, Some(theme::SYSTEM_ACCENT))
}

fn bordered_button(ui: &mut Ui, icon: Icon, label: &str) -> egui::Response {
    sized_button(ui, icon, label, None)
}

/// A large (`.controlSize(.large)`) button with a leading icon.
fn sized_button(ui: &mut Ui, icon: Icon, label: &str, fill: Option<Color32>) -> egui::Response {
    let font = FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 52.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let base = fill.unwrap_or_else(theme::control_fill);
    let ink = if fill.is_some() {
        Color32::WHITE
    } else {
        theme::text_primary()
    };
    let color = if response.is_pointer_button_down_on() {
        base.gamma_multiply(0.8)
    } else if response.hovered() {
        base.gamma_multiply(1.12)
    } else {
        base
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 7.0, color);
    if fill.is_none() {
        painter.rect_stroke(
            rect,
            7.0,
            Stroke::new(1.0, theme::separator()),
            egui::StrokeKind::Inside,
        );
    }
    icons::paint(
        painter,
        egui::Rect::from_center_size(pos2(rect.min.x + 20.0, rect.center().y), Vec2::splat(13.0)),
        icon,
        ink,
    );
    painter.galley(
        pos2(rect.min.x + 36.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

const DOCS_URL: &str = "https://www.tradingview.com/pine-script-docs/";

#[allow(dead_code)]
fn _keep(_: Instant, _: Align) {}
