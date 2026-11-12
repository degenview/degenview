//! Whether the app runs inside a Flatpak sandbox, where it cannot replace itself and the
//! packaging (not this program) owns how it is installed.

use std::path::Path;

/// True under Flatpak. Flatpak sets `FLATPAK_ID` for the app and mounts `/.flatpak-info` in every
/// sandbox; either is enough.
pub fn is_flatpak() -> bool {
    cfg!(target_os = "linux")
        && from_signals(
            std::env::var_os("FLATPAK_ID").as_deref(),
            Path::new("/.flatpak-info"),
        )
}

/// What an update notice says in a Flatpak. Flatpak records nothing about the remote an app was
/// installed from (`/.flatpak-info` has no such key), so the app cannot tell a copy installed from
/// the hosted repo, which `flatpak update` replaces, from one installed from a downloaded bundle,
/// which it cannot. The notice is true for both. The repo can trail GitHub by hours.
pub const FLATPAK_UPDATE_NOTE: &str = "Run flatpak update if you installed from the DegenView repository, or download the new .flatpak from the release page.";

fn from_signals(flatpak_id: Option<&std::ffi::OsStr>, info_file: &Path) -> bool {
    flatpak_id.is_some_and(|id| !id.is_empty()) || info_file.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn the_app_id_variable_means_flatpak() {
        let missing = Path::new("/definitely/not/here");
        assert!(from_signals(Some(OsStr::new("es.nico.degenview")), missing));
    }

    #[test]
    fn an_empty_or_missing_variable_does_not() {
        let missing = Path::new("/definitely/not/here");
        assert!(!from_signals(None, missing));
        assert!(!from_signals(Some(OsStr::new("")), missing));
    }

    #[test]
    fn the_flatpak_note_names_both_ways_to_update() {
        assert!(FLATPAK_UPDATE_NOTE.contains("flatpak update"));
        assert!(FLATPAK_UPDATE_NOTE.contains("release page"));
    }

    #[test]
    fn the_info_file_means_flatpak_without_the_variable() {
        assert!(from_signals(None, Path::new(env!("CARGO_MANIFEST_DIR"))));
    }
}
