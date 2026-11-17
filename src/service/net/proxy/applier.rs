//! Applying a saved proxy setting to the running process. Saving it in Settings is quick, but
//! applying it is not (the system's proxy is read, the secure storage may prompt), so it runs on a
//! worker. The queue lives here, not in the Settings host: the host is only driven while the
//! window is open, and a change saved just before closing it must still take effect.

use super::{ProxyConfig, apply};
use crate::lock::lock;
use crate::service::keychain::proxy_password::ProxyPasswordStore;
use crate::service::webhook::delivery::SecretStoreError;
use crate::service::work_queue;
use std::sync::{Arc, LazyLock, Mutex};

/// What the proxy's password is to become.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasswordChange {
    Keep,
    Set(String),
    Remove,
}

/// What an apply has to say to the page, kept until the page reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyReport {
    /// A new password reached the secure storage.
    PasswordSaved,
    /// The storage refused it: it works until the app quits.
    PasswordNotKept(SecretStoreError),
}

/// Called after each apply, to repaint whatever shows the proxy.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Queue {
    /// The latest setting not yet applied, with what to do to the password.
    pending: Option<(ProxyConfig, PasswordChange)>,
    /// Whether a worker is draining `pending`; it clears this itself, in the lock that finds the
    /// queue empty, so a save can never be left waiting for a worker that is just leaving.
    running: bool,
    report: Option<ApplyReport>,
}

impl Queue {
    /// Queues `config`. A later save that keeps the password does not undo an earlier one that
    /// changed it and was not applied yet.
    fn push(&mut self, config: ProxyConfig, change: PasswordChange) {
        let change = match (self.pending.take(), change) {
            (Some((_, earlier)), PasswordChange::Keep) => earlier,
            (_, change) => change,
        };
        self.pending = Some((config, change));
    }
}

pub struct Applier {
    queue: Mutex<Queue>,
    passwords: &'static ProxyPasswordStore,
}

static SHARED: LazyLock<Applier> = LazyLock::new(|| Applier::new(ProxyPasswordStore::shared()));

/// Applies `config` and the password change on a worker, after any apply already running. Returns
/// at once.
pub fn submit(config: ProxyConfig, change: PasswordChange, wake: Wake) {
    SHARED.submit(config, change, wake);
}

/// The result of an apply the page has not shown yet.
pub fn take_report() -> Option<ApplyReport> {
    lock(&SHARED.queue).report.take()
}

/// Clears `running` if the worker dies in the middle of a job, so the next save starts another
/// (what was queued waits for it). A worker that leaves normally has cleared it already, in the
/// lock that found the queue empty; clearing it again here could erase the flag of a successor
/// that a save started in between, and the next save would then start a second worker beside it.
struct Running<'a>(&'a Mutex<Queue>);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            lock(self.0).running = false;
        }
    }
}

impl Applier {
    fn new(passwords: &'static ProxyPasswordStore) -> Self {
        Self {
            queue: Mutex::new(Queue::default()),
            passwords,
        }
    }

    fn submit(&'static self, config: ProxyConfig, change: PasswordChange, wake: Wake) {
        let start = {
            let mut queue = lock(&self.queue);
            queue.push(config, change);
            !std::mem::replace(&mut queue.running, true)
        };
        if start {
            // The secure storage may wait on a prompt, so not one of the general workers.
            work_queue::SNAPSHOTS.submit(move || self.drain(&wake));
        }
    }

    fn drain(&self, wake: &Wake) {
        let _running = Running(&self.queue);
        loop {
            let next = {
                let mut queue = lock(&self.queue);
                let next = queue.pending.take();
                if next.is_none() {
                    queue.running = false;
                }
                next
            };
            let Some((config, change)) = next else {
                return;
            };
            let report = self.apply_one(config, &change);
            if report.is_some() {
                lock(&self.queue).report = report;
            }
            wake();
        }
    }

    fn apply_one(&self, config: ProxyConfig, change: &PasswordChange) -> Option<ApplyReport> {
        let report = match change {
            PasswordChange::Keep => None,
            PasswordChange::Set(password) => Some(match self.passwords.save(password) {
                Ok(()) => ApplyReport::PasswordSaved,
                Err(error) => ApplyReport::PasswordNotKept(error),
            }),
            PasswordChange::Remove => self
                .passwords
                .remove()
                .err()
                .map(ApplyReport::PasswordNotKept),
        };
        let password = config
            .needs_password()
            .then(|| self.passwords.read())
            .flatten();
        apply(config, password);
        report
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{AppliedProxy, manual};
    use super::super::{ProxyMode, snapshot, status};
    use super::*;
    use crate::service::keychain::api_keys::{KeyBackend, MemoryBackend};
    use crate::service::keychain::webhook_secrets::Scope;
    use std::sync::mpsc;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    fn config(mode: ProxyMode) -> ProxyConfig {
        ProxyConfig {
            mode,
            username: "me".to_owned(),
            ..manual("127.0.0.1", Some(7890))
        }
    }

    /// An applier over its own password store, so no test touches the real one.
    fn applier(backend: impl KeyBackend + 'static) -> &'static Applier {
        let passwords = Box::leak(Box::new(ProxyPasswordStore::with_backend(Box::new(
            backend,
        ))));
        Box::leak(Box::new(Applier::new(passwords)))
    }

    /// A wake that reports each finished apply on a channel.
    fn wake() -> (Wake, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let wake: Wake = Arc::new(move || {
            let _ = lock(&sender).send(());
        });
        (wake, receiver)
    }

    #[test]
    fn a_later_save_that_keeps_the_password_keeps_the_earlier_change() {
        let mut queue = Queue::default();
        queue.push(
            config(ProxyMode::Manual),
            PasswordChange::Set("one".to_owned()),
        );
        queue.push(config(ProxyMode::Off), PasswordChange::Keep);
        let (latest, change) = queue.pending.clone().unwrap();
        assert_eq!(latest.mode, ProxyMode::Off, "the latest setting wins");
        assert_eq!(change, PasswordChange::Set("one".to_owned()));

        queue.push(config(ProxyMode::Manual), PasswordChange::Remove);
        queue.push(config(ProxyMode::Manual), PasswordChange::Keep);
        assert_eq!(queue.pending.unwrap().1, PasswordChange::Remove);
    }

    /// The point of the queue living here: no Settings window, host or frame is involved, yet the
    /// saved setting and its password take effect.
    #[test]
    fn a_save_takes_effect_with_no_settings_window_open() {
        let _applied = AppliedProxy::new(config(ProxyMode::Off));
        let applier = applier(MemoryBackend::default());
        let (wake, done) = wake();
        applier.submit(
            config(ProxyMode::Manual),
            PasswordChange::Set("pw".to_owned()),
            wake,
        );
        done.recv_timeout(WAIT).expect("the worker applied it");
        assert_eq!(status().mode, ProxyMode::Manual);
        assert_eq!(
            snapshot().proxy.unwrap().spec.password.as_deref(),
            Some("pw")
        );
        assert_eq!(
            lock(&applier.queue).report.take(),
            Some(ApplyReport::PasswordSaved)
        );
    }

    /// A worker that leaves normally has already cleared `running` in the lock that found the queue
    /// empty. If a save started a successor in the moment after, the leaving worker must not clear
    /// the successor's flag: the next save would start a second worker beside it, and the two could
    /// apply settings out of order.
    #[test]
    fn a_leaving_worker_does_not_clear_its_successors_flag() {
        let queue = Mutex::new(Queue::default());
        let leaving = Running(&queue);
        lock(&queue).running = true; // the successor
        drop(leaving);
        assert!(lock(&queue).running);
    }

    /// A worker that dies in the middle of a job does clear it, so the next save starts another.
    #[test]
    fn a_worker_that_panics_frees_the_next_save_to_start_one() {
        let queue = Mutex::new(Queue {
            running: true,
            ..Queue::default()
        });
        let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _running = Running(&queue);
            panic!("the storage broke");
        }));
        assert!(died.is_err());
        assert!(!lock(&queue).running);
    }

    /// A password backend that stops inside `write` until released, to hold an apply open.
    struct Gate {
        inner: MemoryBackend,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl KeyBackend for Gate {
        fn read(&self, scope: Scope, account: &str) -> Option<String> {
            self.inner.read(scope, account)
        }
        fn exists(&self, scope: Scope, account: &str) -> bool {
            self.inner.exists(scope, account)
        }
        fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError> {
            let _ = self.entered.send(());
            let _ = lock(&self.release).recv_timeout(WAIT);
            self.inner.write(scope, account, value)
        }
        fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError> {
            self.inner.delete(scope, account)
        }
    }

    /// A save that arrives while an apply is running is applied after it, by the same worker, and
    /// the setting that ends up in force is the later one.
    #[test]
    fn a_save_made_during_an_apply_is_applied_after_it() {
        let _applied = AppliedProxy::new(config(ProxyMode::Off));
        let (entered, inside) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let applier = applier(Gate {
            inner: MemoryBackend::default(),
            entered,
            release: Mutex::new(held),
        });
        let (wake, done) = wake();
        applier.submit(
            config(ProxyMode::Manual),
            PasswordChange::Set("pw".to_owned()),
            Arc::clone(&wake),
        );
        inside
            .recv_timeout(WAIT)
            .expect("the first apply is under way");
        applier.submit(config(ProxyMode::Off), PasswordChange::Keep, wake);
        assert!(lock(&applier.queue).running);
        release.send(()).unwrap();
        done.recv_timeout(WAIT).unwrap();
        done.recv_timeout(WAIT).unwrap();
        assert_eq!(status().mode, ProxyMode::Off, "the later save won");
        assert!(!lock(&applier.queue).running, "the worker left");
    }
}
