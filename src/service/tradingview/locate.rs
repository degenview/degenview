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

/// The data folders that hold a cache here.
pub fn data_folders(environment: &Environment) -> Vec<PathBuf> {
    candidates(environment)
        .into_iter()
        .filter(|folder| cache_in(folder).is_some())
        .collect()
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
}
