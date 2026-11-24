//! Platform-specific words and availability for user-facing text, so UI code states what is true
//! on the machine it runs on instead of hard-coding macOS. Key combinations live in
//! `ui::shortcut`.

use crate::i18n::tr;

// Wording that names the machine: one whole message per platform, never the platform's word put into
// a sentence, so another language writes the sentence its own way and inflects the word.

/// "Secret not found in your Keychain": a webhook whose secret cannot be read.
pub fn webhook_secret_missing() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Secret not found in your Keychain")
    } else if cfg!(windows) {
        tr!("Secret not found in Windows Credential Manager")
    } else {
        tr!("Secret not found in secure storage")
    }
}

/// How a webhook's secret is kept and used, in the page that explains webhooks.
pub fn webhook_secret_is_kept() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "A secret is kept in your Keychain and added to the URL or a header where you place it."
        )
    } else if cfg!(windows) {
        tr!(
            "A secret is kept in Windows Credential Manager and added to the URL or a header where you place it."
        )
    } else {
        tr!(
            "A secret is kept in secure storage and added to the URL or a header where you place it."
        )
    }
}

/// The tip on a webhook that has a secret.
pub fn webhook_secret_in_use() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Uses a secret kept in your Keychain")
    } else if cfg!(windows) {
        tr!("Uses a secret kept in Windows Credential Manager")
    } else {
        tr!("Uses a secret kept in secure storage")
    }
}

/// The advice to move what looks like a credential out of a webhook's address or headers.
pub fn webhook_move_to_secret() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Save it as the secret so it is kept in your Keychain instead of in plain text.")
    } else if cfg!(windows) {
        tr!(
            "Save it as the secret so it is kept in Windows Credential Manager instead of in plain text."
        )
    } else {
        tr!("Save it as the secret so it is kept in secure storage instead of in plain text.")
    }
}

/// The hint under a webhook's secret field.
pub fn webhook_secret_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Kept in your Keychain and inserted wherever you place it, in the URL or a header.")
    } else if cfg!(windows) {
        tr!(
            "Kept in Windows Credential Manager and inserted wherever you place it, in the URL or a header."
        )
    } else {
        tr!("Kept in secure storage and inserted wherever you place it, in the URL or a header.")
    }
}

/// What the System theme does, under Settings ▸ Appearance.
pub fn system_theme_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("System follows your Mac's light or dark setting.")
    } else {
        tr!("System follows your computer's light or dark setting.")
    }
}

/// The menu item that opens the OS file browser on a file or folder.
pub fn show_in_file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Show in Finder")
    } else if cfg!(windows) {
        tr!("Show in Explorer")
    } else {
        tr!("Show in File Manager")
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
            assert_eq!(
                webhook_secret_in_use(),
                "Uses a secret kept in your Keychain"
            );
            assert_eq!(key_stays_stored(), "The key stays in this Mac's Keychain.");
            assert_eq!(show_in_file_manager(), "Show in Finder");
            assert!(has_system_notifications());
        } else if cfg!(windows) {
            assert_eq!(
                webhook_secret_in_use(),
                "Uses a secret kept in Windows Credential Manager"
            );
            assert_eq!(show_in_file_manager(), "Show in Explorer");
            assert!(!has_system_notifications());
        } else {
            assert_eq!(
                webhook_secret_in_use(),
                "Uses a secret kept in secure storage"
            );
            assert_eq!(show_in_file_manager(), "Show in File Manager");
            assert!(!has_system_notifications());
        }
    }

    #[test]
    fn every_wording_is_a_whole_message_in_the_pseudo_locale() {
        use crate::i18n::{Locale, with_locale};
        with_locale(Locale::PSEUDO, || {
            for text in [
                webhook_secret_missing(),
                webhook_secret_is_kept(),
                webhook_secret_in_use(),
                webhook_move_to_secret(),
                webhook_secret_hint(),
                system_theme_hint(),
                show_in_file_manager(),
                saved_securely(),
                credential_stored_hint(),
                key_stays_stored(),
                credential_saved_hint(),
            ] {
                assert!(
                    text.starts_with('[') && text.ends_with(']'),
                    "not put through tr!: {text}"
                );
            }
        });
    }
}
