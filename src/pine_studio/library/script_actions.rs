//! What can be done to a script: the rows of the script picker's right-click menu and of the File
//! menu over the editor, in one list so the two cannot drift.

use crate::model::pine::script::ScriptType;
use crate::pine_studio::commands::Cmd;
use crate::ui::icons::Icon;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::pine_studio) enum ScriptAction {
    Favorite,
    Rename,
    Duplicate,
    ShowInFileManager,
    Export,
    AddToChart,
    CreateAlert,
    Delete,
}

impl ScriptAction {
    /// The command that does it to the script `id`.
    pub fn command(self, id: &str) -> Cmd {
        let id = id.to_owned();
        match self {
            Self::Favorite => Cmd::ToggleFavorite(id),
            Self::Rename => Cmd::BeginRename(id),
            Self::Duplicate => Cmd::Duplicate(id),
            Self::ShowInFileManager => Cmd::RevealFile(id),
            Self::Export => Cmd::Export(id),
            Self::AddToChart => Cmd::AddToChart(id),
            Self::CreateAlert => Cmd::CreateAlert(id),
            Self::Delete => Cmd::AskDelete(id),
        }
    }

    /// Changes the list itself rather than taking the person elsewhere, so the picker stays up.
    pub fn stays_in_list(self) -> bool {
        self == Self::Favorite
    }
}

/// One row: what it does, its icon and label, and whether it is drawn as destructive.
pub(in crate::pine_studio) struct ScriptRow<'a> {
    pub action: ScriptAction,
    pub icon: Icon,
    pub label: &'a str,
    pub destructive: bool,
}

/// The rows for a script of `script_type`, in groups the menus separate. A library has no chart to
/// go on, so no Add to Chart and no alert. `show_in` is the label of the row that shows the file
/// in the platform's file manager.
pub(in crate::pine_studio) fn script_groups(
    script_type: ScriptType,
    favorite: bool,
    show_in: &str,
) -> Vec<Vec<ScriptRow<'_>>> {
    let row = |action, icon, label, destructive| ScriptRow {
        action,
        icon,
        label,
        destructive,
    };
    let mut groups = vec![
        vec![
            row(
                ScriptAction::Favorite,
                if favorite { Icon::Star } else { Icon::StarFill },
                if favorite {
                    "Remove Favorite"
                } else {
                    "Favorite"
                },
                false,
            ),
            row(ScriptAction::Rename, Icon::Pencil, "Rename…", false),
            row(ScriptAction::Duplicate, Icon::CopyDoc, "Duplicate", false),
        ],
        vec![
            row(ScriptAction::ShowInFileManager, Icon::Books, show_in, false),
            row(ScriptAction::Export, Icon::ShareUp, "Export…", false),
        ],
    ];
    if script_type != ScriptType::Library {
        groups.push(vec![
            row(
                ScriptAction::AddToChart,
                Icon::ChartLine,
                "Add to Chart",
                false,
            ),
            row(ScriptAction::CreateAlert, Icon::Bell, "Create Alert", false),
        ]);
    }
    groups.push(vec![row(
        ScriptAction::Delete,
        Icon::Trash,
        "Delete…",
        true,
    )]);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(script_type: ScriptType, favorite: bool) -> Vec<Vec<String>> {
        script_groups(script_type, favorite, "Show in Finder")
            .into_iter()
            .map(|group| group.into_iter().map(|row| row.label.to_owned()).collect())
            .collect()
    }

    #[test]
    fn a_script_can_be_favorited_renamed_shown_exported_put_on_a_chart_and_deleted() {
        assert_eq!(
            labels(ScriptType::Indicator, false),
            [
                vec!["Favorite", "Rename…", "Duplicate"],
                vec!["Show in Finder", "Export…"],
                vec!["Add to Chart", "Create Alert"],
                vec!["Delete…"],
            ]
        );
    }

    #[test]
    fn a_favorite_offers_to_be_removed_and_a_library_has_no_chart_rows() {
        let favorite = labels(ScriptType::Library, true);
        assert_eq!(favorite[0][0], "Remove Favorite");
        assert_eq!(favorite.len(), 3);
        assert!(
            favorite
                .iter()
                .flatten()
                .all(|label| !label.contains("Chart"))
        );
    }

    #[test]
    fn each_action_is_the_command_for_that_script_and_only_the_star_keeps_the_list_up() {
        assert!(matches!(
            ScriptAction::Rename.command("a"),
            Cmd::BeginRename(id) if id == "a"
        ));
        assert!(matches!(
            ScriptAction::Delete.command("a"),
            Cmd::AskDelete(_)
        ));
        assert!(matches!(
            ScriptAction::ShowInFileManager.command("a"),
            Cmd::RevealFile(_)
        ));
        for action in [
            ScriptAction::Rename,
            ScriptAction::Duplicate,
            ScriptAction::Export,
            ScriptAction::AddToChart,
            ScriptAction::CreateAlert,
            ScriptAction::Delete,
        ] {
            assert!(!action.stays_in_list());
        }
        assert!(ScriptAction::Favorite.stays_in_list());
    }
}
