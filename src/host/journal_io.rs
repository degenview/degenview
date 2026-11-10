//! The Trade Journal's file work: choosing, reading and writing files, importing, backing up and
//! restoring. All of it runs on a worker (a file chooser and a database write must not hold the
//! frame), one job at a time, and hands back one outcome the tab turns into a sheet, a notice or a
//! refresh.
//!
//! Nothing here uploads anything, and nothing is written outside the file the user chose.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::service::{file_dialog, work_queue};
use crate::store::SharedStore;
use degenview_journal::backup::{Backup, RestoreReport};
use degenview_journal::csv::{self, ImportSource, MAX_BYTES, Table};
use degenview_journal::source::RejectedExecution;

/// What a finished job has to say.
#[derive(Debug)]
pub enum Outcome {
    /// A CSV was chosen and read: its name and rows.
    CsvPicked(Result<Option<(String, Table)>, String>),
    /// Something was written (or the save panel was cancelled).
    Saved {
        what: &'static str,
        result: Result<Option<PathBuf>, String>,
    },
    Imported(Result<ImportSummary, String>),
    /// A backup file was chosen and read.
    BackupPicked(Result<Option<(String, Backup)>, String>),
    Restored(Result<RestoreReport, String>),
}

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub inserted: usize,
    pub already_present: usize,
    pub rejected: Vec<RejectedExecution>,
    pub accounts: usize,
}

impl ImportSummary {
    pub fn message(&self, skipped_rows: usize) -> String {
        let mut text = format!(
            "{} new executions added to {} account{}",
            self.inserted,
            self.accounts,
            if self.accounts == 1 { "" } else { "s" }
        );
        if self.already_present > 0 {
            text.push_str(&format!(
                "; {} were already in the journal and were left alone",
                self.already_present
            ));
        }
        if skipped_rows + self.rejected.len() > 0 {
            text.push_str(&format!(
                "; {} rows were refused",
                skipped_rows + self.rejected.len()
            ));
        }
        text.push('.');
        text
    }
}

#[derive(Default)]
pub struct Io {
    inbox: Option<Receiver<Outcome>>,
    /// What the running job is doing, for the tab to say.
    pub busy: Option<&'static str>,
}

impl Io {
    pub fn is_busy(&self) -> bool {
        self.inbox.is_some()
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        what: &'static str,
        job: impl FnOnce() -> Outcome + Send + 'static,
    ) {
        if self.is_busy() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.inbox = Some(receiver);
        self.busy = Some(what);
        let ctx = ctx.clone();
        work_queue::ASSETS.submit(move || {
            let _ = sender.send(job());
            ctx.request_repaint();
        });
    }

    /// The finished job's outcome, once.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        let receiver = self.inbox.as_ref()?;
        match receiver.try_recv() {
            Ok(outcome) => {
                self.inbox = None;
                self.busy = None;
                Some(outcome)
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.inbox = None;
                self.busy = None;
                Some(Outcome::Saved {
                    what: "the file",
                    result: Err("The job stopped unexpectedly.".to_owned()),
                })
            }
        }
    }

    /// Asks for a CSV and reads it.
    pub fn pick_csv(&mut self, ctx: &egui::Context) {
        self.start(ctx, "Reading the file\u{2026}", || {
            Outcome::CsvPicked(pick_and_read_csv())
        });
    }

    /// Writes `text` where the user chooses.
    pub fn save_text(
        &mut self,
        ctx: &egui::Context,
        what: &'static str,
        default_name: &'static str,
        text: String,
    ) {
        self.start(ctx, "Saving\u{2026}", move || Outcome::Saved {
            what,
            result: choose_and_write(what, default_name, &text),
        });
    }

    /// Builds text from the store on the worker, then saves it where the user chooses.
    pub fn save_built(
        &mut self,
        ctx: &egui::Context,
        what: &'static str,
        default_name: &'static str,
        build: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        self.start(ctx, "Saving\u{2026}", move || Outcome::Saved {
            what,
            result: build().and_then(|text| choose_and_write(what, default_name, &text)),
        });
    }

    /// Adds a planned import to the journal.
    pub fn import(&mut self, ctx: &egui::Context, store: SharedStore, sources: Vec<ImportSource>) {
        self.start(ctx, "Importing\u{2026}", move || {
            let result = (|| {
                let store = store
                    .lock()
                    .map_err(|_| "The library is busy.".to_owned())?;
                let mut summary = ImportSummary::default();
                for source in &sources {
                    let report = store
                        .journal_ingest(source, true)
                        .map_err(|error| format!("The import could not be saved: {error}"))?;
                    summary.inserted += report.inserted;
                    summary.already_present += report.already_present;
                    summary.rejected.extend(report.rejected);
                    summary.accounts += 1;
                }
                Ok(summary)
            })();
            Outcome::Imported(result)
        });
    }

    /// Asks for a backup file and reads it.
    pub fn pick_backup(&mut self, ctx: &egui::Context) {
        self.start(ctx, "Reading the backup\u{2026}", || {
            Outcome::BackupPicked(pick_and_read_backup())
        });
    }

    pub fn restore(&mut self, ctx: &egui::Context, store: SharedStore, backup: Backup) {
        self.start(ctx, "Restoring\u{2026}", move || {
            let result = store
                .lock()
                .map_err(|_| "The library is busy.".to_owned())
                .and_then(|store| {
                    store
                        .journal_restore(&backup)
                        .map_err(|error| format!("The backup could not be restored: {error}"))
                });
            Outcome::Restored(result)
        });
    }
}

fn pick_and_read_csv() -> Result<Option<(String, Table)>, String> {
    let Some(path) = file_dialog::choose_csv("Choose a CSV of executions")? else {
        return Ok(None);
    };
    let text = file_dialog::read_text_up_to(&path, MAX_BYTES)?;
    let table = csv::read_table(&text)?;
    Ok(Some((file_name(&path), table)))
}

fn pick_and_read_backup() -> Result<Option<(String, Backup)>, String> {
    let Some(path) = file_dialog::choose_json("Choose a journal backup")? else {
        return Ok(None);
    };
    // A backup carries screenshots: allow a good deal more than a CSV.
    let text = file_dialog::read_text_up_to(&path, 512 * 1024 * 1024)?;
    let backup = Backup::parse(&text)?;
    Ok(Some((file_name(&path), backup)))
}

fn choose_and_write(what: &str, default_name: &str, text: &str) -> Result<Option<PathBuf>, String> {
    let Some(path) = file_dialog::choose_save_path(&format!("Save {what}"), default_name)? else {
        return Ok(None);
    };
    file_dialog::write_text(&path, text)?;
    Ok(Some(path))
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
