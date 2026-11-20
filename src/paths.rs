//! Where the app keeps its files: the user-data folder (the database, scripts, backups, logs) and
//! the cache folder for regenerable network data. Every layer asks here, so no module needs the
//! store to find a path.

use std::path::PathBuf;

const APP_DIR: &str = "es.nico.degenview";

/// The app's data directory: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows,
/// `$XDG_DATA_HOME` (or `~/.local/share`) elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(env("HOME")?).join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(env("APPDATA").or_else(|| env("LOCALAPPDATA"))?)
    } else {
        match env("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env("HOME")?).join(".local/share"),
        }
    };
    Some(base.join(APP_DIR))
}

/// Overrides `cache_dir` (dev runs and manual verification against a scratch directory).
pub const CACHE_DIR_ENV: &str = "DEGENVIEW_CACHE_DIR";

/// Where regenerable network data lives (icons, lookups, FX rates) — separate from `data_dir`
/// because the OS may reclaim it and the user may clear it: `~/Library/Caches` on macOS,
/// `%LOCALAPPDATA%` on Windows, `$XDG_CACHE_HOME` (or `~/.cache`) elsewhere.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_from(
        &|name| std::env::var_os(name),
        cfg!(target_os = "macos"),
        cfg!(windows),
    )
}

fn cache_dir_from(
    env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    macos: bool,
    windows: bool,
) -> Option<PathBuf> {
    let env = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(dir) = env(CACHE_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    Some(if macos {
        PathBuf::from(env("HOME")?)
            .join("Library/Caches")
            .join(APP_DIR)
    } else if windows {
        PathBuf::from(env("LOCALAPPDATA").or_else(|| env("APPDATA"))?)
            .join(APP_DIR)
            .join("Cache")
    } else {
        match env("XDG_CACHE_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env("HOME")?).join(".cache"),
        }
        .join(APP_DIR)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
        }
    }

    #[test]
    fn the_cache_dir_follows_each_platforms_convention() {
        let mac = cache_dir_from(&env_of(&[("HOME", "/Users/a")]), true, false);
        assert_eq!(
            mac,
            Some(PathBuf::from("/Users/a/Library/Caches/es.nico.degenview"))
        );
        let windows = cache_dir_from(
            &env_of(&[("LOCALAPPDATA", "C:\\L"), ("APPDATA", "C:\\R")]),
            false,
            true,
        );
        assert_eq!(
            windows,
            Some(
                PathBuf::from("C:\\L")
                    .join("es.nico.degenview")
                    .join("Cache")
            )
        );
        let xdg = cache_dir_from(
            &env_of(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/home/a")]),
            false,
            false,
        );
        assert_eq!(xdg, Some(PathBuf::from("/x/es.nico.degenview")));
        let home = cache_dir_from(&env_of(&[("HOME", "/home/a")]), false, false);
        assert_eq!(
            home,
            Some(PathBuf::from("/home/a/.cache/es.nico.degenview"))
        );
        assert_eq!(cache_dir_from(&env_of(&[]), false, false), None);
    }

    #[test]
    fn the_cache_dir_can_be_overridden_and_an_empty_value_is_ignored() {
        let over = cache_dir_from(
            &env_of(&[(CACHE_DIR_ENV, "/scratch"), ("HOME", "/Users/a")]),
            true,
            false,
        );
        assert_eq!(over, Some(PathBuf::from("/scratch")));
        let empty = cache_dir_from(
            &env_of(&[(CACHE_DIR_ENV, ""), ("HOME", "/h")]),
            false,
            false,
        );
        assert_eq!(empty, Some(PathBuf::from("/h/.cache/es.nico.degenview")));
    }
}
