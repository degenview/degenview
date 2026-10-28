//! The macOS Keychain behind `WebhookSecretStore` (Linux: the Secret Service, see
//! `secret_service`): one generic-password item per endpoint. Talks to the Security framework directly (a few C calls), so it adds no
//! crate to the build.
//!
//! Items go in the shared `es.nico.degenview.shared` access group when this process is
//! entitled to it, which both the app and the login-item alert agent are, and are read
//! from that group only. The agent has no UI to answer a Keychain prompt, so reads fail
//! instead of asking, and a missing item is simply "unavailable". Without the entitlement
//! (Debug builds) the app can still store and read its own item, but the agent cannot see
//! it.
//!
//! Under `is_disabled` (tests, `DEGENVIEW_NO_KEYCHAIN=1`) values stay in memory: a
//! rebuilt, re-signed binary would otherwise block on a Keychain access prompt.

#![allow(dead_code)] // Transient: the Settings ▸ Webhooks editor and the alert runtime wire it next.

use crate::model::webhook::EndpointId;
use crate::service::webhook_delivery::{
    InMemoryWebhookSecretStore, SecretStoreError, WebhookSecretStore,
};

const SERVICE: &str = "es.nico.degenview.webhook";

/// Which Keychain items a call addresses: a service name, and whether the item lives in the
/// shared access group (when this process is entitled to it) or in the process's own default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Scope {
    pub service: &'static str,
    pub shared_group: bool,
}

const WEBHOOKS: Scope = Scope {
    service: SERVICE,
    shared_group: true,
};
const ACCESS_GROUP_SUFFIX: &str = "es.nico.degenview.shared";

/// Whether the Keychain is off-limits to this process.
pub fn is_disabled() -> bool {
    cfg!(test) || std::env::var_os("DEGENVIEW_NO_KEYCHAIN").is_some()
}

/// The Keychain account name for an endpoint.
pub fn account(endpoint: EndpointId) -> String {
    format!("endpoint-{endpoint}")
}

/// Picks the shared group out of the process's `keychain-access-groups` entitlement.
pub fn shared_access_group(groups: &[String]) -> Option<&str> {
    groups
        .iter()
        .map(String::as_str)
        .find(|group| group.ends_with(ACCESS_GROUP_SUFFIX))
}

pub struct KeychainWebhookSecretStore {
    /// Used instead of the Keychain when it is disabled.
    memory: InMemoryWebhookSecretStore,
}

impl KeychainWebhookSecretStore {
    pub fn new() -> Self {
        Self {
            memory: InMemoryWebhookSecretStore::default(),
        }
    }
}

impl Default for KeychainWebhookSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookSecretStore for KeychainWebhookSecretStore {
    fn secret(&self, endpoint: EndpointId) -> Option<String> {
        if is_disabled() {
            return self.memory.secret(endpoint);
        }
        platform::read(WEBHOOKS, &account(endpoint))
    }

    fn has_secret(&self, endpoint: EndpointId) -> bool {
        if is_disabled() {
            return self.memory.has_secret(endpoint);
        }
        platform::exists(WEBHOOKS, &account(endpoint))
    }

    fn set_secret(&self, endpoint: EndpointId, secret: &str) -> Result<(), SecretStoreError> {
        if is_disabled() {
            return self.memory.set_secret(endpoint, secret);
        }
        platform::write(WEBHOOKS, &account(endpoint), secret)
    }

    fn remove(&self, endpoint: EndpointId) {
        if is_disabled() {
            return self.memory.remove(endpoint);
        }
        platform::delete(WEBHOOKS, &account(endpoint));
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod platform {
    use super::{Scope, SecretStoreError, shared_access_group};
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;
    type CFIndex = isize;
    type OSStatus = i32;

    const ERR_SEC_SUCCESS: OSStatus = 0;
    const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;
    const ERR_SEC_INTERACTION_NOT_ALLOWED: OSStatus = -25308;
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    struct CFDictionaryKeyCallBacks {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct CFDictionaryValueCallBacks {
        _private: [u8; 0],
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
        static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
        static kCFBooleanTrue: CFTypeRef;

        fn CFRelease(value: CFTypeRef);
        fn CFDictionaryCreate(
            allocator: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: CFIndex,
            key_callbacks: *const CFDictionaryKeyCallBacks,
            value_callbacks: *const CFDictionaryValueCallBacks,
        ) -> CFTypeRef;
        fn CFStringCreateWithBytes(
            allocator: CFTypeRef,
            bytes: *const u8,
            length: CFIndex,
            encoding: u32,
            is_external: u8,
        ) -> CFTypeRef;
        fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
        fn CFStringGetBytes(
            string: CFTypeRef,
            range_location: CFIndex,
            range_length: CFIndex,
            encoding: u32,
            loss_byte: u8,
            is_external: u8,
            buffer: *mut u8,
            max_buffer: CFIndex,
            used: *mut CFIndex,
        ) -> CFIndex;
        fn CFDataCreate(allocator: CFTypeRef, bytes: *const u8, length: CFIndex) -> CFTypeRef;
        fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
        fn CFDataGetLength(data: CFTypeRef) -> CFIndex;
        fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
    }

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecClass: CFTypeRef;
        static kSecClassGenericPassword: CFTypeRef;
        static kSecAttrService: CFTypeRef;
        static kSecAttrAccount: CFTypeRef;
        static kSecAttrAccessGroup: CFTypeRef;
        static kSecReturnData: CFTypeRef;
        static kSecReturnAttributes: CFTypeRef;
        static kSecMatchLimit: CFTypeRef;
        static kSecMatchLimitOne: CFTypeRef;
        static kSecValueData: CFTypeRef;
        static kSecUseAuthenticationUI: CFTypeRef;
        static kSecUseAuthenticationUIFail: CFTypeRef;

        fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
        fn SecItemAdd(attributes: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
        fn SecItemUpdate(query: CFTypeRef, attributes: CFTypeRef) -> OSStatus;
        fn SecItemDelete(query: CFTypeRef) -> OSStatus;
        fn SecTaskCreateFromSelf(allocator: CFTypeRef) -> CFTypeRef;
        fn SecTaskCopyValueForEntitlement(
            task: CFTypeRef,
            entitlement: CFTypeRef,
            error: *mut CFTypeRef,
        ) -> CFTypeRef;
    }

    /// An owned Core Foundation object, released on drop.
    struct Owned(CFTypeRef);

    impl Owned {
        fn new(raw: CFTypeRef) -> Option<Self> {
            // Lazily: `then_some(Self(raw))` would build — and so drop, and `CFRelease` — a
            // wrapper around NULL whenever a Security call returned nothing, which crashes.
            (!raw.is_null()).then(|| Self(raw))
        }
        fn as_raw(&self) -> CFTypeRef {
            self.0
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a non-null object this wrapper owns exactly once.
            unsafe { CFRelease(self.0) };
        }
    }

    #[cfg(test)]
    #[test]
    fn a_null_result_is_never_released() {
        // Before the fix this aborted the process with "CFRelease() called with NULL".
        assert!(Owned::new(std::ptr::null()).is_none());
        // An entitlement the process does not have reads as none, without crashing.
        // SAFETY: as in `access_group`.
        let missing = unsafe {
            let task = Owned::new(SecTaskCreateFromSelf(std::ptr::null()));
            let key = string("com.degenview.test.no-such-entitlement");
            match (task, key) {
                (Some(task), Some(key)) => Owned::new(SecTaskCopyValueForEntitlement(
                    task.as_raw(),
                    key.as_raw(),
                    std::ptr::null_mut(),
                )),
                _ => None,
            }
        };
        assert!(missing.is_none());
    }

    fn string(text: &str) -> Option<Owned> {
        // SAFETY: the pointer and length describe `text`, which outlives the call.
        Owned::new(unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                text.as_ptr(),
                text.len() as CFIndex,
                CF_STRING_ENCODING_UTF8,
                0,
            )
        })
    }

    fn data(bytes: &[u8]) -> Option<Owned> {
        // SAFETY: the pointer and length describe `bytes`, which outlives the call.
        Owned::new(unsafe {
            CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex)
        })
    }

    /// A dictionary of borrowed keys to borrowed values (both stay alive for the call).
    fn dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> Option<Owned> {
        let keys: Vec<CFTypeRef> = pairs.iter().map(|(key, _)| *key).collect();
        let values: Vec<CFTypeRef> = pairs.iter().map(|(_, value)| *value).collect();
        // SAFETY: the arrays hold `pairs.len()` valid CF objects; the standard CFType
        // callbacks retain them, so the dictionary owns its own references.
        Owned::new(unsafe {
            CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                pairs.len() as CFIndex,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
        })
    }

    fn string_from(raw: CFTypeRef) -> Option<String> {
        // SAFETY: `raw` is a CFString the caller got from Core Foundation; the buffer is
        // sized for the worst case of four bytes per UTF-16 unit.
        unsafe {
            let length = CFStringGetLength(raw);
            let capacity = (length as usize).saturating_mul(4).max(1);
            let mut buffer = vec![0u8; capacity];
            let mut used: CFIndex = 0;
            let converted = CFStringGetBytes(
                raw,
                0,
                length,
                CF_STRING_ENCODING_UTF8,
                0,
                0,
                buffer.as_mut_ptr(),
                capacity as CFIndex,
                &mut used,
            );
            (converted == length).then(|| {
                buffer.truncate(used as usize);
                String::from_utf8_lossy(&buffer).into_owned()
            })
        }
    }

    /// The `keychain-access-groups` entitlement of this process, if it has one.
    fn access_group() -> Option<String> {
        // SAFETY: standard CF ownership: both the task and the copied value are +1 and are
        // wrapped to be released; array elements are borrowed while the array lives.
        unsafe {
            let task = Owned::new(SecTaskCreateFromSelf(std::ptr::null()))?;
            let entitlement = string("keychain-access-groups")?;
            let value = Owned::new(SecTaskCopyValueForEntitlement(
                task.as_raw(),
                entitlement.as_raw(),
                std::ptr::null_mut(),
            ))?;
            let count = CFArrayGetCount(value.as_raw());
            let groups: Vec<String> = (0..count)
                .filter_map(|index| string_from(CFArrayGetValueAtIndex(value.as_raw(), index)))
                .collect();
            shared_access_group(&groups).map(str::to_owned)
        }
    }

    /// The attributes that identify an endpoint's item. Reads never prompt.
    struct Query {
        service: Owned,
        account: Owned,
        group: Option<Owned>,
    }

    impl Query {
        fn new(scope: Scope, account: &str) -> Option<Self> {
            Some(Self {
                service: string(scope.service)?,
                account: string(account)?,
                group: if scope.shared_group {
                    access_group().and_then(|group| string(&group))
                } else {
                    None
                },
            })
        }

        /// Identity attributes, plus `extra` pairs, as a dictionary.
        fn dictionary(&self, extra: &[(CFTypeRef, CFTypeRef)]) -> Option<Owned> {
            // SAFETY: reading the framework's constant keys/values.
            let mut pairs = unsafe {
                vec![
                    (kSecClass, kSecClassGenericPassword),
                    (kSecAttrService, self.service.as_raw()),
                    (kSecAttrAccount, self.account.as_raw()),
                ]
            };
            if let Some(group) = &self.group {
                // SAFETY: as above.
                pairs.push((unsafe { kSecAttrAccessGroup }, group.as_raw()));
            }
            pairs.extend_from_slice(extra);
            dictionary(&pairs)
        }
    }

    pub fn read(scope: Scope, account: &str) -> Option<String> {
        let query = Query::new(scope, account)?;
        // SAFETY: constants from the framework.
        let (return_data, limit_one, ui_key, ui_fail, yes) = unsafe {
            (
                kSecReturnData,
                kSecMatchLimitOne,
                kSecUseAuthenticationUI,
                kSecUseAuthenticationUIFail,
                kCFBooleanTrue,
            )
        };
        let dict = query.dictionary(&[
            (return_data, yes),
            // SAFETY: constant from the framework.
            (unsafe { kSecMatchLimit }, limit_one),
            (ui_key, ui_fail),
        ])?;
        let mut result: CFTypeRef = std::ptr::null();
        // SAFETY: `dict` is a valid query; `result` receives a +1 CFData on success.
        let status = unsafe { SecItemCopyMatching(dict.as_raw(), &mut result) };
        if status != ERR_SEC_SUCCESS {
            return None;
        }
        let bytes = Owned::new(result)?;
        // SAFETY: `bytes` is a CFData; the pointer is valid for `length` bytes while it lives.
        let value = unsafe {
            let length = CFDataGetLength(bytes.as_raw()).max(0) as usize;
            std::slice::from_raw_parts(CFDataGetBytePtr(bytes.as_raw()), length).to_vec()
        };
        String::from_utf8(value).ok()
    }

    pub fn exists(scope: Scope, account: &str) -> bool {
        let Some(query) = Query::new(scope, account) else {
            return false;
        };
        // SAFETY: constants from the framework.
        let (return_attributes, limit, limit_one, ui_key, ui_fail, yes) = unsafe {
            (
                kSecReturnAttributes,
                kSecMatchLimit,
                kSecMatchLimitOne,
                kSecUseAuthenticationUI,
                kSecUseAuthenticationUIFail,
                kCFBooleanTrue,
            )
        };
        let Some(dict) = query.dictionary(&[
            (return_attributes, yes),
            (limit, limit_one),
            (ui_key, ui_fail),
        ]) else {
            return false;
        };
        let mut result: CFTypeRef = std::ptr::null();
        // SAFETY: as in `read`.
        let status = unsafe { SecItemCopyMatching(dict.as_raw(), &mut result) };
        drop(Owned::new(result));
        // An item that exists but would need to prompt still exists.
        status == ERR_SEC_SUCCESS || status == ERR_SEC_INTERACTION_NOT_ALLOWED
    }

    pub fn write(scope: Scope, account: &str, secret: &str) -> Result<(), SecretStoreError> {
        let fail = |reason: &str| SecretStoreError(reason.to_owned());
        let query = Query::new(scope, account).ok_or_else(|| fail("Keychain unavailable"))?;
        let value = data(secret.as_bytes()).ok_or_else(|| fail("Keychain unavailable"))?;
        // SAFETY: constant from the framework.
        let value_key = unsafe { kSecValueData };

        let key = query
            .dictionary(&[])
            .ok_or_else(|| fail("Keychain unavailable"))?;
        let update = dictionary(&[(value_key, value.as_raw())])
            .ok_or_else(|| fail("Keychain unavailable"))?;
        // SAFETY: both dictionaries are valid for the call.
        let status = unsafe { SecItemUpdate(key.as_raw(), update.as_raw()) };
        if status == ERR_SEC_SUCCESS {
            return Ok(());
        }
        if status != ERR_SEC_ITEM_NOT_FOUND {
            return Err(SecretStoreError(format!("Keychain error {status}")));
        }
        let item = query
            .dictionary(&[(value_key, value.as_raw())])
            .ok_or_else(|| fail("Keychain unavailable"))?;
        // SAFETY: `item` is a valid attribute dictionary; no result is requested.
        let added = unsafe { SecItemAdd(item.as_raw(), std::ptr::null_mut()) };
        if added == ERR_SEC_SUCCESS {
            Ok(())
        } else {
            Err(SecretStoreError(format!("Keychain error {added}")))
        }
    }

    pub fn delete(scope: Scope, account: &str) {
        let Some(query) = Query::new(scope, account) else {
            return;
        };
        if let Some(key) = query.dictionary(&[]) {
            // SAFETY: `key` is a valid query dictionary.
            let _ = unsafe { SecItemDelete(key.as_raw()) };
        }
    }
}

/// Linux keeps secrets in the Secret Service, or the Secret portal inside a Flatpak.
#[cfg(target_os = "linux")]
pub(crate) use crate::service::secret_service as platform;

/// Other platforms have no secret store: nothing is stored or read.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) mod platform {
    use super::{Scope, SecretStoreError};

    pub fn read(_: Scope, _: &str) -> Option<String> {
        None
    }
    pub fn exists(_: Scope, _: &str) -> bool {
        false
    }
    pub fn write(_: Scope, _: &str, _: &str) -> Result<(), SecretStoreError> {
        Err(SecretStoreError("no Keychain on this platform".to_owned()))
    }
    pub fn delete(_: Scope, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_names_the_endpoint() {
        assert_eq!(account(42), "endpoint-42");
    }

    #[test]
    fn only_the_shared_group_is_used() {
        let groups = vec![
            "TEAMID.com.example.other".to_owned(),
            "TEAMID.es.nico.degenview.shared".to_owned(),
        ];
        assert_eq!(
            shared_access_group(&groups),
            Some("TEAMID.es.nico.degenview.shared")
        );
        assert_eq!(shared_access_group(&groups[..1]), None);
        assert_eq!(shared_access_group(&[]), None);
    }

    #[test]
    fn under_test_the_keychain_is_off_and_values_stay_in_memory() {
        assert!(is_disabled());
        let store = KeychainWebhookSecretStore::new();
        assert_eq!(store.secret(1), None);
        assert!(!store.has_secret(1));
        store.set_secret(1, "token").unwrap();
        assert_eq!(store.secret(1).as_deref(), Some("token"));
        assert!(store.has_secret(1));
        store.remove(1);
        assert_eq!(store.secret(1), None);
    }

    /// A round trip through the real Keychain, for a human to run once after changing the
    /// FFI: `cargo test keychain_round_trip -- --ignored`. It cannot run in the normal
    /// suite (`is_disabled` is true under test), and a rebuilt binary may prompt.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches the real Keychain"]
    fn keychain_round_trip() {
        let account = format!("test-{}", std::process::id());
        for scope in [
            WEBHOOKS,
            Scope {
                service: "es.nico.degenview.test",
                shared_group: false,
            },
        ] {
            platform::delete(scope, &account);
            assert!(!platform::exists(scope, &account));
            platform::write(scope, &account, "s3cret ünï/😀").unwrap();
            assert!(platform::exists(scope, &account));
            assert_eq!(
                platform::read(scope, &account).as_deref(),
                Some("s3cret ünï/😀")
            );
            platform::write(scope, &account, "replaced").unwrap();
            assert_eq!(platform::read(scope, &account).as_deref(), Some("replaced"));
            platform::delete(scope, &account);
            assert_eq!(platform::read(scope, &account), None);
        }
    }
}
