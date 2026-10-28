//! Native user-notification delivery. Bundled macOS builds use `UNUserNotificationCenter`;
//! tests, unbundled development binaries and other platforms use a no-op implementation.

use crate::model::alert::AlertNotificationPermission;
use crate::model::alert_notification::NotificationContent;
use std::sync::Arc;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const SOUND_NAME: &str = "Glass";

pub fn is_disabled() -> bool {
    cfg!(test)
        || std::env::var_os("DEGENVIEW_NO_NOTIFICATIONS").is_some()
        || std::env::var_os("DEGENVIEW_NO_KEYCHAIN").is_some()
}

pub trait Notifier: Send + Sync {
    fn deliver(&self, content: &NotificationContent) -> Result<(), String>;

    fn permission(&self) -> AlertNotificationPermission {
        AlertNotificationPermission::Unknown
    }

    fn request_authorization(&self) -> Result<AlertNotificationPermission, String> {
        Ok(self.permission())
    }
}

pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn deliver(&self, _: &NotificationContent) -> Result<(), String> {
        Ok(())
    }
}

pub fn default_notifier() -> Arc<dyn Notifier> {
    if is_disabled() || !platform::is_bundled() {
        Arc::new(NullNotifier)
    } else {
        Arc::new(platform::NativeNotifier::new())
    }
}

#[allow(dead_code)]
pub fn notify_async(
    notifier: Arc<dyn Notifier>,
    content: NotificationContent,
    done: impl FnOnce(&NotificationContent, Result<(), String>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let outcome = notifier.deliver(&content);
        done(&content, outcome);
    });
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread, define_class, msg_send};
    use objc2_foundation::{NSError, NSObject, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotification, UNNotificationPresentationOptions, UNNotificationRequest,
        UNNotificationResponse, UNNotificationSettings, UNNotificationSound,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::sync::{Mutex, Once};
    use std::time::Duration;

    const CALLBACK_TIMEOUT: Duration = Duration::from_secs(3);
    static INSTALL_DELEGATE: Once = Once::new();

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this class owns no ivars.
        #[unsafe(super = NSObject)]
        #[name = "DegenViewNotificationDelegate"]
        #[ivars = ()]
        struct NotificationDelegate;

        // SAFETY: NSObjectProtocol has no additional invariants.
        unsafe impl NSObjectProtocol for NotificationDelegate {}

        // SAFETY: both method signatures are copied from the generated protocol bindings.
        unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                completion.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                _response: &UNNotificationResponse,
                completion: &DynBlock<dyn Fn()>,
            ) {
                let _ = std::process::Command::new("/usr/bin/open")
                    .args(["-b", "es.nico.degenview"])
                    .spawn();
                completion.call(());
            }
        }
    );

    impl NotificationDelegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: NSObject's initializer has the generated Objective-C signature.
            unsafe { msg_send![super(this), init] }
        }
    }

    fn center() -> Retained<UNUserNotificationCenter> {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        INSTALL_DELEGATE.call_once(|| {
            let delegate = NotificationDelegate::new();
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // The center's delegate is weak. This process-lifetime object is intentionally kept
            // alive so notification activation can open the GUI from the background helper.
            let _ = Retained::into_raw(delegate);
        });
        center
    }

    fn error_message(error: *mut NSError) -> Option<String> {
        NonNull::new(error).map(|error| {
            // SAFETY: the callback promises a live NSError for the duration of the call.
            unsafe { error.as_ref() }.localizedDescription().to_string()
        })
    }

    fn map_permission(status: UNAuthorizationStatus) -> AlertNotificationPermission {
        match status {
            UNAuthorizationStatus::NotDetermined => AlertNotificationPermission::NotDetermined,
            UNAuthorizationStatus::Denied => AlertNotificationPermission::Denied,
            UNAuthorizationStatus::Authorized => AlertNotificationPermission::Authorized,
            UNAuthorizationStatus::Provisional => AlertNotificationPermission::Provisional,
            UNAuthorizationStatus::Ephemeral => AlertNotificationPermission::Ephemeral,
            _ => AlertNotificationPermission::Unknown,
        }
    }

    pub fn is_bundled() -> bool {
        std::env::current_exe().ok().is_some_and(|executable| {
            executable
                .ancestors()
                .any(|path| path.extension().is_some_and(|extension| extension == "app"))
        })
    }

    pub struct NativeNotifier {
        permission: Arc<Mutex<AlertNotificationPermission>>,
    }

    impl NativeNotifier {
        pub fn new() -> Self {
            let _ = center();
            let notifier = Self {
                permission: Arc::new(Mutex::new(AlertNotificationPermission::Unknown)),
            };
            notifier.refresh_permission();
            notifier
        }

        fn refresh_permission(&self) {
            let permission = Arc::clone(&self.permission);
            let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
                // SAFETY: UserNotifications passes a live settings object to this callback.
                let status = unsafe { settings.as_ref() }.authorizationStatus();
                if let Ok(mut cached) = permission.lock() {
                    *cached = map_permission(status);
                }
            });
            center().getNotificationSettingsWithCompletionHandler(&completion);
        }
    }

    impl Notifier for NativeNotifier {
        fn deliver(&self, value: &NotificationContent) -> Result<(), String> {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&value.title));
            content.setBody(&NSString::from_str(&value.body));
            content.setThreadIdentifier(&NSString::from_str("degenview-alerts"));
            if value.sound {
                let sound = UNNotificationSound::soundNamed(&NSString::from_str(SOUND_NAME));
                content.setSound(Some(&sound));
            }
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&value.identifier),
                &content,
                None,
            );
            let (sender, receiver) = mpsc::channel();
            let completion = RcBlock::new(move |error: *mut NSError| {
                let _ = sender.send(match error_message(error) {
                    Some(error) => Err(error),
                    None => Ok(()),
                });
            });
            center().addNotificationRequest_withCompletionHandler(&request, Some(&completion));
            receiver
                .recv_timeout(CALLBACK_TIMEOUT)
                .map_err(|_| "Notification Center did not answer".to_owned())?
        }

        fn permission(&self) -> AlertNotificationPermission {
            self.refresh_permission();
            self.permission
                .lock()
                .map(|value| *value)
                .unwrap_or_default()
        }

        fn request_authorization(&self) -> Result<AlertNotificationPermission, String> {
            let (sender, receiver) = mpsc::channel();
            let permission = Arc::clone(&self.permission);
            let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
                let result = match error_message(error) {
                    Some(error) => Err(error),
                    None if granted.as_bool() => Ok(AlertNotificationPermission::Authorized),
                    None => Ok(AlertNotificationPermission::Denied),
                };
                if let Ok(value) = &result
                    && let Ok(mut cached) = permission.lock()
                {
                    *cached = *value;
                }
                let _ = sender.send(result);
            });
            center().requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &completion,
            );
            receiver
                .recv_timeout(CALLBACK_TIMEOUT)
                .map_err(|_| "Notification authorization did not answer".to_owned())?
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub fn is_bundled() -> bool {
        false
    }

    pub struct NativeNotifier;

    impl NativeNotifier {
        pub fn new() -> Self {
            Self
        }
    }

    impl Notifier for NativeNotifier {
        fn deliver(&self, _: &NotificationContent) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn content(sound: bool) -> NotificationContent {
        NotificationContent {
            title: "BTC price alert".into(),
            body: "Reached $70,000.00".into(),
            identifier: "42".into(),
            sound,
            user_info: vec![("alertID".into(), "7".into())],
        }
    }

    #[test]
    fn under_test_the_default_notifier_posts_nothing() {
        assert!(is_disabled());
        assert!(default_notifier().deliver(&content(true)).is_ok());
    }

    struct Recording(Mutex<Vec<String>>, Result<(), String>);

    impl Notifier for Recording {
        fn deliver(&self, content: &NotificationContent) -> Result<(), String> {
            self.0.lock().unwrap().push(content.identifier.clone());
            self.1.clone()
        }
    }

    #[test]
    fn the_async_post_reports_back_with_the_outcome() {
        let ok = Arc::new(Recording(Mutex::new(Vec::new()), Ok(())));
        let (sender, receiver) = channel();
        let sent = sender.clone();
        notify_async(ok.clone(), content(false), move |content, outcome| {
            let _ = sent.send((content.identifier.clone(), outcome));
        });
        let (identifier, outcome) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((identifier.as_str(), outcome), ("42", Ok(())));
        assert_eq!(*ok.0.lock().unwrap(), vec!["42".to_owned()]);

        let failing = Arc::new(Recording(Mutex::new(Vec::new()), Err("denied".into())));
        notify_async(failing, content(false), move |_, outcome| {
            let _ = sender.send(("x".into(), outcome));
        });
        let (_, outcome) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(outcome, Err("denied".to_owned()));
    }

    #[test]
    fn stable_identifiers_and_sound_choice_are_kept_in_content() {
        assert_eq!(content(true).identifier, "42");
        assert!(content(true).sound);
        assert!(!content(false).sound);
        assert_eq!(SOUND_NAME, "Glass");
    }
}
