//! Platform-specific words and availability for user-facing text, so UI code states what is true
//! on the machine it runs on instead of hard-coding macOS. Key combinations live in
//! `ui::shortcut`.

/// "in your Keychain" / "in secure storage".
pub const fn in_secret_store() -> &'static str {
    if cfg!(target_os = "macos") {
        "in your Keychain"
    } else {
        "in secure storage"
    }
}

/// "Saved securely in Keychain." in green beside a credentials form.
pub const fn saved_securely() -> &'static str {
    if cfg!(target_os = "macos") {
        "Saved securely in Keychain."
    } else {
        "Saved securely in secure storage."
    }
}

/// The hint under a credential field that holds nothing yet.
pub const fn credential_stored_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Stored in your Mac's Keychain, never in a file."
    } else {
        "Stored in this computer's secure storage, never in a file."
    }
}

/// The hint under a credential field that already holds a key.
pub const fn credential_saved_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "A key is saved in your Keychain. Enter a new one to replace it."
    } else {
        "A key is saved in secure storage. Enter a new one to replace it."
    }
}

/// The user's machine, for "your Mac's light or dark setting".
pub const fn computer() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mac"
    } else {
        "computer"
    }
}

/// The OS file browser's name.
pub const fn file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(windows) {
        "Explorer"
    } else {
        "File Manager"
    }
}

/// Whether the background alert agent, Login Items and the macOS notification centre exist.
pub const fn has_system_notifications() -> bool {
    cfg!(target_os = "macos")
}

/// Opens the OS file browser on `path`, selecting it where the OS can.
pub fn reveal_in_file_manager(path: &std::path::Path) {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    let _ = command.spawn();
}

/// Whether this build can sync with iCloud: the Swift CloudKit bridge is macOS-only and compiled
/// in only with the `icloud` feature. Settings hides the page otherwise.
#[allow(dead_code)] // the Settings page consumes this in a later stage
pub const fn has_icloud_sync() -> bool {
    cfg!(all(target_os = "macos", feature = "icloud"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_words_stay_mac_only() {
        if cfg!(target_os = "macos") {
            assert_eq!(in_secret_store(), "in your Keychain");
            assert_eq!(file_manager(), "Finder");
            assert!(has_system_notifications());
        } else {
            assert_eq!(in_secret_store(), "in secure storage");
            assert_ne!(file_manager(), "Finder");
            assert!(!has_system_notifications());
        }
    }
}
