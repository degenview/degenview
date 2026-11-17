//! Platform-specific words and availability for user-facing text, so UI code states what is true
//! on the machine it runs on instead of hard-coding macOS. Key combinations live in
//! `ui::shortcut`.

use crate::i18n::{tr, tr_c};

/// "in your Keychain" / "in secure storage". One whole message per platform: another language puts
/// the platform's word where it wants it.
pub fn in_secret_store() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("in your Keychain")
    } else if cfg!(windows) {
        tr!("in Windows Credential Manager")
    } else {
        tr!("in secure storage")
    }
}

/// "Saved securely in Keychain." in green beside a credentials form.
pub fn saved_securely() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Saved securely in Keychain.")
    } else if cfg!(windows) {
        tr!("Saved securely in Windows Credential Manager.")
    } else {
        tr!("Saved securely in secure storage.")
    }
}

/// The hint under a credential field that holds nothing yet.
pub fn credential_stored_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Stored in your Mac's Keychain, never in a file.")
    } else if cfg!(windows) {
        tr!("Stored in Windows Credential Manager, never in a file.")
    } else {
        tr!("Stored in this computer's secure storage, never in a file.")
    }
}

/// Where a saved key stays, as a sentence: "The key stays in this Mac's Keychain."
pub fn key_stays_stored() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("The key stays in this Mac's Keychain.")
    } else if cfg!(windows) {
        tr!("The key stays in Windows Credential Manager on this computer.")
    } else {
        tr!("The key stays in this computer's secure storage.")
    }
}

/// The hint under a credential field that already holds a key.
pub fn credential_saved_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("A key is saved in your Keychain. Enter a new one to replace it.")
    } else if cfg!(windows) {
        tr!("A key is saved in Windows Credential Manager. Enter a new one to replace it.")
    } else {
        tr!("A key is saved in secure storage. Enter a new one to replace it.")
    }
}

/// The user's machine, for "your Mac's light or dark setting". A word put into a sentence: a
/// language that inflects it needs the whole sentence per platform instead (see
/// `docs/localization.md`).
pub fn computer() -> &'static str {
    if cfg!(target_os = "macos") {
        tr_c!("the user's own machine", "Mac")
    } else {
        tr_c!("the user's own machine", "computer")
    }
}

/// The OS file browser's name, for "Show in Finder". Same caveat as `computer`.
pub fn file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        tr_c!("the OS file browser", "Finder")
    } else if cfg!(windows) {
        tr_c!("the OS file browser", "Explorer")
    } else {
        tr_c!("the OS file browser", "File Manager")
    }
}

/// Whether open and save panels exist (`service::system::file_dialog`). Where they do not, exports go to
/// the downloads folder under a free name.
pub const fn has_file_dialogs() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
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
            assert_eq!(key_stays_stored(), "The key stays in this Mac's Keychain.");
            assert_eq!(file_manager(), "Finder");
            assert!(has_system_notifications());
        } else if cfg!(windows) {
            assert_eq!(in_secret_store(), "in Windows Credential Manager");
            assert_ne!(file_manager(), "Finder");
            assert!(!has_system_notifications());
        } else {
            assert_eq!(in_secret_store(), "in secure storage");
            assert_ne!(file_manager(), "Finder");
            assert!(!has_system_notifications());
        }
    }

    #[test]
    fn every_wording_is_a_whole_message_in_the_pseudo_locale() {
        use crate::i18n::{Locale, with_locale};
        with_locale(Locale::PSEUDO, || {
            for text in [
                in_secret_store(),
                saved_securely(),
                credential_stored_hint(),
                key_stays_stored(),
                credential_saved_hint(),
                computer(),
                file_manager(),
            ] {
                assert!(
                    text.starts_with('[') && text.ends_with(']'),
                    "not put through tr!: {text}"
                );
            }
        });
    }
}
