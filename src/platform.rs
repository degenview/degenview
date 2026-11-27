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

/// The hint under the address of an OpenAI-compatible server.
pub fn ai_server_url_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "OpenRouter, Ollama, LM Studio and similar. Use https://, or http:// for a server on this Mac."
        )
    } else {
        tr!(
            "OpenRouter, Ollama, LM Studio and similar. Use https://, or http:// for a server on this computer."
        )
    }
}

/// Why a plain HTTP address elsewhere is refused.
pub fn ai_plain_http_refused() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Use https://, or http:// only for a server on this Mac.")
    } else {
        tr!("Use https://, or http:// only for a server on this computer.")
    }
}

/// Where the assistant's conversations live.
pub fn ai_history_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "Each script's conversation with the assistant is kept on this Mac so it is there after a restart. It is never synced."
        )
    } else {
        tr!(
            "Each script's conversation with the assistant is kept on this computer so it is there after a restart. It is never synced."
        )
    }
}

/// What happens to broker credentials.
pub fn broker_credentials_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "Credentials are stored in your system's secure storage and never leave this Mac. They are not synced, and no script, alert or other program can use them to trade."
        )
    } else {
        tr!(
            "Credentials are stored in your system's secure storage and never leave this computer. They are not synced, and no script, alert or other program can use them to trade."
        )
    }
}

/// What disconnecting a broker does.
pub fn broker_disconnect_message() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "This removes the key from this Mac and stops the connection. Orders already placed stay live at the broker. It does not revoke the key: to do that, delete it in the broker's own settings."
        )
    } else {
        tr!(
            "This removes the key from this computer and stops the connection. Orders already placed stay live at the broker. It does not revoke the key: to do that, delete it in the broker's own settings."
        )
    }
}

/// A broker was disconnected and its key deleted.
pub fn broker_key_removed() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Disconnected. The key was removed from this Mac.")
    } else {
        tr!("Disconnected. The key was removed from this computer.")
    }
}

/// A broker was disconnected but its key is still stored.
pub fn broker_key_not_removed(error: impl std::fmt::Display) -> String {
    if cfg!(target_os = "macos") {
        tr!(
            "Disconnected, but the key could NOT be removed from this Mac and will connect again when DegenView starts. Remove it in your keychain, or try again: {error}",
            error = error
        )
    } else if cfg!(windows) {
        tr!(
            "Disconnected, but the key could NOT be removed from this computer and will connect again when DegenView starts. Remove it in Windows Credential Manager, or try again: {error}",
            error = error
        )
    } else {
        tr!(
            "Disconnected, but the key could NOT be removed from this computer and will connect again when DegenView starts. Remove it in your keyring, or try again: {error}",
            error = error
        )
    }
}

/// The editor theme asks for a font this machine lacks.
pub fn font_not_installed(name: impl std::fmt::Display) -> String {
    if cfg!(target_os = "macos") {
        tr!(
            "“{name}” is not installed on this Mac, so the default is used until it is.",
            name = name
        )
    } else {
        tr!(
            "“{name}” is not installed on this computer, so the default is used until it is.",
            name = name
        )
    }
}

/// Which fonts the editor can use.
pub fn font_choice_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Built in: JetBrains Mono. Or any monospaced font installed on this Mac.")
    } else {
        tr!("Built in: JetBrains Mono. Or any monospaced font installed on this computer.")
    }
}

/// The line under Settings ▸ MCP Server.
pub fn mcp_page_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "Let AI assistants and automation on this Mac inspect DegenView, with permissions you control."
        )
    } else {
        tr!(
            "Let AI assistants and automation on this computer inspect DegenView, with permissions you control."
        )
    }
}

/// What switching the MCP server on does.
pub fn mcp_enable_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "Lets AI assistants and automation on this Mac inspect DegenView, within the permissions below. Off until you turn it on."
        )
    } else {
        tr!(
            "Lets AI assistants and automation on this computer inspect DegenView, within the permissions below. Off until you turn it on."
        )
    }
}

/// What the MCP page's activity list holds.
pub fn mcp_recent_activity_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "What connected clients asked DegenView to do, newest first. Kept on this Mac only, and holds ids and results, never what a request contained."
        )
    } else {
        tr!(
            "What connected clients asked DegenView to do, newest first. Kept on this computer only, and holds ids and results, never what a request contained."
        )
    }
}

/// What the whole MCP activity log holds.
pub fn mcp_activity_log_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "What connected clients asked DegenView to do. Kept on this Mac only; it holds ids and results, never what a request contained."
        )
    } else {
        tr!(
            "What connected clients asked DegenView to do. Kept on this computer only; it holds ids and results, never what a request contained."
        )
    }
}

/// No address was typed for the interface to listen on.
pub fn mcp_address_missing() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Enter the address of this Mac to listen on")
    } else {
        tr!("Enter the address of this computer to listen on")
    }
}

/// The typed address is the machine's own loopback.
pub fn mcp_address_is_loopback() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("That is this Mac's own address. Turn the switch off to listen here only")
    } else {
        tr!("That is this computer's own address. Turn the switch off to listen here only")
    }
}

/// What the remote ceiling limits.
pub fn mcp_remote_ceiling_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("The most a credential with remote access can do, even on this Mac.")
    } else {
        tr!("The most a credential with remote access can do, even on this computer.")
    }
}

/// The tip on the button that finds the local address.
pub fn mcp_detect_address_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Fill in this Mac's address on the local network")
    } else {
        tr!("Fill in this computer's address on the local network")
    }
}

/// A credential without remote access.
pub fn mcp_local_only() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Off: this credential works from this Mac only.")
    } else {
        tr!("Off: this credential works from this computer only.")
    }
}

/// The key that seals MCP credentials could not be made.
pub fn mcp_seal_not_created(error: impl std::fmt::Display) -> String {
    if cfg!(target_os = "macos") {
        tr!(
            "MCP credentials cannot be protected against edits to the database on this Mac ({error})",
            error = error
        )
    } else {
        tr!(
            "MCP credentials cannot be protected against edits to the database on this computer ({error})",
            error = error
        )
    }
}

/// The saved MCP credentials were edited outside the app.
pub fn mcp_seal_mismatch() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "The saved MCP credentials do not match the seal kept in the Keychain, so they were changed outside DegenView. The server stays off."
        )
    } else {
        tr!(
            "The saved MCP credentials do not match the seal kept in secure storage, so they were changed outside DegenView. The server stays off."
        )
    }
}

/// The key that seals MCP credentials is gone.
pub fn mcp_seal_key_missing() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "The Keychain key that protects the saved MCP credentials is missing, so they cannot be trusted. The server stays off."
        )
    } else {
        tr!(
            "The key in secure storage that protects the saved MCP credentials is missing, so they cannot be trusted. The server stays off."
        )
    }
}

/// The key that seals MCP credentials cannot be read.
pub fn mcp_seal_key_unreadable() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "The Keychain key that protects the saved MCP credentials could not be read, so the server stays off. Restart DegenView to try again."
        )
    } else {
        tr!(
            "The key in secure storage that protects the saved MCP credentials could not be read, so the server stays off. Restart DegenView to try again."
        )
    }
}

/// Resetting MCP access could not replace the key.
pub fn mcp_seal_key_not_replaced(
    error: impl std::fmt::Display,
    item: impl std::fmt::Display,
) -> String {
    if cfg!(target_os = "macos") {
        tr!(
            "Could not replace the Keychain key, so nothing was changed ({error}). Delete the “{item}” item in Keychain Access, then restart DegenView.",
            error = error,
            item = item
        )
    } else if cfg!(windows) {
        tr!(
            "Could not replace the key in Windows Credential Manager, so nothing was changed ({error}). Delete the “{item}” item there, then restart DegenView.",
            error = error,
            item = item
        )
    } else {
        tr!(
            "Could not replace the key in secure storage, so nothing was changed ({error}). Delete the “{item}” item in your keyring, then restart DegenView.",
            error = error,
            item = item
        )
    }
}

/// The key was replaced but the old MCP state is still there.
pub fn mcp_state_not_cleared() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!(
            "The Keychain key was replaced but the saved MCP state could not be cleared. Choose Reset MCP Access again."
        )
    } else {
        tr!(
            "The key in secure storage was replaced but the saved MCP state could not be cleared. Choose Reset MCP Access again."
        )
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

    /// Every wording above that can name the machine or its secret store.
    fn machine_wordings() -> Vec<String> {
        vec![
            ai_server_url_hint().to_string(),
            ai_plain_http_refused().to_string(),
            ai_history_hint().to_string(),
            broker_credentials_hint().to_string(),
            broker_disconnect_message().to_string(),
            broker_key_removed().to_string(),
            broker_key_not_removed("x").to_string(),
            font_not_installed("x").to_string(),
            font_choice_hint().to_string(),
            mcp_page_hint().to_string(),
            mcp_enable_hint().to_string(),
            mcp_recent_activity_hint().to_string(),
            mcp_activity_log_hint().to_string(),
            mcp_address_missing().to_string(),
            mcp_address_is_loopback().to_string(),
            mcp_remote_ceiling_hint().to_string(),
            mcp_detect_address_hint().to_string(),
            mcp_local_only().to_string(),
            mcp_seal_not_created("x").to_string(),
            mcp_seal_mismatch().to_string(),
            mcp_seal_key_missing().to_string(),
            mcp_seal_key_unreadable().to_string(),
            mcp_seal_key_not_replaced("x", "x").to_string(),
            mcp_state_not_cleared().to_string(),
        ]
    }

    #[test]
    fn only_a_mac_hears_of_a_mac_or_a_keychain() {
        for text in machine_wordings() {
            let names_one = text.contains("Mac") || text.to_lowercase().contains("keychain");
            assert_eq!(names_one, cfg!(target_os = "macos"), "{text}");
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
            ]
            .map(str::to_owned)
            .into_iter()
            .chain(machine_wordings())
            {
                assert!(
                    text.starts_with('[') && text.ends_with(']'),
                    "not put through tr!: {text}"
                );
            }
        });
    }
}
