//! Reading and writing theme files: the open and save panels, and where a file goes on a system
//! that has none. Every call blocks until the user answers, so run them on a worker thread.

use crate::service::file_dialog;
use crate::service::snapshot_export::downloads_path;
use std::path::PathBuf;

/// What a theme file is called: the theme's name, then what it is, then `.json`, so a folder of
/// them is recognisable and any editor opens them.
pub fn file_name(stem: &str) -> String {
    format!("{stem}.degenview-theme.json")
}

/// Asks for a theme file and reads it, up to `limit` bytes. `None` when the person cancels.
pub fn choose_and_read(limit: usize) -> Result<Option<String>, String> {
    let Some(path) = file_dialog::choose_json("Import a theme")? else {
        return Ok(None);
    };
    file_dialog::read_text_up_to(&path, limit).map(Some)
}

/// Asks where to save `text` as the theme `stem` and writes it: the save panel, or the downloads
/// folder under a free name where there is no panel. `None` when the person cancels.
pub fn save(stem: &str, text: &str) -> Result<Option<PathBuf>, String> {
    let name = file_name(stem);
    let path = if crate::platform::has_file_dialogs() {
        file_dialog::choose_save_path("Export theme", &name)?
    } else {
        Some(downloads_path(&name).map_err(|error| error.message())?)
    };
    let Some(path) = path else {
        return Ok(None);
    };
    file_dialog::write_text(&path, text)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_file_says_what_it_is() {
        assert_eq!(file_name("My Dracula"), "My Dracula.degenview-theme.json");
        assert!(file_name("x").ends_with(".json"));
    }
}
