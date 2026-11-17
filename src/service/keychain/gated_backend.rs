//! A key backend for tests that can be held inside a call, to show that nothing else waits for
//! it: a Keychain write or an unlock prompt takes as long as the user does.

use crate::service::keychain::api_keys::{KeyBackend, MemoryBackend};
use crate::service::keychain::webhook_secrets::Scope;
use crate::service::webhook::delivery::SecretStoreError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A held call is let go after this, so a test that forgets to release it ends instead of hanging.
const HOLD: Duration = Duration::from_secs(5);

pub(crate) struct GatedBackend {
    inner: MemoryBackend,
    hold_reads: bool,
    hold_writes: bool,
    entered: Mutex<Sender<()>>,
    release: Mutex<Receiver<()>>,
    reads: Arc<AtomicUsize>,
}

/// The test's hand on a [`GatedBackend`].
pub(crate) struct Gate {
    entered: Receiver<()>,
    release: Sender<()>,
    /// How many times the storage was asked to read.
    pub reads: Arc<AtomicUsize>,
}

/// A backend that holds every read (`hold_reads`) and/or write (`hold_writes`) until the gate
/// releases it, and the gate that does so.
pub(crate) fn gated(hold_reads: bool, hold_writes: bool) -> (GatedBackend, Gate) {
    let (entered, entered_rx) = channel();
    let (release_tx, release) = channel();
    let reads = Arc::new(AtomicUsize::new(0));
    let backend = GatedBackend {
        inner: MemoryBackend::default(),
        hold_reads,
        hold_writes,
        entered: Mutex::new(entered),
        release: Mutex::new(release),
        reads: Arc::clone(&reads),
    };
    let gate = Gate {
        entered: entered_rx,
        release: release_tx,
        reads,
    };
    (backend, gate)
}

impl Gate {
    /// Waits until a call is held inside the backend.
    pub(crate) fn wait_until_held(&self) {
        self.entered
            .recv_timeout(HOLD)
            .expect("a call should be held inside the storage");
    }

    /// Lets one held call go.
    pub(crate) fn release(&self) {
        let _ = self.release.send(());
    }
}

impl GatedBackend {
    fn hold(&self) {
        let _ = crate::lock::lock(&self.entered).send(());
        let _ = crate::lock::lock(&self.release).recv_timeout(HOLD);
    }
}

impl KeyBackend for GatedBackend {
    fn read(&self, scope: Scope, account: &str) -> Option<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.hold_reads {
            self.hold();
        }
        self.inner.read(scope, account)
    }
    fn exists(&self, scope: Scope, account: &str) -> bool {
        self.inner.exists(scope, account)
    }
    fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError> {
        if self.hold_writes {
            self.hold();
        }
        self.inner.write(scope, account, value)
    }
    fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError> {
        self.inner.delete(scope, account)
    }
}
