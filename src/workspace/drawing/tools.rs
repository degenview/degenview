//! The tab's drawing tools: the armed tool and the Zoom In mode, the strip's remembered options,
//! the selection, the copied style, and the dialogs about drawings. `drawing` and its children
//! act on it; the grid, the chart menu and the shortcuts go through the methods below.

use crate::model::{CursorMode, Drawing, DrawingStyle, MagnetMode, Tool};
use crate::store::drawing_template::{DrawingPrefs, DrawingTemplateStore};
use crate::ui::drawing::properties::DrawingProperties;

pub(in crate::workspace) struct DrawingTools {
    pub(super) active: Tool,
    /// The Zoom In mode of the tool rail. A mode of its own, never a drawing tool: a drag on a
    /// plot selects a rectangle to zoom to. Exclusive with `active`; not saved.
    pub(super) zoom_armed: bool,
    /// The cursor the pointer rests on when no drawing tool or Zoom In holds it. Seeded from the
    /// saved options (`prefs.cursor`) and written back to them when the user picks another.
    pub(super) cursor: CursorMode,
    /// The strip's options: the magnet, keep drawing, and the tool each grouped tile shows.
    pub(super) prefs: DrawingPrefs,
    /// The strip's options as last saved, to notice a change.
    pub(super) saved_prefs: DrawingPrefs,
    /// Saved drawing templates, each tool's default style and the strip's remembered options.
    /// Opened on first use (`ChartWorkspace::drawing_templates`).
    pub(super) templates: Option<DrawingTemplateStore>,
    /// Every drawing is hidden (view state only: nothing is changed or saved).
    pub(super) hide_all: bool,
    /// The selected drawings of one chart: `(chart id, drawing ids)`.
    pub(super) selected: Option<(u64, Vec<u64>)>,
    /// The style last copied from a drawing, for Paste style.
    pub(super) style_clipboard: Option<(Tool, DrawingStyle)>,
    /// The open properties dialog of a drawing.
    pub(super) properties: Option<DrawingProperties>,
    /// A chart whose drawings the user asked to remove all of; waiting for the confirmation.
    pub(super) confirm_remove: Option<u64>,
}

impl Default for DrawingTools {
    fn default() -> Self {
        Self {
            active: Tool::Cursor,
            zoom_armed: false,
            cursor: CursorMode::default(),
            prefs: DrawingPrefs::default(),
            saved_prefs: DrawingPrefs::default(),
            templates: None,
            hide_all: false,
            selected: None,
            style_clipboard: None,
            properties: None,
            confirm_remove: None,
        }
    }
}

impl DrawingTools {
    pub(in crate::workspace) fn active_tool(&self) -> Tool {
        self.active
    }

    pub(in crate::workspace) fn is_zoom_armed(&self) -> bool {
        self.zoom_armed
    }

    /// The cursor in force: the chosen one at rest, the cross while a drawing tool or Zoom In is
    /// armed (placing an anchor needs the lines and the tags).
    pub(in crate::workspace) fn cursor(&self) -> CursorMode {
        if self.active != Tool::Cursor || self.zoom_armed {
            CursorMode::Cross
        } else {
            self.cursor
        }
    }

    /// The cursor the user chose, whether or not a tool holds the pointer now.
    pub(in crate::workspace) fn chosen_cursor(&self) -> CursorMode {
        self.cursor
    }

    /// Puts the pointer on `mode`: any armed drawing tool and Zoom In are put down.
    pub(in crate::workspace) fn choose_cursor(&mut self, mode: CursorMode) {
        self.cursor = mode;
        if mode.persists() {
            self.prefs.cursor = mode;
        }
        if !mode.caps().edits_drawings {
            // Nothing stays picked while the pointer erases or marks.
            self.selected = None;
        }
        self.put_down();
    }

    /// Whether holding the button still shows the values of the bar.
    pub(in crate::workspace) fn values_tooltip(&self) -> bool {
        self.prefs.values_tooltip
    }

    pub(in crate::workspace) fn toggle_values_tooltip(&mut self) {
        self.prefs.values_tooltip = !self.prefs.values_tooltip;
    }

    /// Zoom In and a drawing tool are one choice: whichever was picked last holds the pointer, so
    /// an armed drawing tool puts Zoom In down.
    pub(in crate::workspace) fn give_pointer_to_tool(&mut self) {
        if self.active != Tool::Cursor {
            self.zoom_armed = false;
        }
    }

    /// Lets go of whatever holds the pointer: Zoom In or a drawing tool (they are one choice), back
    /// to the resting cursor. Esc, a right-click on the chart, a placed one-shot drawing and the
    /// cursor tile all end here. Reports whether something was held.
    pub(in crate::workspace) fn put_down(&mut self) -> bool {
        let held = self.zoom_armed || self.active != Tool::Cursor;
        self.zoom_armed = false;
        self.active = Tool::Cursor;
        held
    }

    /// Esc steps back one level: what holds the pointer first (Zoom In or the armed tool), then a
    /// cursor of the moment (Demonstration, the Eraser) back to the resting one, then the
    /// selection.
    pub(in crate::workspace) fn escape(&mut self) {
        if self.put_down() {
            return;
        }
        if !self.cursor.persists() {
            self.cursor = self.prefs.cursor;
        } else {
            self.selected = None;
        }
    }

    /// How drawings snap to the bars' prices.
    pub(in crate::workspace) fn magnet(&self) -> MagnetMode {
        self.prefs.magnet
    }

    /// Whether a drawing tool stays armed after a drawing is placed.
    pub(in crate::workspace) fn keeps_drawing(&self) -> bool {
        self.prefs.keep_drawing
    }

    pub(in crate::workspace) fn hides_all(&self) -> bool {
        self.hide_all
    }

    pub(in crate::workspace) fn toggle_hide_all(&mut self) {
        self.hide_all = !self.hide_all;
    }

    /// Shows every drawing again (a placed or pasted drawing would otherwise be invisible).
    pub(in crate::workspace) fn reveal_all(&mut self) {
        self.hide_all = false;
    }

    /// The selected drawings and their chart.
    pub(in crate::workspace) fn selection(&self) -> Option<&(u64, Vec<u64>)> {
        self.selected.as_ref()
    }

    /// Lets go of what refers to the drawings `chart_id` shows: its selection, its open properties
    /// dialog and a pending "remove all". Used when it shows another market, whose drawings these
    /// are not.
    pub(in crate::workspace) fn forget_chart(&mut self, chart_id: u64) {
        if self
            .selected
            .as_ref()
            .is_some_and(|(selected_chart, _)| *selected_chart == chart_id)
        {
            self.selected = None;
        }
        if self
            .properties
            .as_ref()
            .is_some_and(|props| props.chart_id == chart_id)
        {
            self.properties = None;
        }
        if self.confirm_remove == Some(chart_id) {
            self.confirm_remove = None;
        }
    }

    /// The selected drawings when they are on `chart_id`, else none.
    pub(in crate::workspace) fn selected_on(&self, chart_id: u64) -> Vec<u64> {
        self.selected
            .as_ref()
            .filter(|(selected_chart, _)| *selected_chart == chart_id)
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default()
    }

    pub(in crate::workspace) fn select(&mut self, chart_id: u64, drawings: Vec<u64>) {
        self.selected = Some((chart_id, drawings));
    }

    pub(in crate::workspace) fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Takes a drawing that was removed out of the selection.
    pub(in crate::workspace) fn deselect(&mut self, drawing_id: u64) {
        if let Some((_, ids)) = &mut self.selected {
            ids.retain(|selected| *selected != drawing_id);
        }
    }

    /// The style Paste style puts on a drawing.
    pub(in crate::workspace) fn style_clipboard(&self) -> Option<(Tool, DrawingStyle)> {
        self.style_clipboard
    }

    pub(in crate::workspace) fn copy_style(&mut self, tool: Tool, style: DrawingStyle) {
        self.style_clipboard = Some((tool, style));
    }

    /// A dialog about drawings (properties, or Remove all's question) is up and takes the keys.
    pub(in crate::workspace) fn has_dialog(&self) -> bool {
        self.properties.is_some() || self.confirm_remove.is_some()
    }

    /// The drawing the properties dialog edits, as it would look: the chart draws it in its place.
    pub(in crate::workspace) fn previewed_draft(&self) -> Option<(u64, Drawing)> {
        self.properties
            .as_ref()
            .map(|props| (props.chart_id, props.draft().clone()))
    }

    /// Asks before removing every drawing of `chart_id` (`confirm_drawings_removal`).
    pub(in crate::workspace) fn ask_to_remove_all(&mut self, chart_id: u64) {
        self.confirm_remove = Some(chart_id);
    }

    #[cfg(test)]
    pub(in crate::workspace) fn removal_asked(&self) -> Option<u64> {
        self.confirm_remove
    }
}
