//! Where the TradingView desktop app keeps its data on each platform, and the cache inside it.

use super::cache;
use std::path::{Path, PathBuf};

/// The folders of the environment the search starts from.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub home: Option<PathBuf>,
    /// Windows' `%APPDATA%`.
    pub roaming: Option<PathBuf>,
    /// Windows' `%LOCALAPPDATA%`.
    pub local: Option<PathBuf>,
}

impl Environment {
    pub fn current() -> Self {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self {
            home: var("HOME"),
            roaming: var("APPDATA"),
            local: var("LOCALAPPDATA"),
        }
    }
}

/// The data folders there could be, whether or not they exist.
fn candidates(environment: &Environment) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(home) = &environment.home {
            found.push(home.join("Library/Application Support/TradingView"));
            // The App Store build is sandboxed.
            found.push(home.join(
                "Library/Containers/com.tradingview.tradingviewapp.desktop/Data/Library/\
                 Application Support/TradingView",
            ));
        }
    } else if cfg!(windows) {
        // The packaged (MSIX) app, whose folder ends in a publisher hash.
        if let Some(local) = &environment.local
            && let Ok(packages) = std::fs::read_dir(local.join("Packages"))
        {
            found.extend(
                packages
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("TradingView.Desktop_")
                    })
                    .map(|entry| entry.path().join("LocalCache/Roaming/TradingView")),
            );
        }
        if let Some(roaming) = &environment.roaming {
            found.push(roaming.join("TradingView"));
        }
    } else if let Some(home) = &environment.home {
        found.push(home.join("snap/tradingview/current/.config/TradingView"));
        found.push(home.join(".config/TradingView"));
        found.push(home.join(".var/app/com.tradingview.tradingview/config/TradingView"));
    }
    found
}

/// The cache inside `folder`: the folder itself, or where the app puts it.
pub fn cache_in(folder: &Path) -> Option<PathBuf> {
    ["Cache/Cache_Data", "Cache_Data", "Cache", ""]
        .iter()
        .map(|inside| {
            if inside.is_empty() {
                folder.to_owned()
            } else {
                folder.join(inside)
            }
        })
        .find(|dir| cache::is_cache(dir))
}

/// Whether the system asks the person before an app may open `folder`: macOS guards the
/// container of a sandboxed app.
fn guarded(folder: &Path) -> bool {
    cfg!(target_os = "macos") && folder.to_string_lossy().contains("/Library/Containers/")
}

fn with_cache(folders: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    folders
        .filter(|folder| cache_in(folder).is_some())
        .collect()
}

/// The data folders that hold a cache here.
pub fn data_folders(environment: &Environment) -> Vec<PathBuf> {
    with_cache(candidates(environment).into_iter())
}

/// Those of [`data_folders`] that can be looked into unasked, without the system putting a
/// question to the person. The others are not touched.
pub fn unguarded_data_folders(environment: &Environment) -> Vec<PathBuf> {
    with_cache(
        candidates(environment)
            .into_iter()
            .filter(|folder| !guarded(folder)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::tradingview::cache::testing::{self, Fake, Scratch};

    #[test]
    fn the_cache_is_found_from_the_data_folder_or_itself() {
        let temp = Scratch::new();
        let inner = temp.path().join("Cache/Cache_Data");
        testing::simple(&inner, &[Fake::ok("https://x.com/", "")]);
        assert_eq!(cache_in(temp.path()), Some(inner.clone()));
        assert_eq!(cache_in(&inner), Some(inner));
        assert_eq!(cache_in(&temp.path().join("nothing")), None);
    }

    #[test]
    fn an_empty_environment_has_no_folders() {
        assert!(data_folders(&Environment::default()).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_sandboxed_apps_container_is_not_looked_into_unasked() {
        let temp = Scratch::new();
        let environment = Environment {
            home: Some(temp.path().to_owned()),
            ..Environment::default()
        };
        let folders = candidates(&environment);
        for folder in &folders {
            testing::simple(
                &folder.join("Cache/Cache_Data"),
                &[Fake::ok("https://x.com/", "")],
            );
        }
        assert_eq!(data_folders(&environment), folders);
        assert_eq!(unguarded_data_folders(&environment), folders[..1]);
    }
}
