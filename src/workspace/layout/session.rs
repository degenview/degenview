//! A tab's relation to the layout library: the layout it has open, what was last saved, the
//! prompts and alerts of saving and opening, and the autosave debounce. `layout` and `autosave`
//! act on it; the rest of the workspace asks it the open layout's name and notes drawing edits.

use super::autosave::AutosaveState;
use crate::model::{Drawing, LayoutSnapshot};
use crate::ui::workspace::layout_prompts::NamePrompt;
use std::collections::HashMap;

#[derive(Default)]
pub(in crate::workspace) struct LayoutSession {
    /// The layout's fingerprint as last saved or opened; `None` until the tab has one.
    pub(super) baseline: Option<LayoutSnapshot>,
    /// Bumped by every change to a chart's drawings; see `ChartWorkspace::drawings_dirty`.
    pub(super) drawings_version: u64,
    pub(super) saved_drawings_version: u64,
    /// Each chart's drawings as of the last save or open: the common ancestor when another Mac's
    /// edit to the open layout arrives (`adopt_remote_layout`).
    pub(super) saved_drawings: HashMap<u64, Vec<Drawing>>,
    pub(super) active_view_id: Option<u64>,
    pub(super) active_view_name: Option<String>,
    /// The name prompt that is open, if any, and the text in it.
    pub(super) prompt: Option<NamePrompt>,
    pub(super) prompt_name: String,
    pub(super) prompt_focus: bool,
    /// A layout waiting for the "save changes?" answer before it opens.
    pub(super) pending_open_id: Option<u64>,
    /// Why the last layout operation failed, until the alert is dismissed.
    pub(super) error: Option<String>,
    pub(super) autosave: AutosaveState,
}

impl LayoutSession {
    /// The saved layout this tab has open, if any.
    pub(in crate::workspace) fn active_view_id(&self) -> Option<u64> {
        self.active_view_id
    }

    /// The open layout's name; `None` on a tab with no saved layout ("Unnamed").
    pub(in crate::workspace) fn active_view_name(&self) -> Option<&str> {
        self.active_view_name.as_deref()
    }

    /// The tab has saved layout `id`, called `name`, open.
    pub(in crate::workspace) fn set_active_view(&mut self, id: u64, name: String) {
        self.active_view_id = Some(id);
        self.active_view_name = Some(name);
    }

    /// A chart's drawings changed: an Autosave layout now has something to write.
    pub(in crate::workspace) fn drawings_edited(&mut self) {
        self.drawings_version += 1;
    }

    /// A name prompt is up, which takes the keyboard.
    pub(in crate::workspace) fn is_prompting(&self) -> bool {
        self.prompt.is_some()
    }

    /// The last autosave did not reach disk.
    pub(in crate::workspace) fn autosave_failed(&self) -> bool {
        self.autosave.failed
    }

    #[cfg(test)]
    pub(in crate::workspace) fn drawings_version(&self) -> u64 {
        self.drawings_version
    }
}
