//! Secret storage on Windows, behind the same `platform::{read, exists, write, delete}` calls the
//! macOS Keychain and the Linux Secret Service answer: one Credential Manager "generic" credential
//! per item (`CredWriteW` / `CredReadW` / `CredDeleteW`).
//!
//! Credential Manager encrypts the blob with the signed-in user's DPAPI master key. Windows
//! normally lets only that user, on this computer, decrypt it (Microsoft documents an exception: a
//! roaming profile lets the same user decrypt on another computer). It is stored with
//! `CRED_PERSIST_LOCAL_MACHINE`: it survives a restart and a log-off, and Credential Manager does
//! not roam it with a domain profile. The user can see and remove the entries in Control Panel > Credential Manager >
//! Windows Credentials, under the target name `<service>/<account>`.
//!
//! What this does not give: a separate unlock step or a per-application ACL. Any process running
//! as the same user can read the items, as on the macOS Keychain without an access group. The
//! guarantee is that a secret is never in the SQLite file, a backup of it, or iCloud Sync, and is
//! not readable from another account or from the disk of a powered-off machine.
//!
//! The blob is the secret's UTF-8 bytes, without a terminator. (UTF-16, which the Credential
//! Manager window would display, halves the room: a webhook secret may be 2048 characters, and a
//! credential holds at most [`MAX_BLOB_BYTES`] bytes.) The window therefore shows the blob as
//! unreadable text; the entry is for seeing that it exists and removing it. A secret that would
//! not fit (over 2560 bytes, which only a long non-ASCII webhook secret reaches) is refused with an
//! error the Settings page shows, never truncated.
//!
//! Credential Manager never prompts, so there is no worker thread and no timeout. Calls go to a
//! local service, though, and the webhook list asks `exists` once per row every frame, so `exists`
//! is answered from a cache that this module's own writes keep current and that expires after
//! [`EXISTS_TTL`] (an item removed in the Credential Manager window shows up as gone then).
//!
//! Buffers that held a secret are overwritten before they are freed. That is best effort: the
//! caller's own `String` is not ours to wipe. Under `DEGENVIEW_NO_KEYCHAIN` the callers never reach
//! this module (`keychain_webhook_secrets::is_disabled`).
//!
//! The Keychain's access groups do not exist here, so `Scope::shared_group` is ignored: an item is
//! identified by its service and account alone.
//!
//! Only the Win32 calls are Windows-specific. The name, the encoding, the size limit and the
//! cache are plain Rust and are tested on every platform.

// Off Windows nothing but the tests reaches the helpers.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::service::webhook_delivery::SecretStoreError;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The most bytes Credential Manager keeps in one credential (`CRED_MAX_CREDENTIAL_BLOB_SIZE`).
const MAX_BLOB_BYTES: usize = 2560;
/// The longest target name a generic credential may have (`CRED_MAX_GENERIC_TARGET_NAME_LENGTH`),
/// in UTF-16 units.
const MAX_TARGET_UNITS: usize = 32767;
/// Shown as the credential's user name in the Credential Manager window.
const USER_NAME: &str = "degenview";
/// How long `exists` believes its last answer.
const EXISTS_TTL: Duration = Duration::from_secs(30);

// MARK: Pure helpers

/// The credential's target name, or `None` when it cannot be one (empty parts, a NUL, too long).
fn target_name(service: &str, account: &str) -> Option<String> {
    if service.is_empty() || account.is_empty() {
        return None;
    }
    let name = format!("{service}/{account}");
    if name.contains('\0') || name.encode_utf16().count() > MAX_TARGET_UNITS {
        return None;
    }
    Some(name)
}

/// `text` as a NUL-terminated UTF-16 string for the `...W` calls.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Bytes that are overwritten with zeros when dropped.
pub(crate) struct Wiped(pub(crate) Vec<u8>);

impl std::fmt::Debug for Wiped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wiped({} bytes)", self.0.len())
    }
}

impl Drop for Wiped {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Zeroes `bytes` in a way the optimizer may not remove as a dead store.
pub(crate) fn wipe(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// The credential blob for `secret`: its UTF-8 bytes. Refuses what would not fit.
fn encode_blob(secret: &str) -> Result<Wiped, SecretStoreError> {
    if secret.len() > MAX_BLOB_BYTES {
        return Err(SecretStoreError(format!(
            "Windows Credential Manager stores at most {MAX_BLOB_BYTES} bytes per item"
        )));
    }
    Ok(Wiped(secret.as_bytes().to_vec()))
}

/// The secret in a credential blob, or `None` when it is not UTF-8 (an entry another program
/// wrote under our name).
fn decode_blob(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

/// What `exists` last learned about each item, and when.
#[derive(Default)]
struct ExistsCache {
    known: Mutex<HashMap<String, (bool, Instant)>>,
}

impl ExistsCache {
    fn get(&self, target: &str, now: Instant) -> Option<bool> {
        let known = self.known.lock().ok()?;
        let (found, at) = known.get(target)?;
        (now.saturating_duration_since(*at) < EXISTS_TTL).then_some(*found)
    }

    fn set(&self, target: &str, found: bool, now: Instant) {
        if let Ok(mut known) = self.known.lock() {
            known.insert(target.to_owned(), (found, now));
        }
    }

    fn forget(&self, target: &str) {
        if let Ok(mut known) = self.known.lock() {
            known.remove(target);
        }
    }
}

// MARK: Win32

#[cfg(windows)]
mod os {
    use super::Wiped;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    pub enum Fetched {
        Found(Wiped),
        Missing,
        Failed,
    }

    /// Reads the credential `target` (NUL-terminated UTF-16).
    pub fn fetch(target: &[u16]) -> Fetched {
        let mut credential: *mut CREDENTIALW = null_mut();
        // SAFETY: `target` is NUL-terminated and outlives the call; `credential` receives a
        // buffer that `CredFree` releases.
        let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if ok == 0 || credential.is_null() {
            // SAFETY: no other Win32 call ran since the failure.
            let code = unsafe { GetLastError() };
            return if code == ERROR_NOT_FOUND {
                Fetched::Missing
            } else {
                Fetched::Failed
            };
        }
        // SAFETY: the call succeeded, so `credential` points at a valid `CREDENTIALW` whose blob
        // holds `CredentialBlobSize` bytes, until `CredFree`.
        let blob = unsafe {
            let credential_ref = &*credential;
            let size = credential_ref.CredentialBlobSize as usize;
            if credential_ref.CredentialBlob.is_null() || size == 0 {
                Vec::new()
            } else {
                let copy = std::slice::from_raw_parts(credential_ref.CredentialBlob, size).to_vec();
                // The system's copy is not ours after `CredFree`; clear it first.
                std::ptr::write_bytes(credential_ref.CredentialBlob, 0, size);
                copy
            }
        };
        // SAFETY: `credential` came from `CredReadW` and is freed once.
        unsafe { CredFree(credential as *const std::ffi::c_void) };
        Fetched::Found(Wiped(blob))
    }

    /// Creates or replaces the credential `target` with `blob`.
    pub fn store(target: &[u16], user: &[u16], blob: &mut Wiped) -> Result<(), u32> {
        let mut target = target.to_vec();
        let mut user = user.to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.0.len() as u32,
            CredentialBlob: if blob.0.is_empty() {
                null_mut()
            } else {
                blob.0.as_mut_ptr()
            },
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: `credential` and the buffers it points to are valid for the call; the system
        // copies what it keeps.
        let ok = unsafe { CredWriteW(&credential, 0) };
        if ok == 0 {
            // SAFETY: no other Win32 call ran since the failure.
            Err(unsafe { GetLastError() })
        } else {
            Ok(())
        }
    }

    /// Deletes the credential `target`; a missing one counts as deleted.
    pub fn remove(target: &[u16]) -> Result<(), u32> {
        // SAFETY: `target` is NUL-terminated and outlives the call.
        let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok != 0 {
            return Ok(());
        }
        // SAFETY: no other Win32 call ran since the failure.
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            Ok(())
        } else {
            Err(code)
        }
    }
}

// MARK: The platform seam

#[cfg(windows)]
mod seam {
    use super::*;
    use crate::service::keychain_webhook_secrets::Scope;
    use std::sync::OnceLock;

    fn cache() -> &'static ExistsCache {
        static CACHE: OnceLock<ExistsCache> = OnceLock::new();
        CACHE.get_or_init(ExistsCache::default)
    }

    pub fn read(scope: Scope, account: &str) -> Option<String> {
        let name = target_name(scope.service, account)?;
        match os::fetch(&wide(&name)) {
            os::Fetched::Found(blob) => {
                cache().set(&name, true, Instant::now());
                decode_blob(&blob.0)
            }
            os::Fetched::Missing => {
                cache().set(&name, false, Instant::now());
                None
            }
            os::Fetched::Failed => None,
        }
    }

    pub fn exists(scope: Scope, account: &str) -> bool {
        let Some(name) = target_name(scope.service, account) else {
            return false;
        };
        let now = Instant::now();
        if let Some(found) = cache().get(&name, now) {
            return found;
        }
        match os::fetch(&wide(&name)) {
            os::Fetched::Found(_) => {
                cache().set(&name, true, now);
                true
            }
            os::Fetched::Missing => {
                cache().set(&name, false, now);
                false
            }
            // Not knowing is not an answer worth keeping.
            os::Fetched::Failed => false,
        }
    }

    pub fn write(scope: Scope, account: &str, secret: &str) -> Result<(), SecretStoreError> {
        let name = target_name(scope.service, account)
            .ok_or_else(|| SecretStoreError("invalid Credential Manager item name".to_owned()))?;
        let mut blob = encode_blob(secret)?;
        let result = os::store(&wide(&name), &wide(USER_NAME), &mut blob);
        match result {
            Ok(()) => {
                cache().set(&name, true, Instant::now());
                Ok(())
            }
            Err(code) => {
                cache().forget(&name);
                Err(SecretStoreError(format!(
                    "Windows Credential Manager error {code}"
                )))
            }
        }
    }

    pub fn delete(scope: Scope, account: &str) -> Result<(), SecretStoreError> {
        let Some(name) = target_name(scope.service, account) else {
            return Err(SecretStoreError(
                "That credential name cannot be used on Windows.".to_owned(),
            ));
        };
        match os::remove(&wide(&name)) {
            Ok(()) => {
                cache().set(&name, false, Instant::now());
                Ok(())
            }
            Err(code) => {
                cache().forget(&name);
                Err(SecretStoreError(format!(
                    "Windows Credential Manager error {code}"
                )))
            }
        }
    }
}

#[cfg(windows)]
pub use seam::{delete, exists, read, write};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_name_joins_service_and_account() {
        assert_eq!(
            target_name("es.nico.degenview.webhook", "endpoint-3").as_deref(),
            Some("es.nico.degenview.webhook/endpoint-3")
        );
    }

    #[test]
    fn a_target_name_that_cannot_be_one_is_refused() {
        assert_eq!(target_name("", "a"), None);
        assert_eq!(target_name("s", ""), None);
        assert_eq!(target_name("s", "a\0b"), None);
        assert_eq!(target_name("s", &"x".repeat(MAX_TARGET_UNITS)), None);
        assert!(target_name("s", &"x".repeat(MAX_TARGET_UNITS - 2)).is_some());
    }

    #[test]
    fn wide_strings_end_in_nul() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(wide(""), vec![0]);
    }

    #[test]
    fn a_blob_round_trips_any_text() {
        for text in ["", "token", "s3cret ünï/😀", "日本語のキー"] {
            let blob = encode_blob(text).unwrap();
            assert_eq!(decode_blob(&blob.0).as_deref(), Some(text));
        }
    }

    #[test]
    fn a_blob_is_the_utf8_bytes_without_a_terminator() {
        assert_eq!(encode_blob("Ab").unwrap().0, b"Ab".to_vec());
        assert_eq!(encode_blob("😀").unwrap().0.len(), 4);
    }

    #[test]
    fn a_webhook_secret_at_its_longest_fits() {
        use crate::model::webhook_policy::MAXIMUM_SECRET_LENGTH;
        assert!(encode_blob(&"k".repeat(MAXIMUM_SECRET_LENGTH)).is_ok());
    }

    #[test]
    fn a_secret_that_does_not_fit_is_refused_not_truncated() {
        assert!(encode_blob(&"k".repeat(MAX_BLOB_BYTES)).is_ok());
        let error = encode_blob(&"k".repeat(MAX_BLOB_BYTES + 1)).unwrap_err();
        assert!(error.0.contains("2560"));
        // The limit counts bytes, so a four-byte character counts four times.
        assert!(encode_blob(&"😀".repeat(MAX_BLOB_BYTES / 4)).is_ok());
        assert!(encode_blob(&"😀".repeat(MAX_BLOB_BYTES / 4 + 1)).is_err());
    }

    #[test]
    fn a_blob_that_is_not_text_reads_as_nothing() {
        assert_eq!(decode_blob(&[0xFF, 0xFE, 0x41]), None);
    }

    #[test]
    fn errors_never_contain_the_secret() {
        let error = encode_blob(&"hunter2".repeat(400)).unwrap_err();
        assert!(!error.0.contains("hunter2"));
    }

    #[test]
    fn wiped_bytes_are_zeroed_on_drop() {
        let mut bytes = vec![1u8, 2, 3];
        wipe(&mut bytes);
        assert_eq!(bytes, vec![0, 0, 0]);
    }

    #[test]
    fn the_exists_cache_expires_and_forgets() {
        let cache = ExistsCache::default();
        let start = Instant::now();
        assert_eq!(cache.get("a", start), None);
        cache.set("a", true, start);
        assert_eq!(cache.get("a", start), Some(true));
        assert_eq!(
            cache.get("a", start + EXISTS_TTL - Duration::from_secs(1)),
            Some(true)
        );
        assert_eq!(cache.get("a", start + EXISTS_TTL), None);
        cache.set("a", false, start);
        assert_eq!(cache.get("a", start), Some(false));
        cache.forget("a");
        assert_eq!(cache.get("a", start), None);
    }

    /// A round trip through the real Credential Manager. `is_disabled` does not apply here: this
    /// calls the store itself, with an account of its own that it removes again.
    #[cfg(windows)]
    #[test]
    fn credential_manager_round_trip() {
        use crate::service::keychain_webhook_secrets::Scope;

        struct Cleanup(Scope, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(self.0, &self.1);
            }
        }

        let scope = Scope {
            service: "es.nico.degenview.test",
            shared_group: false,
        };
        let account = format!("test-{}", std::process::id());
        let _cleanup = Cleanup(scope, account.clone());

        delete(scope, &account).unwrap();
        assert!(!exists(scope, &account));
        assert_eq!(read(scope, &account), None);

        write(scope, &account, "s3cret ünï/😀").unwrap();
        assert!(exists(scope, &account));
        assert_eq!(read(scope, &account).as_deref(), Some("s3cret ünï/😀"));

        write(scope, &account, "replaced").unwrap();
        assert_eq!(read(scope, &account).as_deref(), Some("replaced"));

        // The largest secret that fits.
        let longest = "k".repeat(MAX_BLOB_BYTES);
        write(scope, &account, &longest).unwrap();
        assert_eq!(read(scope, &account).as_deref(), Some(longest.as_str()));
        assert!(write(scope, &account, &(longest + "k")).is_err());
        // A refused write leaves the old value alone.
        assert_eq!(read(scope, &account).map(|v| v.len()), Some(MAX_BLOB_BYTES));

        delete(scope, &account).unwrap();
        assert!(!exists(scope, &account));
        assert_eq!(read(scope, &account), None);
        // Deleting what is not there is fine.
        delete(scope, &account).unwrap();
    }
}
