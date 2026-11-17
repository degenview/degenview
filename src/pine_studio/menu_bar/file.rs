//! The File menu: new, open and save, and what can be done to the open script.

use super::entry::{Entry, Item, grouped};
use crate::i18n::tr;
use crate::model::pine::script::{ScriptId, ScriptType};
use crate::pine_studio::commands::Cmd;
use crate::pine_studio::library::script_actions::script_groups;
use crate::ui::icons::Icon;
use crate::ui::shortcut;

/// What the File menu reads of the open script.
pub(in crate::pine_studio) struct FileState {
    pub id: ScriptId,
    pub favorite: bool,
    pub script_type: ScriptType,
    /// The editor holds text that is not saved.
    pub dirty: bool,
}

pub(super) fn entries(state: &FileState) -> Vec<Entry> {
    let show_in = crate::platform::show_in_file_manager();
    let mut groups = vec![vec![
        Item::new(tr!("New Script…"), Cmd::NewScript)
            .icon(Icon::Plus)
            .shortcut(&shortcut::NEW_SCRIPT)
            .into(),
        Item::new(tr!("Open Script…"), Cmd::OpenScriptPicker)
            .icon(Icon::DocText)
            .shortcut(&shortcut::OPEN_SCRIPT_PICKER)
            .into(),
        Item::new(tr!("Save"), Cmd::Save)
            .icon(Icon::Download)
            .shortcut(&shortcut::SAVE)
            .enabled(state.dirty)
            .into(),
    ]];
    let mut per_script: Vec<Vec<Entry>> = script_groups(state.script_type, state.favorite, show_in)
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .map(|row| {
                    Item::new(row.label, row.action.command(&state.id))
                        .icon(row.icon)
                        .destructive(row.destructive)
                        .into()
                })
                .collect()
        })
        .collect();
    // Delete stays last; what adds scripts comes before it.
    let delete = per_script.pop();
    groups.extend(per_script);
    groups.push(vec![
        Item::new(tr!("Add Example Scripts"), Cmd::AddExamples)
            .icon(Icon::Layers)
            .into(),
    ]);
    groups.extend(delete);
    grouped(groups)
}
