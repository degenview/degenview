//! Which build this is, for the About page, crash reports and the updater. Constants only, so any
//! layer can name the build without depending on the code that updates it.

/// The app's version, as `Cargo.toml` states it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit this build came from (`-dirty` when it had uncommitted changes), set by `build.rs`.
/// Shown in Settings ▸ About and crash reports for bug reports; nothing else reads it.
pub const BUILD_ID: &str = env!("DEGENVIEW_BUILD_ID");

#[cfg(test)]
mod tests {
    #[test]
    fn the_build_id_is_one_word() {
        assert!(!super::BUILD_ID.is_empty());
        assert!(!super::BUILD_ID.contains(char::is_whitespace));
    }
}
