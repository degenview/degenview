//! What a socket feed that lives on a viewer's lease is made of, shared by the liquidation and the
//! taker-flow feeds.
//!
//! A [`Stream`] runs one thread per `net::ws::Session` (each reconnects with backoff). The viewer
//! renews a [`Lease`] each frame it draws. A watchdog watches the lease: when it lapses (the panel
//! closed, a tab in the background) it shuts every thread down together, whether it is reading, waiting
//! to reconnect or still looking something up, so nothing keeps retrying for a viewer that left.
//! Dropping the stream closes them at once.

mod retry;

pub use retry::retry_until_shutdown;

use crate::clock::now_ms;
use crate::service::net::ws::{Frame, Session};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

/// How often a burst of messages asks for a repaint at most.
const REPAINT_EVERY_MS: i64 = 250;
/// How often the watchdog looks at the lease.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// Asks the window to draw a frame.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// The time until which the viewer wants the feed, in milliseconds since the epoch.
#[derive(Clone, Default)]
pub struct Lease(Arc<AtomicI64>);

impl Lease {
    /// Wanted for `span` from now.
    pub fn renew(&self, span: Duration) {
        self.0
            .store(now_ms() + span.as_millis() as i64, Ordering::Release);
    }

    fn is_held(&self) -> bool {
        now_ms() < self.0.load(Ordering::Acquire)
    }
}

/// What the threads of one stream share.
pub struct Worker {
    lease: Lease,
    repaint: Repaint,
    last_repaint: AtomicI64,
    shutdown: AtomicBool,
    /// How many threads were started, and how many still run.
    started: AtomicUsize,
    running: AtomicUsize,
}

impl Worker {
    /// Connects `session` and hands each text message to `on_text` until the stream is dropped or
    /// the lease lapses. `on_text` answers whether to keep reading.
    pub fn run(&self, session: &Session, mut on_text: impl FnMut(&str) -> bool) {
        session.run(&self.shutdown, |text| self.is_wanted() && on_text(text));
    }

    /// Like [`Self::run`], and `on_link` hears when the socket is up or down; see
    /// [`Session::run_linked`].
    pub fn run_linked(
        &self,
        session: &Session,
        on_link: impl FnMut(bool),
        mut on_text: impl FnMut(&str) -> bool,
    ) {
        session.run_linked(&self.shutdown, on_link, |frame| match frame {
            Frame::Text(text) => self.is_wanted() && on_text(text),
            Frame::Binary(_) => true,
        });
    }

    /// Calls `attempt` until it answers, every `every`; `None` when the stream was shut down
    /// first. See [`retry_until_shutdown`].
    pub fn retry<T>(&self, every: Duration, attempt: impl FnMut() -> Option<T>) -> Option<T> {
        retry_until_shutdown(&self.shutdown, every, attempt)
    }

    /// Whether anybody still wants the feed.
    pub fn is_wanted(&self) -> bool {
        self.lease.is_held()
    }

    /// Set once the stream is dropped; long waits poll it.
    pub fn shutdown(&self) -> &AtomicBool {
        &self.shutdown
    }

    /// Asks for a repaint, at most every 250 ms however many messages arrive.
    pub fn wake(&self) {
        let now = now_ms();
        if now - self.last_repaint.load(Ordering::Relaxed) >= REPAINT_EVERY_MS {
            self.last_repaint.store(now, Ordering::Relaxed);
            (self.repaint)();
        }
    }
}

/// The running threads of one feed; dropping it closes their sockets.
pub struct Stream {
    worker: Arc<Worker>,
}

impl Stream {
    /// A stream with no threads yet. The lease must be held (renewed) first, or the watchdog ends
    /// it at once.
    pub fn new(lease: Lease, repaint: Repaint) -> Self {
        let worker = Arc::new(Worker {
            lease,
            repaint,
            last_repaint: AtomicI64::new(0),
            shutdown: AtomicBool::new(false),
            started: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
        });
        let watched = Arc::clone(&worker);
        std::thread::spawn(move || {
            while !watched.shutdown.load(Ordering::Acquire) {
                if !watched.lease.is_held() {
                    watched.shutdown.store(true, Ordering::Release);
                    return;
                }
                std::thread::sleep(WATCH_EVERY);
            }
        });
        Self { worker }
    }

    /// Runs `work` on a thread of its own.
    pub fn spawn(&self, work: impl FnOnce(&Worker) + Send + 'static) {
        self.worker.started.fetch_add(1, Ordering::AcqRel);
        self.worker.running.fetch_add(1, Ordering::AcqRel);
        let worker = Arc::clone(&self.worker);
        std::thread::spawn(move || {
            work(&worker);
            worker.running.fetch_sub(1, Ordering::AcqRel);
        });
    }

    /// Whether every thread started still runs. One that ended (the lease lapsed, or it gave up)
    /// leaves the stream short of an exchange, so it counts as ended and a new stream is needed:
    /// never one that serves some of the feeds it was made for.
    pub fn is_running(&self) -> bool {
        let started = self.worker.started.load(Ordering::Acquire);
        started > 0 && self.worker.running.load(Ordering::Acquire) == started
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.worker.shutdown.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(lease: &Lease, repaint: Repaint) -> Stream {
        Stream::new(lease.clone(), repaint)
    }

    #[test]
    fn a_feed_is_wanted_only_while_its_lease_is_held() {
        let lease = Lease::default();
        let stream = stream(&lease, Arc::new(|| {}));
        assert!(!stream.worker.is_wanted(), "no lease yet");
        lease.renew(Duration::from_secs(5));
        assert!(stream.worker.is_wanted());
        lease.renew(Duration::ZERO);
        assert!(!stream.worker.is_wanted());
    }

    #[test]
    fn a_burst_asks_for_one_repaint() {
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked);
        let stream = stream(
            &Lease::default(),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        );
        for _ in 0..50 {
            stream.worker.wake();
        }
        assert_eq!(asked.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn dropping_the_stream_shuts_its_threads_down() {
        let lease = Lease::default();
        lease.renew(Duration::from_secs(60));
        let stream = stream(&lease, Arc::new(|| {}));
        let worker = Arc::clone(&stream.worker);
        assert!(!worker.shutdown().load(Ordering::Acquire));
        drop(stream);
        assert!(worker.shutdown().load(Ordering::Acquire));
    }

    #[test]
    fn a_lapsed_lease_shuts_every_thread_down_without_a_message() {
        let lease = Lease::default();
        lease.renew(Duration::from_millis(100));
        let stream = stream(&lease, Arc::new(|| {}));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !stream.worker.shutdown().load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "the watchdog never fired"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_renewed_lease_keeps_the_threads() {
        let lease = Lease::default();
        let stream = stream(&lease, Arc::new(|| {}));
        for _ in 0..6 {
            lease.renew(Duration::from_millis(400));
            std::thread::sleep(Duration::from_millis(150));
        }
        assert!(!stream.worker.shutdown().load(Ordering::Acquire));
    }

    #[test]
    fn a_stream_whose_one_thread_ended_is_not_running() {
        let lease = Lease::default();
        lease.renew(Duration::from_secs(60));
        let stream = stream(&lease, Arc::new(|| {}));
        assert!(!stream.is_running(), "nothing started");
        let (release_one, wait_one) = std::sync::mpsc::channel::<()>();
        let (release_two, wait_two) = std::sync::mpsc::channel::<()>();
        let (started, running) = std::sync::mpsc::channel::<()>();
        for wait in [wait_one, wait_two] {
            let started = started.clone();
            stream.spawn(move |_| {
                started.send(()).unwrap();
                let _ = wait.recv();
            });
        }
        running.recv().unwrap();
        running.recv().unwrap();
        assert!(stream.is_running());
        release_one.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while stream.is_running() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        // The other thread still runs, and the stream is still not to be trusted.
        assert!(stream.worker.running.load(Ordering::Acquire) == 1);
        assert!(!stream.is_running());
        release_two.send(()).unwrap();
    }
}
