//! One applied script's background runner: a persistent `ExecutionController` fed the chart's
//! bars on the work queue, latest request first.

use super::super::KlineSecurityDataProvider;
use crate::model::Kline;
use crate::pine::compiler::program::{ChartTheme, InputValue, SymbolInfo, VisualOutput};
use crate::pine::logs::LogBatch;
use crate::pine::runtime::execution::ExecutionController;
use crate::pine::syntax::token::{Diagnostic, Limits};
use crate::pine_studio::library::registry::LibraryRegistry;
use eframe::egui;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

pub(super) struct WorkerRequest {
    pub(super) generation: u64,
    /// The bars to run over, shared with every other request made from the same bars.
    pub(super) bars: Arc<Vec<Kline>>,
    pub(super) visible_range: Option<(i64, i64)>,
    pub(super) now_ms: i64,
}

pub(super) struct WorkerResult {
    pub(super) generation: u64,
    pub(super) output: VisualOutput,
    pub(super) error: Option<Diagnostic>,
    pub(super) logs: LogBatch,
    /// The controller compiled the declaration, so `output.overlay` is authoritative even when
    /// execution stopped with a runtime error.
    pub(super) placement_known: bool,
}

/// What the controller is built from, on the first request.
pub(super) struct WorkerConfig {
    pub(super) theme: ChartTheme,
    pub(super) source: String,
    pub(super) inputs: HashMap<String, InputValue>,
    pub(super) symbol: SymbolInfo,
    /// The length of the chart's bars, when their times cannot say (price-based charts).
    pub(super) bar_seconds: Option<f64>,
}

enum WorkerEngine {
    Uninitialized(WorkerConfig),
    Ready(Box<ExecutionController>),
    Failed(Diagnostic),
}

struct WorkerState {
    engine: Option<WorkerEngine>,
    pending: Option<WorkerRequest>,
    running: bool,
    cancelled: bool,
}

/// Latest-wins background runner. Only one work-queue job touches a controller at a time; updates
/// arriving while it runs replace the pending snapshot rather than building an unbounded queue.
pub(super) struct PineWorker {
    state: Arc<Mutex<WorkerState>>,
    pub(super) results: Receiver<WorkerResult>,
    result_sender: Sender<WorkerResult>,
    repaint: egui::Context,
    cancellation: Arc<AtomicBool>,
}

impl PineWorker {
    pub(super) fn new(config: WorkerConfig, repaint: egui::Context) -> Self {
        let (result_sender, results) = mpsc::channel();
        Self {
            state: Arc::new(Mutex::new(WorkerState {
                engine: Some(WorkerEngine::Uninitialized(config)),
                pending: None,
                running: false,
                cancelled: false,
            })),
            results,
            result_sender,
            repaint,
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn request(&self, request: WorkerRequest) {
        let should_start = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.cancelled {
                return;
            }
            state.pending = Some(request);
            if state.running {
                false
            } else {
                state.running = true;
                true
            }
        };
        if !should_start {
            return;
        }
        let state = Arc::clone(&self.state);
        let sender = self.result_sender.clone();
        let repaint = self.repaint.clone();
        let cancellation = Arc::clone(&self.cancellation);
        crate::service::work_queue::submit(move || {
            loop {
                let (request, engine) = {
                    let Ok(mut state) = state.lock() else {
                        return;
                    };
                    if state.cancelled {
                        state.running = false;
                        return;
                    }
                    let Some(request) = state.pending.take() else {
                        state.running = false;
                        return;
                    };
                    let engine = state.engine.take().expect("worker engine");
                    (request, engine)
                };
                let mut engine = match engine {
                    WorkerEngine::Uninitialized(config) => match ExecutionController::new(
                        &config.source,
                        config.inputs,
                        config.symbol,
                        request.visible_range,
                        Limits::default(),
                        &LibraryRegistry,
                        Some(Box::new(KlineSecurityDataProvider)),
                        Some(Arc::clone(&cancellation)),
                    ) {
                        Ok(controller) => {
                            let controller = controller.with_theme(config.theme).with_logs();
                            WorkerEngine::Ready(Box::new(match config.bar_seconds {
                                Some(seconds) => controller.with_bar_seconds(seconds),
                                None => controller,
                            }))
                        }
                        Err(error) => WorkerEngine::Failed(error),
                    },
                    engine => engine,
                };
                let result = match &mut engine {
                    WorkerEngine::Ready(controller) => {
                        let update =
                            controller.update(&request.bars, request.visible_range, request.now_ms);
                        WorkerResult {
                            generation: request.generation,
                            output: update.output,
                            error: update.error,
                            logs: update.logs,
                            placement_known: true,
                        }
                    }
                    WorkerEngine::Failed(error) => WorkerResult {
                        generation: request.generation,
                        output: VisualOutput::default(),
                        error: Some(error.clone()),
                        logs: LogBatch::default(),
                        placement_known: false,
                    },
                    WorkerEngine::Uninitialized(_) => unreachable!(),
                };
                {
                    let Ok(mut state) = state.lock() else {
                        return;
                    };
                    state.engine = Some(engine);
                    if state.cancelled {
                        state.running = false;
                        return;
                    }
                }
                let _ = sender.send(result);
                repaint.request_repaint();
            }
        });
    }
}

impl Drop for PineWorker {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.cancelled = true;
            state.pending = None;
        }
    }
}
