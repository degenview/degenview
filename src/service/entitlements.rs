//! Whether this copy of DegenView is signed to use iCloud. Creating a CloudKit container without
//! the entitlement raises an Objective-C exception that Swift cannot catch, so the answer comes
//! from the signature first and CloudKit is never touched when it is "no".

use crate::sync::account::CONTAINER;

/// What a signed entitlement holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntitlementValue {
    Text(String),
    List(Vec<String>),
}

pub const SERVICES: &str = "com.apple.developer.icloud-services";
pub const CONTAINERS: &str = "com.apple.developer.icloud-container-identifiers";
pub const ENVIRONMENT: &str = "com.apple.developer.icloud-container-environment";

/// The CloudKit environment this build will talk to, or why it cannot use iCloud at all.
///
/// A debug build must use the Development environment and a release build Production, so test
/// data can never reach people's real iCloud data, nor a release build write to test data.
pub fn icloud_environment(
    read: &dyn Fn(&str) -> Option<EntitlementValue>,
    release_build: bool,
) -> Result<String, String> {
    let not_signed = || {
        "This copy of DegenView is not signed for iCloud. Install a release downloaded from the \
         DegenView website."
            .to_string()
    };
    let has = |key: &str, wanted: &str| match read(key) {
        Some(EntitlementValue::List(items)) => items.iter().any(|item| item == wanted),
        Some(EntitlementValue::Text(text)) => text == wanted,
        None => false,
    };
    if !has(SERVICES, "CloudKit") || !has(CONTAINERS, CONTAINER) {
        return Err(not_signed());
    }
    let environment = match read(ENVIRONMENT) {
        Some(EntitlementValue::Text(text)) if text == "Production" || text == "Development" => text,
        Some(_) => return Err("This build has an unrecognised iCloud environment.".into()),
        // A development-signed build without the key talks to Development; a release must say so.
        None if !release_build => "Development".to_string(),
        None => return Err("This release is missing its iCloud environment.".into()),
    };
    match (release_build, environment.as_str()) {
        (true, "Development") => {
            Err("A release build must not use iCloud's Development environment.".into())
        }
        (false, "Production") => {
            Err("A development build must not use iCloud's Production environment.".into())
        }
        _ => Ok(environment),
    }
}

#[cfg(target_os = "macos")]
pub fn read_self(name: &str) -> Option<EntitlementValue> {
    platform::read(name)
}

#[cfg(not(target_os = "macos"))]
pub fn read_self(_name: &str) -> Option<EntitlementValue> {
    None
}

#[cfg(target_os = "macos")]
mod platform {
    use super::EntitlementValue;
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;
    type CFIndex = isize;
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CFTypeRef);
        fn CFGetTypeID(value: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFArrayGetTypeID() -> usize;
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
        fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
    }

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecTaskCreateFromSelf(allocator: CFTypeRef) -> CFTypeRef;
        fn SecTaskCopyValueForEntitlement(
            task: CFTypeRef,
            entitlement: CFTypeRef,
            error: *mut CFTypeRef,
        ) -> CFTypeRef;
    }

    /// Releases a +1 Core Foundation object on drop. Never wraps NULL.
    struct Owned(CFTypeRef);

    impl Owned {
        fn new(raw: CFTypeRef) -> Option<Self> {
            (!raw.is_null()).then(|| Self(raw))
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: a non-null object this wrapper owns exactly once.
            unsafe { CFRelease(self.0) };
        }
    }

    fn cf_string(text: &str) -> Option<Owned> {
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

    fn rust_string(raw: CFTypeRef) -> Option<String> {
        // SAFETY: `raw` is a CFString; the buffer fits four bytes per UTF-16 unit.
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

    pub fn read(name: &str) -> Option<EntitlementValue> {
        // SAFETY: standard Core Foundation ownership: the task and the copied value are +1 and
        // wrapped to be released; array elements are borrowed while the array lives.
        unsafe {
            let task = Owned::new(SecTaskCreateFromSelf(std::ptr::null()))?;
            let key = cf_string(name)?;
            let value = Owned::new(SecTaskCopyValueForEntitlement(
                task.0,
                key.0,
                std::ptr::null_mut(),
            ))?;
            let type_id = CFGetTypeID(value.0);
            if type_id == CFStringGetTypeID() {
                rust_string(value.0).map(EntitlementValue::Text)
            } else if type_id == CFArrayGetTypeID() {
                let count = CFArrayGetCount(value.0);
                Some(EntitlementValue::List(
                    (0..count)
                        .filter_map(|i| rust_string(CFArrayGetValueAtIndex(value.0, i)))
                        .collect(),
                ))
            } else {
                None
            }
        }
    }

    #[cfg(test)]
    #[test]
    fn an_entitlement_this_process_lacks_reads_as_none() {
        assert_eq!(read("com.degenview.test.no-such-entitlement"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn signed(env: Option<&str>) -> HashMap<&'static str, EntitlementValue> {
        let mut map = HashMap::new();
        map.insert(SERVICES, EntitlementValue::List(vec!["CloudKit".into()]));
        map.insert(CONTAINERS, EntitlementValue::List(vec![CONTAINER.into()]));
        if let Some(env) = env {
            map.insert(ENVIRONMENT, EntitlementValue::Text(env.into()));
        }
        map
    }

    fn check(
        map: &HashMap<&'static str, EntitlementValue>,
        release: bool,
    ) -> Result<String, String> {
        icloud_environment(&|key| map.get(key).cloned(), release)
    }

    #[test]
    fn an_unsigned_build_is_unavailable_and_never_reaches_cloudkit() {
        let none = HashMap::new();
        assert!(
            check(&none, true)
                .unwrap_err()
                .contains("not signed for iCloud")
        );
        assert!(check(&none, false).is_err());
    }

    #[test]
    fn the_container_and_service_must_both_be_present() {
        let mut no_service = signed(Some("Production"));
        no_service.remove(SERVICES);
        assert!(check(&no_service, true).is_err());
        let mut other_container = signed(Some("Production"));
        other_container.insert(
            CONTAINERS,
            EntitlementValue::List(vec!["iCloud.other".into()]),
        );
        assert!(check(&other_container, true).is_err());
        let mut anonymous_only = signed(Some("Production"));
        anonymous_only.insert(
            SERVICES,
            EntitlementValue::List(vec!["CloudKit-Anonymous".into()]),
        );
        assert!(check(&anonymous_only, true).is_err());
    }

    #[test]
    fn a_release_uses_production_and_a_debug_build_uses_development() {
        assert_eq!(
            check(&signed(Some("Production")), true),
            Ok("Production".into())
        );
        assert_eq!(
            check(&signed(Some("Development")), false),
            Ok("Development".into())
        );
    }

    #[test]
    fn a_build_can_never_cross_environments() {
        assert!(
            check(&signed(Some("Development")), true)
                .unwrap_err()
                .contains("must not use")
        );
        assert!(
            check(&signed(Some("Production")), false)
                .unwrap_err()
                .contains("must not use")
        );
    }

    #[test]
    fn a_missing_environment_defaults_to_development_only_for_debug_builds() {
        assert_eq!(check(&signed(None), false), Ok("Development".into()));
        assert!(check(&signed(None), true).unwrap_err().contains("missing"));
    }

    #[test]
    fn an_unknown_environment_is_refused() {
        assert!(check(&signed(Some("Staging")), true).is_err());
        let mut odd = signed(None);
        odd.insert(
            ENVIRONMENT,
            EntitlementValue::List(vec!["Production".into()]),
        );
        assert!(check(&odd, true).is_err());
    }

    #[test]
    fn this_test_process_is_not_signed_for_icloud() {
        assert!(icloud_environment(&read_self, false).is_err());
    }
}
