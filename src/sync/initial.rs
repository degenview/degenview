//! The first-time merge: what happens the first time a database syncs, whether it is the first
//! Mac to use iCloud for DegenView or the second joining data that is already there.
//!
//! While this has not run, the core reads iCloud's data but applies and uploads nothing. The
//! user sees what was found and chooses how to combine it; only then are the fetched records
//! applied (with the ordinary merge rules, so nothing on either side is silently discarded) and
//! this Mac's own data queued to upload. An empty iCloud is never a reason to delete local data.

use super::account::{disable, stash};
use super::apply::{ApplyCtx, ApplyReport, apply_batch, remote_from_inbox};
use super::core::RemoteBatch;
use super::policy::EntityKind;
use super::scripts::{ScriptCache, ScriptSink, scan_scripts};
use crate::store::Store;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PENDING: &str = "initial_pending";

/// How many of each kind of thing. Drawings are counted with their layouts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub watchlists: usize,
    pub layouts: usize,
    pub templates: usize,
    pub scripts: usize,
    pub alerts: usize,
    pub portfolios: usize,
    pub paper_accounts: usize,
    pub settings: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.watchlists
            + self.layouts
            + self.templates
            + self.scripts
            + self.alerts
            + self.portfolios
            + self.paper_accounts
            + self.settings
    }

    fn add(&mut self, kind: EntityKind) {
        match kind {
            EntityKind::Watchlist => self.watchlists += 1,
            EntityKind::Layout => self.layouts += 1,
            EntityKind::ChartTemplate | EntityKind::DrawingTemplate | EntityKind::EditorTheme => {
                self.templates += 1
            }
            EntityKind::Script => self.scripts += 1,
            EntityKind::Alert => self.alerts += 1,
            EntityKind::Portfolio => self.portfolios += 1,
            EntityKind::PaperAccount => self.paper_accounts += 1,
            EntityKind::Setting => self.settings += 1,
            // Counted with their portfolio and layout.
            EntityKind::LayoutDrawing
            | EntityKind::DrawingAsset
            | EntityKind::PortfolioTransaction => {}
        }
    }
}

/// What the user is shown before choosing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSummary {
    /// What iCloud already holds.
    pub cloud: Counts,
    /// What this Mac holds that would sync.
    pub local: Counts,
    /// The Macs that wrote iCloud's data, by device id.
    pub cloud_devices: BTreeSet<String>,
}

impl JoinSummary {
    /// iCloud has nothing yet: this Mac is first, and nothing needs choosing.
    pub fn cloud_is_empty(&self) -> bool {
        self.cloud.total() == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinMode {
    /// Combine both. Nothing is discarded on either side.
    Merge,
    /// Use iCloud's watchlists, layouts and templates in place of this Mac's, after stashing a
    /// recoverable copy. Alerts and scripts are always merged, never replaced.
    UseCloud,
    /// Do not sync this database.
    KeepLocalOnly,
}

/// Counts what iCloud holds (read, not yet applied) next to what this Mac holds.
pub fn summarize(store: &Store, local_scripts: usize) -> rusqlite::Result<JoinSummary> {
    let mut cloud = Counts::default();
    let mut cloud_devices = BTreeSet::new();
    for entry in store.inbox()?.into_iter().filter(|e| e.reason == PENDING) {
        if let Some(kind) = EntityKind::from_prefix(&entry.entity_type) {
            cloud.add(kind);
            if !entry.origin_device.is_empty() {
                cloud_devices.insert(entry.origin_device);
            }
        }
    }
    let rows = |table: &str| store.raw_rows(table).map(|r| r.len());
    let local = Counts {
        watchlists: rows("watchlist")?,
        layouts: rows("saved_view")?,
        templates: rows("chart_template")? + rows("drawing_template")? + rows("editor_theme")?,
        scripts: local_scripts,
        alerts: rows("price_alert")?,
        portfolios: rows("portfolio")?,
        paper_accounts: rows("paper_account")?,
        settings: 0,
    };
    Ok(JoinSummary {
        cloud,
        local,
        cloud_devices,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinOutcome {
    pub report: ApplyReport,
    /// Where this Mac's replaced data was stashed, for `UseCloud`.
    pub backup: Option<PathBuf>,
}

/// Runs the first-time merge in the chosen mode. Safe to run again after an interruption: the
/// records it applies are still in the inbox until they succeed, and applying is idempotent.
pub fn finish_join(
    store: &Store,
    scripts: &mut dyn ScriptSink,
    script_cache: &ScriptCache,
    mode: JoinMode,
    backups_dir: &Path,
    ctx: &ApplyCtx,
) -> rusqlite::Result<JoinOutcome> {
    if mode == JoinMode::KeepLocalOnly {
        for entry in store.inbox()?.into_iter().filter(|e| e.reason == PENDING) {
            store.remove_inbox(&entry.record_name)?;
        }
        disable(store)?;
        return Ok(JoinOutcome {
            report: ApplyReport::default(),
            backup: None,
        });
    }

    let mut backup = None;
    if mode == JoinMode::UseCloud {
        backup = Some(stash(store, backups_dir, ctx.now_ms)?);
        store.clear_replaceable_collections()?;
    }

    let mut records = Vec::new();
    for entry in store.inbox()?.into_iter().filter(|e| e.reason == PENDING) {
        match remote_from_inbox(&entry) {
            Some(record) => records.push(record),
            None => store.remove_inbox(&entry.record_name)?,
        }
    }
    let batch = RemoteBatch {
        id: 0,
        records,
        deletions: Vec::new(),
    };
    let mut report = apply_batch(store, scripts, &batch, ctx)?;
    // Whatever applied is done; anything that could not be applied was re-filed under its own
    // reason, so what still says "pending" has been dealt with.
    for entry in store.inbox()?.into_iter().filter(|e| e.reason == PENDING) {
        store.remove_inbox(&entry.record_name)?;
    }

    // An untouched default paper account would only duplicate the one iCloud brought.
    if store.drop_untouched_default_paper_account(ctx.now_ms)? {
        report.tables.insert("paper_account".to_string());
    }

    // Now that iCloud's data is linked, queue what is only here or differs from iCloud.
    scan_scripts(store, scripts, script_cache)?;
    store.reconcile_stored()?;
    if let Some(mut state) = store.sync_state()? {
        state.initial_sync_done = true;
        store.save_sync_state(&state)?;
    }
    Ok(JoinOutcome { report, backup })
}
