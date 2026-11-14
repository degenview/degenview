//! The tab's drawing tools: the armed tool and the Zoom In mode, the strip's remembered options,
//! the selection, the copied style, and the dialogs about drawings. `drawing` and its children
//! act on it; the grid, the chart menu and the shortcuts go through the methods below.

use crate::model::{Drawing, DrawingStyle, MagnetMode, Tool};
use crate::store::drawing_template::{DrawingPrefs, DrawingTemplateStore};
use crate::ui::drawing::properties::DrawingProperties;

pub(in crate::workspace) struct DrawingTools {
    pub(super) active: Tool,
    /// The Zoom In mode of the tool rail. A mode of its own, never a drawing tool: a drag on a
    /// plot selects a rectangle to zoom to. Exclusive with `active`; not saved.
    pub(super) zoom_armed: bool,
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

    /// Zoom In and a drawing tool are one choice: whichever was picked last holds the pointer, so
    /// an armed drawing tool puts Zoom In down.
    pub(in crate::workspace) fn give_pointer_to_tool(&mut self) {
        if self.active != Tool::Cursor {
            self.zoom_armed = false;
        }
    }

    /// Back to the crosshair, after a drawing is placed or as asked.
    pub(in crate::workspace) fn disarm(&mut self) {
        self.active = Tool::Cursor;
    }

    /// Puts Zoom In down (a zoom was made with it).
    pub(in crate::workspace) fn disarm_zoom(&mut self) {
        self.zoom_armed = false;
    }

    /// Esc steps back one level: Zoom In first, then the armed tool, then the selection.
    pub(in crate::workspace) fn escape(&mut self) {
        if self.zoom_armed {
            self.zoom_armed = false;
        } else if self.active != Tool::Cursor {
            self.active = Tool::Cursor;
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
