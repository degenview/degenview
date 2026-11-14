//! Scripts run once each over the same bars, one after the other on the work queue: what the
//! indicator picker draws its previews from. Each result is handed over as it finishes, so a frame
//! never waits for a script.

use crate::model::Kline;
use crate::pine::compiler::program::{SymbolInfo, VisualOutput};
use crate::pine::syntax::token::Diagnostic;
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// One script's run: what it drew, and the failure that stopped it, if one did.
pub struct Finished<K> {
    pub key: K,
    pub output: VisualOutput,
    pub failure: Option<Diagnostic>,
}

/// A batch of runs in flight. Dropping it stops the batch after the script running then.
pub struct BatchRun<K> {
    results: Receiver<Finished<K>>,
}

impl<K: Send + 'static> BatchRun<K> {
    /// Runs every `(key, source)` in order over `bars` at its default inputs, repainting as each
    /// one finishes.
    pub fn start(
        scripts: Vec<(K, String)>,
        bars: Arc<Vec<Kline>>,
        symbol: SymbolInfo,
        repaint: egui::Context,
    ) -> Self {
        let (sender, results) = channel();
        crate::service::work_queue::submit(move || {
            for (key, source) in scripts {
                let (output, failure) =
                    super::run_script_checked(&source, &bars, &symbol, &HashMap::new());
                if sender
                    .send(Finished {
                        key,
                        output,
                        failure,
                    })
                    .is_err()
                {
                    return;
                }
                repaint.request_repaint();
            }
        });
        Self { results }
    }

    /// The runs finished since the last call.
    pub fn finished(&self) -> impl Iterator<Item = Finished<K>> + '_ {
        self.results.try_iter()
    }
}
