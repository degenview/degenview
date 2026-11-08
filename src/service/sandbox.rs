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

/// True when this Flatpak was installed from a remote (the hosted repo, or Flathub), so
/// `flatpak update` can replace it and the in-app check has nothing to add. A bundle installed from
/// the release page has no remote of its own and keeps the check, which names the release page.
pub fn updated_by_flatpak() -> bool {
    is_flatpak()
        && std::fs::read_to_string("/.flatpak-info").is_ok_and(|info| installed_from_remote(&info))
}

/// The remote a Flatpak was installed from, as `/.flatpak-info` records it (`[Instance]`, `origin`).
fn origin(info: &str) -> Option<&str> {
    let mut in_instance = false;
    for line in info.lines().map(str::trim) {
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_instance = section == "Instance";
        } else if let Some(value) = line.strip_prefix("origin=").filter(|_| in_instance) {
            return Some(value.trim()).filter(|value| !value.is_empty());
        }
    }
    None
}

/// A bundle installed from a file gets a remote named `<app id>-origin` (a place to find its
/// runtime, not somewhere to update from); any other name is a repo the user added.
fn installed_from_remote(info: &str) -> bool {
    origin(info).is_some_and(|name| !name.ends_with("-origin"))
}

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

    const FROM_REPO: &str = "[Application]\nname=es.nico.degenview\n\n[Instance]\ninstance-id=1\nbranch=stable\norigin=degenview\nflatpak-version=1.16.1\n";
    const FROM_BUNDLE: &str = "[Application]\nname=es.nico.degenview\n\n[Instance]\nbranch=beta\norigin=es.nico.degenview-origin\n";

    #[test]
    fn a_repo_the_user_added_updates_the_app() {
        assert_eq!(origin(FROM_REPO), Some("degenview"));
        assert!(installed_from_remote(FROM_REPO));
        assert!(installed_from_remote("[Instance]\norigin=flathub\n"));
    }

    #[test]
    fn a_bundle_from_a_file_does_not() {
        assert_eq!(origin(FROM_BUNDLE), Some("es.nico.degenview-origin"));
        assert!(!installed_from_remote(FROM_BUNDLE));
    }

    #[test]
    fn a_missing_empty_or_misplaced_origin_keeps_the_check() {
        assert!(!installed_from_remote(""));
        assert!(!installed_from_remote("[Instance]\nbranch=stable\n"));
        assert!(!installed_from_remote("[Instance]\norigin=\n"));
        // `origin` in another section says nothing about the install.
        assert!(!installed_from_remote("[Application]\norigin=degenview\n"));
        assert!(!installed_from_remote("origin=degenview\n"));
    }

    #[test]
    fn the_info_file_means_flatpak_without_the_variable() {
        assert!(from_signals(None, Path::new(env!("CARGO_MANIFEST_DIR"))));
    }
}
