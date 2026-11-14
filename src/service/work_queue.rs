//! Small shared pools for blocking, replaceable background requests, and [`Pending`], the handle
//! the frame loop keeps for one answer from them.
//!
//! Work started from a frame (a search, a test connection, a file dialog) goes through
//! [`WorkQueue::start`], [`start`] or [`start_for`] and is polled each frame through the
//! [`Pending`] it returns; the job asks for a repaint when it ends, so the answer is drawn at once.

use crate::lock::lock;
use eframe::egui;
use std::sync::{Arc, Mutex, OnceLock, mpsc};

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed set of worker threads, started on first use, fed from one queue.
pub struct WorkQueue {
    workers: usize,
    sender: OnceLock<mpsc::Sender<Job>>,
}

impl WorkQueue {
    pub const fn new(workers: usize) -> Self {
        Self {
            workers,
            sender: OnceLock::new(),
        }
    }

    pub fn submit(&self, job: impl FnOnce() + Send + 'static) {
        let sender = self.sender.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Job>();
            let receiver = Arc::new(Mutex::new(receiver));
            for _ in 0..self.workers {
                let receiver = receiver.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = lock(&receiver).recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                });
            }
            sender
        });
        let _ = sender.send(Box::new(job));
    }

    /// Runs `job` on these workers and returns the handle its answer arrives on. When the job ends
    /// (answered, panicked, or dropped before it ran) `repaint` is asked for a frame, so a view
    /// that polls the handle sees the end without a timer of its own.
    pub fn start<T: Send + 'static>(
        &self,
        repaint: Option<&egui::Context>,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Pending<T> {
        let (sender, receiver) = mpsc::channel();
        let finish = Finish {
            sender: Some(sender),
            repaint: repaint.cloned(),
        };
        self.submit(move || {
            // Caught so a panicking job never takes a shared worker down with it; the panic hook
            // has already reported it.
            let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).ok();
            finish.answer(answer);
        });
        Pending {
            state: State::Running(receiver),
        }
    }
}

/// The job's end of a [`Pending`]: sends the answer, then (also when it is dropped unanswered, by
/// a queue that never ran it) closes the channel and asks for the repaint, in that order, so the
/// frame it wakes sees the end.
struct Finish<T> {
    sender: Option<mpsc::Sender<T>>,
    repaint: Option<egui::Context>,
}

impl<T> Finish<T> {
    fn answer(self, answer: Option<T>) {
        if let (Some(sender), Some(answer)) = (&self.sender, answer) {
            // The handle may be gone: nobody wants the answer any more.
            let _ = sender.send(answer);
        }
    }
}

impl<T> Drop for Finish<T> {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(ctx) = &self.repaint {
            ctx.request_repaint();
        }
    }
}

/// One answer from a job started on a [`WorkQueue`], polled from the frame loop.
///
/// A job that panicked or was dropped before it ran ends without an answer: [`Pending::poll`] then
/// returns `None` and [`Pending::is_pending`] turns false, and [`Pending::poll_or`] answers in its
/// place. Dropping the handle never blocks and does not stop the job: it runs to the end and its
/// answer is thrown away.
pub struct Pending<T> {
    state: State<T>,
}

enum State<T> {
    Running(mpsc::Receiver<T>),
    /// Ended without an answer, not yet reported by [`Pending::poll_or`].
    Lost,
    /// Answered (and the answer taken), or the loss reported.
    Done,
}

impl<T> Pending<T> {
    /// The answer, the one time it is there; `None` while the job runs and after.
    pub fn poll(&mut self) -> Option<T> {
        let State::Running(receiver) = &self.state else {
            return None;
        };
        match receiver.try_recv() {
            Ok(answer) => {
                self.state = State::Done;
                Some(answer)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.state = State::Lost;
                None
            }
        }
    }

    /// Like [`Self::poll`], but a job that ended without an answer answers `lost()`, once.
    pub fn poll_or(&mut self, lost: impl FnOnce() -> T) -> Option<T> {
        let answer = self.poll();
        if matches!(self.state, State::Lost) {
            self.state = State::Done;
            return Some(lost());
        }
        answer
    }

    /// Whether the job may still answer.
    pub fn is_pending(&self) -> bool {
        matches!(self.state, State::Running(_))
    }
}

/// Several jobs answered together, such as one search asked of several providers.
pub struct PendingAll<T> {
    jobs: Vec<(Pending<T>, Option<T>)>,
    taken: bool,
}

impl<T> PendingAll<T> {
    pub fn new(jobs: impl IntoIterator<Item = Pending<T>>) -> Self {
        Self {
            jobs: jobs.into_iter().map(|job| (job, None)).collect(),
            taken: false,
        }
    }

    /// Once every job has ended, the answers in the order the jobs were given (`None` for a job
    /// that ended without one); `None` before that, and after they were taken.
    pub fn poll(&mut self) -> Option<Vec<Option<T>>> {
        if self.taken {
            return None;
        }
        for (job, answer) in &mut self.jobs {
            if let Some(arrived) = job.poll() {
                *answer = Some(arrived);
            }
        }
        if self.jobs.iter().any(|(job, _)| job.is_pending()) {
            return None;
        }
        self.taken = true;
        Some(
            std::mem::take(&mut self.jobs)
                .into_iter()
                .map(|(_, answer)| answer)
                .collect(),
        )
    }
}

#[cfg(test)]
impl<T> Pending<T> {
    /// A handle answered by whatever a test sends on the other end of `receiver`.
    pub fn from_receiver(receiver: mpsc::Receiver<T>) -> Self {
        Self {
            state: State::Running(receiver),
        }
    }

    /// Blocks until the answer comes, for a test with no frame loop: `None` when the job ended
    /// without one. Gives up after eight seconds.
    pub fn wait(mut self) -> Option<T> {
        let State::Running(receiver) = std::mem::replace(&mut self.state, State::Done) else {
            return None;
        };
        match receiver.recv_timeout(std::time::Duration::from_secs(8)) {
            Ok(answer) => Some(answer),
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("the job never answered"),
        }
    }
}

static QUEUE: WorkQueue = WorkQueue::new(4);
/// Image downloads get their own workers, so a screenful of new logos never queues in front of a
/// chart's candles.
pub static IMAGES: WorkQueue = WorkQueue::new(4);

/// Pictures for drawings are read and decoded on workers of their own, so a chart with a few large
/// images never holds up a chart's candles.
pub static ASSETS: WorkQueue = WorkQueue::new(2);

/// Link previews are fetched one at a time and only when the user asks, from a pool of their own.
pub static LINKS: WorkQueue = WorkQueue::new(1);

/// The updater's release check and download get one worker of their own: a download can run for
/// minutes and must never hold one of the general workers that fetch candles and alert quotes.
pub static UPDATES: WorkQueue = WorkQueue::new(1);

/// Chart snapshots, file dialogs and tests that read a saved key wait on things that take as long
/// as the user or the network does (a save panel, a Keychain prompt, an upload), so they never
/// borrow a general worker.
pub static SNAPSHOTS: WorkQueue = WorkQueue::new(2);

/// Rate-limited providers get a worker each: a request sleeping until its slot comes (seconds, for
/// CoinGecko and GeckoTerminal) must not hold one of the general workers that fetch other charts'
/// candles. One worker matches the limiter, which lets one request through at a time anyway.
static COINGECKO_LANE: WorkQueue = WorkQueue::new(1);

/// Searches the user is typing into run apart from candle fetches, so a picker never waits behind
/// a busy chart; a rate-limited provider's search still takes that provider's lane.
static SEARCHES: WorkQueue = WorkQueue::new(4);
static GECKOTERMINAL_LANE: WorkQueue = WorkQueue::new(1);

pub fn submit(job: impl FnOnce() + Send + 'static) {
    QUEUE.submit(job);
}

/// Runs `job` on the general workers; see [`WorkQueue::start`].
pub fn start<T: Send + 'static>(
    repaint: Option<&egui::Context>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Pending<T> {
    QUEUE.start(repaint, job)
}

/// The workers that suit requests to `source`'s provider.
fn lane(source: crate::model::DataSource) -> &'static WorkQueue {
    use crate::model::DataSource;
    match source {
        DataSource::CoinGecko => &COINGECKO_LANE,
        DataSource::DexScreener => &GECKOTERMINAL_LANE,
        _ => &QUEUE,
    }
}

/// Runs a request to `source`'s provider on the workers that suit it.
pub fn submit_for(source: crate::model::DataSource, job: impl FnOnce() + Send + 'static) {
    lane(source).submit(job);
}

/// Runs a request to `source`'s provider on the workers that suit it, as [`WorkQueue::start`].
pub fn start_for<T: Send + 'static>(
    source: crate::model::DataSource,
    repaint: Option<&egui::Context>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Pending<T> {
    lane(source).start(repaint, job)
}

/// Runs a search of `source`'s provider for a picker, as [`WorkQueue::start`]: on that provider's
/// lane when it has one, otherwise on the search workers.
pub fn start_search<T: Send + 'static>(
    source: crate::model::DataSource,
    repaint: Option<&egui::Context>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Pending<T> {
    let queue = match lane(source) {
        queue if std::ptr::eq(queue, &QUEUE) => &SEARCHES,
        queue => queue,
    };
    queue.start(repaint, job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// A context that counts the repaints asked of it.
    fn counting_context() -> (egui::Context, Arc<AtomicUsize>) {
        let ctx = egui::Context::default();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&repaints);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        (ctx, repaints)
    }

    fn poll_until<T>(pending: &mut Pending<T>) -> Option<T> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(answer) = pending.poll() {
                return Some(answer);
            }
            if !pending.is_pending() {
                return None;
            }
            assert!(Instant::now() < deadline, "the job never ended");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_finished_job_answers_once_and_asks_for_a_repaint() {
        let queue = WorkQueue::new(1);
        let (ctx, repaints) = counting_context();
        let mut pending = queue.start(Some(&ctx), || 7);
        assert_eq!(poll_until(&mut pending), Some(7));
        assert!(!pending.is_pending());
        assert_eq!(pending.poll(), None, "the answer is taken once");
        assert_eq!(pending.poll_or(|| 0), None, "an answered job is not lost");
        assert!(repaints.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn a_job_still_running_is_pending() {
        let queue = WorkQueue::new(1);
        let (release, released) = mpsc::channel::<()>();
        let mut pending = queue.start(None, move || released.recv().is_ok());
        assert_eq!(pending.poll(), None);
        assert!(pending.is_pending());
        release.send(()).unwrap();
        assert_eq!(poll_until(&mut pending), Some(true));
    }

    #[test]
    fn a_panicking_job_ends_without_an_answer_and_the_worker_survives() {
        let queue = WorkQueue::new(1);
        let (ctx, repaints) = counting_context();
        let mut lost = queue.start(Some(&ctx), || -> u8 { panic!("the job failed") });
        assert_eq!(poll_until(&mut lost), None);
        assert!(!lost.is_pending());
        assert_eq!(lost.poll_or(|| 9), Some(9), "the loss is answered once");
        assert_eq!(lost.poll_or(|| 9), None);
        assert!(
            repaints.load(Ordering::SeqCst) >= 1,
            "a lost job repaints too"
        );
        // The only worker is still there to run the next job.
        assert_eq!(queue.start(None, || 3).wait(), Some(3));
    }

    #[test]
    fn a_lost_job_is_reported_by_poll_or_on_the_frame_it_is_seen() {
        let (sender, receiver) = mpsc::channel::<u8>();
        let mut pending = Pending::from_receiver(receiver);
        assert_eq!(pending.poll_or(|| 1), None);
        drop(sender);
        assert_eq!(pending.poll_or(|| 1), Some(1));
        assert!(!pending.is_pending());
    }

    #[test]
    fn jobs_answered_together_wait_for_the_last_and_keep_their_order() {
        let queue = WorkQueue::new(2);
        let (release, released) = mpsc::channel::<()>();
        let mut all = PendingAll::new([
            queue.start(None, move || released.recv().map(|()| 1).unwrap_or(0)),
            queue.start(None, || -> u8 { panic!("this one fails") }),
            queue.start(None, || 3),
        ]);
        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            assert_eq!(all.poll(), None, "the first is still running");
            std::thread::sleep(Duration::from_millis(5));
        }
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let answers = loop {
            if let Some(answers) = all.poll() {
                break answers;
            }
            assert!(Instant::now() < deadline, "the jobs never ended");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(answers, [Some(1), None, Some(3)]);
        assert_eq!(all.poll(), None, "the answers are taken once");
    }

    #[test]
    fn dropping_the_handle_neither_blocks_nor_stops_the_job() {
        let queue = WorkQueue::new(1);
        let (release, released) = mpsc::channel::<()>();
        let (ran, did_run) = mpsc::channel::<()>();
        let pending = queue.start(None, move || {
            let _ = released.recv();
            let _ = ran.send(());
        });
        let dropped = Instant::now();
        drop(pending);
        assert!(dropped.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        did_run
            .recv_timeout(Duration::from_secs(5))
            .expect("the job ran to its end");
    }
}
