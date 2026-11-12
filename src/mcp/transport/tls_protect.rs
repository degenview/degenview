//! How the TLS identity file (`mcp-tls.pem`) is kept on disk.
//!
//! On macOS and Linux the file is the PEM text, readable only by its owner (`0600`, as an SSH key
//! is; see `tls`). Windows has no such mode, so there the whole file is encrypted with DPAPI
//! (`CryptProtectData`) under the signed-in user's key. Windows normally lets only that user, on
//! that computer, decrypt it, so a copy, a backup or a disk image is not readable without the
//! user's Windows credentials. That is the usual case, not a guarantee: Microsoft documents that a
//! user with a roaming profile can decrypt DPAPI data on another computer.
//!
//! What it does not give: any process running as the same user can decrypt the file. The entropy
//! passed to DPAPI is a fixed string that ties the blob to this use, not a secret. The identity is
//! in memory as plain text while the server runs, as it is on every platform.
//!
//! A file that is not in the form this platform writes (plain PEM text on Windows, for example) is
//! refused, never read as it is: there is no older form to migrate from.

/// The identity file's contents as they go to and come from disk.
pub trait AtRest {
    /// The bytes to write for `text`.
    fn wrap(&self, text: &str) -> Result<Vec<u8>, String>;
    /// The text a file written by [`AtRest::wrap`] holds. The error names no path: the caller adds it.
    fn unwrap(&self, bytes: &[u8]) -> Result<String, String>;
}

/// The text as it is, for platforms where the file's mode is the protection.
#[cfg_attr(windows, allow(dead_code))]
pub struct Plain;

impl AtRest for Plain {
    fn wrap(&self, text: &str) -> Result<Vec<u8>, String> {
        Ok(text.as_bytes().to_vec())
    }

    fn unwrap(&self, bytes: &[u8]) -> Result<String, String> {
        String::from_utf8(bytes.to_vec()).map_err(|_| "The TLS file is not text".to_owned())
    }
}

/// What a release build writes: DPAPI on Windows, the PEM text elsewhere. A test pins this choice
/// (`dpapi_is_what_a_windows_build_writes`), because [`platform`] hides it under `cfg(test)`.
pub fn production() -> &'static dyn AtRest {
    #[cfg(windows)]
    {
        &Dpapi
    }
    #[cfg(not(windows))]
    {
        &Plain
    }
}

/// What `tls::load_or_create` uses. Tests get [`Plain`] everywhere, so the file-handling tests can
/// write and read PEM text directly; the Windows protector has tests of its own.
pub fn platform() -> &'static dyn AtRest {
    if cfg!(test) { &Plain } else { production() }
}

/// Begins every protected file, so a file that is not one is told apart from a damaged blob.
#[cfg_attr(not(windows), allow(dead_code))]
const MAGIC: &[u8] = b"DVTLS-DPAPI1\n";

/// Ties the blob to this use. Not a secret.
#[cfg_attr(not(windows), allow(dead_code))]
const ENTROPY: &[u8] = b"degenview.mcp.tls.v1";

/// The blob after the magic, or `None` when the bytes do not start with it.
#[cfg_attr(not(windows), allow(dead_code))]
fn strip_magic(bytes: &[u8]) -> Option<&[u8]> {
    bytes.strip_prefix(MAGIC)
}

#[cfg(windows)]
pub struct Dpapi;

#[cfg(windows)]
impl AtRest for Dpapi {
    fn wrap(&self, text: &str) -> Result<Vec<u8>, String> {
        use crate::service::keychain::windows_credentials::{Wiped, wide};
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
        };

        let mut input = Wiped(text.as_bytes().to_vec());
        let data = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(input.0.len()).map_err(|_| "The TLS file is too large")?,
            pbData: input.0.as_mut_ptr(),
        };
        let entropy = entropy_blob();
        let description = wide("DegenView MCP TLS identity");
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };
        // SAFETY: every pointer is valid for the call; `out` receives a buffer that `take` frees.
        let ok = unsafe {
            CryptProtectData(
                &data,
                description.as_ptr(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            // SAFETY: no other Win32 call ran since the failure.
            let code = unsafe { GetLastError() };
            return Err(format!(
                "Windows could not protect the TLS key (error {code})"
            ));
        }
        let blob = take(out);
        let mut bytes = Vec::with_capacity(MAGIC.len() + blob.0.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&blob.0);
        Ok(bytes)
    }

    fn unwrap(&self, bytes: &[u8]) -> Result<String, String> {
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
        };

        let body = strip_magic(bytes)
            .ok_or("The TLS file is not protected for this Windows user, so it was not used")?;
        let mut input = body.to_vec();
        let data = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(input.len()).map_err(|_| "The TLS file is too large")?,
            pbData: input.as_mut_ptr(),
        };
        let entropy = entropy_blob();
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };
        // SAFETY: as in `wrap`; the description is not wanted, so no pointer for it.
        let ok = unsafe {
            CryptUnprotectData(
                &data,
                null_mut(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            // SAFETY: no other Win32 call ran since the failure.
            let code = unsafe { GetLastError() };
            return Err(format!(
                "The TLS file cannot be decrypted by this Windows user (error {code}); \
                 it was made by another account or machine, or is damaged"
            ));
        }
        let mut plain = take(out);
        String::from_utf8(std::mem::take(&mut plain.0))
            .map_err(|_| "The TLS file is not text".to_owned())
    }
}

#[cfg(windows)]
fn entropy_blob() -> windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
    windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        // DPAPI only reads the entropy; the field is not `const` in the API.
        pbData: ENTROPY.as_ptr() as *mut u8,
    }
}

/// Copies a buffer DPAPI allocated, clears it, and frees it.
#[cfg(windows)]
fn take(
    out: windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB,
) -> crate::service::keychain::windows_credentials::Wiped {
    use crate::service::keychain::windows_credentials::Wiped;
    use windows_sys::Win32::Foundation::LocalFree;

    if out.pbData.is_null() {
        return Wiped(Vec::new());
    }
    let size = out.cbData as usize;
    // SAFETY: DPAPI returned `size` bytes at `pbData`; they are copied, cleared and freed once.
    let copy = unsafe {
        let copy = std::slice::from_raw_parts(out.pbData, size).to_vec();
        std::ptr::write_bytes(out.pbData, 0, size);
        LocalFree(out.pbData as *mut std::ffi::c_void);
        copy
    };
    Wiped(copy)
}

/// A reversible stand-in for a protector, so the file handling in `tls` is tested on every
/// platform: it prefixes a marker and reverses the bytes, so the PEM text is not on disk.
#[cfg(test)]
pub struct Fake;

#[cfg(test)]
impl AtRest for Fake {
    fn wrap(&self, text: &str) -> Result<Vec<u8>, String> {
        let mut bytes: Vec<u8> = text.bytes().rev().collect();
        bytes.splice(0..0, b"FAKE:".iter().copied());
        Ok(bytes)
    }

    fn unwrap(&self, bytes: &[u8]) -> Result<String, String> {
        let body = bytes
            .strip_prefix(b"FAKE:")
            .ok_or("The TLS file is not protected")?;
        String::from_utf8(body.iter().rev().copied().collect())
            .map_err(|_| "The TLS file is not text".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_keeps_the_text_as_it_is() {
        let bytes = Plain.wrap("-----BEGIN CERTIFICATE-----").unwrap();
        assert_eq!(bytes, b"-----BEGIN CERTIFICATE-----");
        assert_eq!(Plain.unwrap(&bytes).unwrap(), "-----BEGIN CERTIFICATE-----");
        assert!(Plain.unwrap(&[0xFF, 0xFE]).is_err());
    }

    #[test]
    fn the_magic_is_told_apart_from_pem_text() {
        assert!(strip_magic(b"-----BEGIN CERTIFICATE-----").is_none());
        assert!(strip_magic(b"# degenview-mcp-tls address=10.0.0.7").is_none());
        assert_eq!(strip_magic(b"DVTLS-DPAPI1\nabc"), Some(&b"abc"[..]));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_is_what_a_windows_build_writes() {
        let bytes = production().wrap("-----BEGIN PRIVATE KEY-----").unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert!(!String::from_utf8_lossy(&bytes).contains("BEGIN"));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_pem_text_is_what_other_builds_write() {
        let text = "-----BEGIN PRIVATE KEY-----";
        assert_eq!(production().wrap(text).unwrap(), text.as_bytes());
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trips_and_leaves_no_pem_text_on_disk() {
        let text = "# degenview-mcp-tls address=10.0.0.7\n-----BEGIN PRIVATE KEY-----\nabc\n";
        let bytes = Dpapi.wrap(text).unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE KEY"));
        assert!(!String::from_utf8_lossy(&bytes).contains("10.0.0.7"));
        assert_eq!(Dpapi.unwrap(&bytes).unwrap(), text);
        // Protecting twice gives different bytes (DPAPI salts), both readable.
        let again = Dpapi.wrap(text).unwrap();
        assert_ne!(bytes, again);
        assert_eq!(Dpapi.unwrap(&again).unwrap(), text);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_refuses_plain_text_and_a_damaged_blob() {
        let error = Dpapi.unwrap(b"-----BEGIN CERTIFICATE-----\n").unwrap_err();
        assert!(error.contains("not protected"), "{error}");
        let mut bytes = Dpapi.wrap("secret key").unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let error = Dpapi.unwrap(&bytes).unwrap_err();
        assert!(error.contains("cannot be decrypted"), "{error}");
        assert!(Dpapi.unwrap(MAGIC).is_err(), "an empty blob is not a file");
    }
}
