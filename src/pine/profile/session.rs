//! A profiling session: the script running once on a thread of its own, and the handle the UI keeps.
//!
//! The thread owns a complete [`ExecutionController`] over a frozen copy of the bars, so a session
//! shares nothing with the chart's own run: no alerts, no orders, no preview state. The alerts and
//! outputs the run produces are dropped, and the broker a strategy trades on is the session's own.
//! The UI learns of progress from a counter it reads, and of the end from one message; the thread
//! never waits for the UI and the UI never waits for the thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Weak};
use std::time::Instant;

use crate::model::Kline;
use crate::pine::compiler::LibraryResolver;
use crate::pine::execution::ExecutionController;
use crate::pine::program::{ChartTheme, InputValue, SymbolInfo};
use crate::pine::runtime_support::SecurityDataProvider;
use crate::pine::{Diagnostic, DiagnosticCategory, Limits, SourceRange};

use super::report::{Completeness, Interruption, ProfileReport};
use super::wait::WaitMeter;

pub struct ProfileConfig {
    pub source: String,
    pub inputs: HashMap<String, InputValue>,
    pub symbol: SymbolInfo,
    /// The bars as the script receives them, frozen: later ticks and refetches of the chart do not
    /// reach a running session.
    pub bars: Vec<Kline>,
    pub now_ms: i64,
    pub visible_range: Option<(i64, i64)>,
    pub limits: Limits,
    pub theme: Option<ChartTheme>,
    pub bar_seconds: Option<f64>,
    pub libraries: Box<dyn LibraryResolver + Send>,
    pub security_data: Option<Box<dyn SecurityDataProvider>>,
}

impl ProfileConfig {
    pub fn new(
        source: impl Into<String>,
        symbol: SymbolInfo,
        bars: Vec<Kline>,
        now_ms: i64,
    ) -> Self {
        struct NoLibraries;
        impl LibraryResolver for NoLibraries {
            fn source_for_library(&self, _: &str) -> Option<String> {
                None
            }
        }
        Self {
            source: source.into(),
            inputs: HashMap::new(),
            symbol,
            bars,
            now_ms,
            visible_range: None,
            limits: Limits::default(),
            theme: None,
            bar_seconds: None,
            libraries: Box::new(NoLibraries),
            security_data: None,
        }
    }
}

#[derive(Debug)]
pub enum StartError {
    NoBars,
    Thread(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBars => f.write_str("There are no bars to run the script on yet."),
            Self::Thread(error) => write!(f, "Could not start the profiler: {error}"),
        }
    }
}

impl std::error::Error for StartError {}

/// How a session ended.
#[derive(Debug)]
pub struct ProfileOutcome {
    /// The profile, complete or partial; or why the script could not be run at all (it does not
    /// compile).
    pub result: Result<ProfileReport, Diagnostic>,
    /// How long compiling the script took, which no part of the profile includes.
    pub compile_ns: u64,
}

/// The UI's end of a session.
pub struct ProfileSession {
    outcome: Receiver<ProfileOutcome>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    total_bars: u64,
    finished: bool,
    alive: Weak<()>,
}

impl ProfileSession {
    /// Starts the script on a new thread. `wake` is called from that thread when it ends, to ask the
    /// UI for a frame.
    pub fn start(
        config: ProfileConfig,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, StartError> {
        if config.bars.is_empty() {
            return Err(StartError::NoBars);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let total_bars = config.bars.len() as u64;
        let (sender, outcome) = mpsc::sync_channel(1);
        let alive = Arc::new(());
        let liveness = Arc::downgrade(&alive);
        let thread_cancel = Arc::clone(&cancel);
        let thread_progress = Arc::clone(&progress);
        std::thread::Builder::new()
            .name("pine-profile".to_owned())
            .spawn(move || {
                let _alive = alive;
                let outcome = run(config, thread_cancel, thread_progress);
                // One message, into a channel with room for it: this never waits. A receiver that
                // is gone (the session was dropped) is not an error.
                let _ = sender.send(outcome);
                wake();
            })
            .map_err(|error| StartError::Thread(error.to_string()))?;
        Ok(Self {
            outcome,
            cancel,
            progress,
            total_bars,
            finished: false,
            alive: liveness,
        })
    }

    /// Asks the run to stop at its next safepoint. What it measured so far still arrives, as a
    /// partial profile.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Bars finished, and bars there are.
    pub fn progress(&self) -> (u64, u64) {
        (self.progress.load(Ordering::Relaxed), self.total_bars)
    }

    /// Whether the outcome has been taken, or can no longer arrive.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The outcome, once, when the run has ended. Never blocks.
    pub fn poll(&mut self) -> Option<ProfileOutcome> {
        if self.finished {
            return None;
        }
        match self.outcome.try_recv() {
            Ok(outcome) => {
                self.finished = true;
                Some(outcome)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                // The thread ended without a word: it panicked before it could say so.
                self.finished = true;
                Some(ProfileOutcome {
                    result: Err(unexpected()),
                    compile_ns: 0,
                })
            }
        }
    }

    /// Whether the session's thread is still running.
    pub fn thread_alive(&self) -> bool {
        self.alive.strong_count() > 0
    }

    /// A handle that outlives the session, to watch its thread end.
    pub fn liveness(&self) -> Weak<()> {
        self.alive.clone()
    }
}

impl Drop for ProfileSession {
    fn drop(&mut self) {
        // The thread ends at its next safepoint.
        self.cancel.store(true, Ordering::Release);
    }
}

fn unexpected() -> Diagnostic {
    Diagnostic::error(
        "PINE9999",
        DiagnosticCategory::Runtime,
        "The profiler stopped unexpectedly.",
        SourceRange::ZERO,
    )
}

fn run(config: ProfileConfig, cancel: Arc<AtomicBool>, progress: Arc<AtomicU64>) -> ProfileOutcome {
    // Fetching another symbol's candles blocks; the time is measured and kept out of every region.
    let waited = Arc::new(AtomicU64::new(0));
    let security_data = config.security_data.map(|inner| {
        Box::new(WaitMeter::new(inner, Arc::clone(&waited))) as Box<dyn SecurityDataProvider>
    });
    let compile_started = Instant::now();
    let built = ExecutionController::new(
        &config.source,
        config.inputs,
        config.symbol,
        config.visible_range,
        config.limits,
        config.libraries.as_ref(),
        security_data,
        Some(Arc::clone(&cancel)),
    );
    let compile_ns = compile_started.elapsed().as_nanos() as u64;
    let mut controller = match built {
        Ok(controller) => controller,
        Err(error) => {
            return ProfileOutcome {
                result: Err(error),
                compile_ns,
            };
        }
    };
    if let Some(theme) = config.theme {
        controller = controller.with_theme(theme);
    }
    if let Some(seconds) = config.bar_seconds {
        controller = controller.with_bar_seconds(seconds);
    }
    controller = controller.with_profiler(progress, waited);
    let bars = config.bars;
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        controller.update(&bars, config.visible_range, config.now_ms)
    }));
    let completeness = match run {
        Ok(update) => match update.error {
            None => Completeness::Complete,
            Some(error) if error.code == "PINE8008" => {
                Completeness::Partial(Interruption::Cancelled)
            }
            Some(error) if error.code == "PINE8007" => {
                Completeness::Partial(Interruption::TimedOut)
            }
            Some(error) => Completeness::Partial(Interruption::Failed(error)),
        },
        Err(_) => Completeness::Partial(Interruption::Failed(unexpected())),
    };
    let result = controller
        .finish_profile(completeness)
        .ok_or_else(unexpected);
    ProfileOutcome { result, compile_ns }
}
