//! Small shared pools for blocking, replaceable background requests.

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
                        let job = receiver.lock().expect("work queue poisoned").recv();
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
}

static QUEUE: WorkQueue = WorkQueue::new(4);
/// Image downloads get their own workers, so a screenful of new logos never queues in front of a
/// chart's candles.
pub static IMAGES: WorkQueue = WorkQueue::new(4);

/// Pictures for drawings are read and decoded on workers of their own, so a chart with a few large
/// images never holds up a chart's candles.
pub static ASSETS: WorkQueue = WorkQueue::new(2);

/// The updater's release check and download get one worker of their own: a download can run for
/// minutes and must never hold one of the general workers that fetch candles and alert quotes.
pub static UPDATES: WorkQueue = WorkQueue::new(1);

/// Chart snapshots wait on things that take as long as the user or the network does (a save panel,
/// an upload), so they never borrow a general worker.
pub static SNAPSHOTS: WorkQueue = WorkQueue::new(2);

/// Rate-limited providers get a worker each: a request sleeping until its slot comes (seconds, for
/// CoinGecko and GeckoTerminal) must not hold one of the general workers that fetch other charts'
/// candles. One worker matches the limiter, which lets one request through at a time anyway.
static COINGECKO_LANE: WorkQueue = WorkQueue::new(1);
static GECKOTERMINAL_LANE: WorkQueue = WorkQueue::new(1);

pub fn submit(job: impl FnOnce() + Send + 'static) {
    QUEUE.submit(job);
}

/// Runs a request to `source`'s provider on the workers that suit it.
pub fn submit_for(source: crate::model::DataSource, job: impl FnOnce() + Send + 'static) {
    use crate::model::DataSource;
    match source {
        DataSource::CoinGecko => COINGECKO_LANE.submit(job),
        DataSource::DexScreener => GECKOTERMINAL_LANE.submit(job),
        _ => QUEUE.submit(job),
    }
}
