//! The Theme menu: the editor's themes, the one worn checked, and a way to Settings, where they
//! are edited. The rows are what Settings' tiles list, in its order: Automatic, the built-in
//! themes, then the person's own.

use super::entry::{Entry, Item, grouped};
use crate::host::editor_theme::Choices;
use crate::i18n::tr;
use crate::model::editor_theme::{AUTOMATIC_NAME, BuiltInTheme, ThemeSelection};
use crate::pine_studio::commands::Cmd;
use crate::ui::icons::Icon;

pub(super) fn entries(choices: &Choices) -> Vec<Entry> {
    let pick = |selection: ThemeSelection, name: &str| -> Entry {
        Item::new(name, Cmd::SetEditorTheme(selection))
            .checked(selection == choices.selected)
            .into()
    };
    let built_in = std::iter::once(pick(ThemeSelection::Auto, AUTOMATIC_NAME))
        .chain(
            BuiltInTheme::ALL
                .into_iter()
                .map(|theme| pick(ThemeSelection::BuiltIn(theme), theme.name())),
        )
        .collect();
    let custom = choices
        .customs
        .iter()
        .map(|(id, name)| pick(ThemeSelection::Custom(*id), name))
        .collect();
    // The themes scroll when there are many; the way to Settings stays in reach under them.
    vec![
        Entry::Scroll(grouped(vec![built_in, custom])),
        Entry::Separator,
        Item::new(tr!("Configure Themes…"), Cmd::OpenThemeSettings)
            .icon(Icon::Sliders)
            .into(),
    ]
}
