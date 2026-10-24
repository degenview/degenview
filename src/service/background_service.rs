//! Registration and status for the embedded macOS login item. Other platforms keep alerts in
//! the foreground and expose the same interface so the app's ownership policy stays portable.

use crate::model::alert::AlertServiceState;

pub const AGENT_BUNDLE_ID: &str = "es.nico.degenview.alert-agent";

#[allow(dead_code)] // `Unavailable` is constructed by the non-macOS implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeServiceStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    Failed,
    Unavailable,
}

pub fn map_service_status(status: NativeServiceStatus) -> AlertServiceState {
    match status {
        NativeServiceStatus::NotRegistered => AlertServiceState::Disabled,
        NativeServiceStatus::Enabled => AlertServiceState::Enabled,
        NativeServiceStatus::RequiresApproval => AlertServiceState::RequiresApproval,
        NativeServiceStatus::NotFound => AlertServiceState::NotFound,
        NativeServiceStatus::Failed => AlertServiceState::Failed,
        NativeServiceStatus::Unavailable => AlertServiceState::Unavailable,
    }
}

#[derive(Default)]
pub struct BackgroundAlertService {
    registration_failed: bool,
}

impl BackgroundAlertService {
    pub fn status(&self) -> AlertServiceState {
        map_service_status(platform::status(self.registration_failed))
    }

    /// Registers the helper when it is embedded. Calling this for an enabled service is harmless.
    pub fn ensure_registered(&mut self) -> AlertServiceState {
        let result = platform::register(false);
        self.registration_failed = result == NativeServiceStatus::Failed;
        map_service_status(result)
    }

    /// Explicit user retry: unregister the old helper before registering the current executable.
    pub fn retry_registration(&mut self) -> AlertServiceState {
        let result = platform::register(true);
        self.registration_failed = result == NativeServiceStatus::Failed;
        map_service_status(result)
    }

    pub fn open_login_items() {
        platform::open_login_items();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{AGENT_BUNDLE_ID, NativeServiceStatus};
    use objc2_foundation::NSString;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};
    use std::path::PathBuf;

    fn helper_path() -> Option<PathBuf> {
        let executable = std::env::current_exe().ok()?;
        let contents = executable.parent()?.parent()?;
        Some(
            contents
                .join("Library/LoginItems")
                .join("DegenViewAlertAgent.app/Contents/Info.plist"),
        )
    }

    fn embedded() -> bool {
        helper_path().is_some_and(|path| path.is_file())
    }

    fn service() -> objc2::rc::Retained<SMAppService> {
        let identifier = NSString::from_str(AGENT_BUNDLE_ID);
        // SAFETY: the identifier is a valid NSString and the API is available on the app's
        // minimum supported macOS version (14).
        unsafe { SMAppService::loginItemServiceWithIdentifier(&identifier) }
    }

    fn native_status() -> NativeServiceStatus {
        if !embedded() {
            return NativeServiceStatus::NotFound;
        }
        // SAFETY: `service` returns a valid SMAppService for the embedded login-item identifier.
        match unsafe { service().status() } {
            SMAppServiceStatus::Enabled => NativeServiceStatus::Enabled,
            SMAppServiceStatus::RequiresApproval => NativeServiceStatus::RequiresApproval,
            SMAppServiceStatus::NotFound => NativeServiceStatus::NotFound,
            SMAppServiceStatus::NotRegistered => NativeServiceStatus::NotRegistered,
            _ => NativeServiceStatus::Failed,
        }
    }

    pub fn status(registration_failed: bool) -> NativeServiceStatus {
        let status = native_status();
        if registration_failed
            && !matches!(
                status,
                NativeServiceStatus::Enabled | NativeServiceStatus::RequiresApproval
            )
        {
            NativeServiceStatus::Failed
        } else {
            status
        }
    }

    pub fn register(force: bool) -> NativeServiceStatus {
        if !embedded() {
            return NativeServiceStatus::NotFound;
        }
        let service = service();
        // SAFETY: the receiver is a valid SMAppService. Errors are mapped into UI state.
        let status = unsafe { service.status() };
        if status == SMAppServiceStatus::Enabled && !force {
            return NativeServiceStatus::Enabled;
        }
        if force && status != SMAppServiceStatus::NotRegistered {
            // SAFETY: synchronous unregister is specifically used for deliberate re-registration.
            let _ = unsafe { service.unregisterAndReturnError() };
        }
        // SAFETY: the helper is embedded at the identifier's required bundle path.
        match unsafe { service.registerAndReturnError() } {
            Ok(()) => native_status(),
            Err(_) => match native_status() {
                NativeServiceStatus::RequiresApproval => NativeServiceStatus::RequiresApproval,
                NativeServiceStatus::Enabled => NativeServiceStatus::Enabled,
                _ => NativeServiceStatus::Failed,
            },
        }
    }

    pub fn open_login_items() {
        // SAFETY: this class method only asks System Settings to open its Login Items pane.
        unsafe { SMAppService::openSystemSettingsLoginItems() };
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::NativeServiceStatus;

    pub fn status(_: bool) -> NativeServiceStatus {
        NativeServiceStatus::Unavailable
    }

    pub fn register(_: bool) -> NativeServiceStatus {
        NativeServiceStatus::Unavailable
    }

    pub fn open_login_items() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_statuses_map_to_the_settings_contract() {
        use AlertServiceState as State;
        use NativeServiceStatus as Native;
        assert_eq!(map_service_status(Native::Enabled), State::Enabled);
        assert_eq!(
            map_service_status(Native::RequiresApproval),
            State::RequiresApproval
        );
        assert_eq!(map_service_status(Native::NotFound), State::NotFound);
        assert_eq!(map_service_status(Native::Failed), State::Failed);
        assert_eq!(map_service_status(Native::NotRegistered), State::Disabled);
        assert_eq!(map_service_status(Native::Unavailable), State::Unavailable);
    }
}
