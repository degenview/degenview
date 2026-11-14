use super::chart::no_chart;
use crate::host::mcp::{McpTabs, confirmation};
use crate::mcp::bridge::{McpError, McpReply};
use crate::mcp::confirm::Summary;
use crate::mcp::snapshot::{SnapshotCommand, SnapshotProgress, SnapshotStarted};
use crate::workspace::{CaptureError, ChartWorkspace};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Starts or collects a picture of a chart. `jobs` are the pictures being made between requests.
pub fn run(
    command: SnapshotCommand,
    jobs: &Mutex<SnapshotJobs>,
    tabs: &impl McpTabs,
    ctx: &eframe::egui::Context,
) -> Result<McpReply, McpError> {
    let mut jobs = jobs
        .lock()
        .map_err(|_| McpError::AppUnavailable("Chart pictures are unavailable right now".into()))?;
    match command {
        SnapshotCommand::Begin {
            workspace_id,
            chart_id,
            scale,
        } => {
            let started = tabs.with_workspace(workspace_id, |workspace| {
                jobs.begin(workspace, chart_id, scale, ctx)
            })?;
            Ok(McpReply::SnapshotStarted(started))
        }
        SnapshotCommand::Poll { job_id } => jobs.poll(job_id).map(McpReply::SnapshotProgress),
    }
}

/// Pictures are reads and are only described if one ever asks (see [`confirmation`]).
pub fn describe(command: &SnapshotCommand) -> Result<Summary, McpError> {
    confirmation::plainly(command)
}

/// A picture being made for an agent: frozen on the UI thread, drawn on the snapshot worker.
pub struct SnapshotJob {
    receiver: std::sync::mpsc::Receiver<Result<Vec<u8>, String>>,
    width: u32,
    height: u32,
    started: Instant,
}

/// The pictures being made for agents. Bounded, and a job nobody asks for is forgotten.
#[derive(Default)]
pub struct SnapshotJobs {
    next: u64,
    jobs: std::collections::HashMap<u64, SnapshotJob>,
}

impl SnapshotJobs {
    const MAX_ACTIVE: usize = 4;
    const MAX_AGE: Duration = Duration::from_secs(120);

    /// Forgets the pictures nobody collected in time (and with them the PNGs they hold). The app
    /// calls it every tick, so they do not wait for another snapshot call to be reaped.
    pub fn expire(&mut self) {
        self.jobs
            .retain(|_, job| job.started.elapsed() < Self::MAX_AGE);
    }

    pub fn begin(
        &mut self,
        workspace: &ChartWorkspace,
        chart_id: u64,
        scale: crate::model::chart::snapshot::SnapshotScale,
        ctx: &eframe::egui::Context,
    ) -> Result<SnapshotStarted, McpError> {
        self.expire();
        if self.jobs.len() >= Self::MAX_ACTIVE {
            return Err(McpError::ResourceLimit(
                "Too many chart pictures are being made; try again shortly".into(),
            ));
        }
        let (receiver, dimensions) = workspace
            .snapshot_png_for_mcp(ctx, chart_id, scale)
            .map_err(|error| match error {
                CaptureError::NoChart | CaptureError::NoMarketChart => {
                    no_chart(workspace, chart_id)
                }
                other => McpError::Conflict(other.to_string()),
            })?;
        self.next += 1;
        self.jobs.insert(
            self.next,
            SnapshotJob {
                receiver,
                width: dimensions.width,
                height: dimensions.height,
                started: Instant::now(),
            },
        );
        Ok(SnapshotStarted {
            job_id: self.next,
            width: dimensions.width,
            height: dimensions.height,
        })
    }

    pub fn poll(&mut self, job_id: u64) -> Result<SnapshotProgress, McpError> {
        self.expire();
        let Some(job) = self.jobs.get(&job_id) else {
            return Err(McpError::NotFound(
                "No such picture job (it finished, was collected or expired)".into(),
            ));
        };
        match job.receiver.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(SnapshotProgress::Pending),
            Ok(Ok(png)) => {
                let (width, height) = (job.width, job.height);
                self.jobs.remove(&job_id);
                if png.len() > crate::model::chart::snapshot::MAX_MCP_PNG_BYTES {
                    return Err(McpError::ResourceLimit(format!(
                        "The picture is {} MiB, over the {} MiB an MCP reply carries; ask for \
                         scale 1x or a chart with fewer or smaller images",
                        png.len().div_ceil(1024 * 1024),
                        crate::model::chart::snapshot::MAX_MCP_PNG_BYTES / (1024 * 1024)
                    )));
                }
                Ok(SnapshotProgress::Ready {
                    png: Arc::new(png),
                    width,
                    height,
                })
            }
            Ok(Err(message)) => {
                self.jobs.remove(&job_id);
                Err(McpError::AppUnavailable(message))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.jobs.remove(&job_id);
                Err(McpError::AppUnavailable(
                    "The picture stopped before it finished".into(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(
        started: Instant,
    ) -> (
        SnapshotJob,
        std::sync::mpsc::Sender<Result<Vec<u8>, String>>,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            SnapshotJob {
                receiver,
                width: 4,
                height: 2,
                started,
            },
            sender,
        )
    }

    #[test]
    fn a_picture_job_is_pending_then_collected_once() {
        let mut jobs = SnapshotJobs::default();
        let (job, sender) = job(Instant::now());
        jobs.jobs.insert(1, job);
        assert_eq!(jobs.poll(1), Ok(SnapshotProgress::Pending));
        sender.send(Ok(vec![1, 2, 3])).unwrap();
        assert_eq!(
            jobs.poll(1),
            Ok(SnapshotProgress::Ready {
                png: Arc::new(vec![1, 2, 3]),
                width: 4,
                height: 2
            })
        );
        assert!(
            matches!(jobs.poll(1), Err(McpError::NotFound(_))),
            "collected once"
        );
    }

    #[test]
    fn a_failed_or_abandoned_picture_job_reports_and_is_forgotten() {
        let mut jobs = SnapshotJobs::default();
        let (failing, sender) = job(Instant::now());
        jobs.jobs.insert(1, failing);
        sender
            .send(Err("Could not draw the image.".into()))
            .unwrap();
        assert_eq!(
            jobs.poll(1),
            Err(McpError::AppUnavailable("Could not draw the image.".into()))
        );
        let (abandoned, sender) = job(Instant::now());
        jobs.jobs.insert(2, abandoned);
        drop(sender);
        assert!(matches!(jobs.poll(2), Err(McpError::AppUnavailable(_))));
        assert!(jobs.jobs.is_empty());
    }

    #[test]
    fn a_picture_over_the_reply_budget_is_refused_and_forgotten() {
        let mut jobs = SnapshotJobs::default();
        let (big, sender) = job(Instant::now());
        jobs.jobs.insert(1, big);
        sender
            .send(Ok(vec![
                0;
                crate::model::chart::snapshot::MAX_MCP_PNG_BYTES + 1
            ]))
            .unwrap();
        assert!(matches!(jobs.poll(1), Err(McpError::ResourceLimit(_))));
        assert!(jobs.jobs.is_empty(), "the oversized PNG is not kept");
        let (fits, sender) = job(Instant::now());
        jobs.jobs.insert(2, fits);
        sender
            .send(Ok(vec![
                0;
                crate::model::chart::snapshot::MAX_MCP_PNG_BYTES
            ]))
            .unwrap();
        assert!(matches!(jobs.poll(2), Ok(SnapshotProgress::Ready { .. })));
    }

    #[test]
    fn expiring_reaps_an_uncollected_picture_without_a_poll() {
        let mut jobs = SnapshotJobs::default();
        let long_ago = Instant::now()
            .checked_sub(Duration::from_secs(300))
            .expect("the clock has run for five minutes");
        let (stale, _sender) = job(long_ago);
        let (fresh, _fresh_sender) = job(Instant::now());
        jobs.jobs.insert(1, stale);
        jobs.jobs.insert(2, fresh);
        jobs.expire();
        assert_eq!(jobs.jobs.len(), 1);
        assert!(jobs.jobs.contains_key(&2));
    }

    #[test]
    fn a_picture_nobody_collects_expires() {
        let mut jobs = SnapshotJobs::default();
        let long_ago = Instant::now()
            .checked_sub(Duration::from_secs(300))
            .expect("the clock has run for five minutes");
        let (stale, _sender) = job(long_ago);
        jobs.jobs.insert(1, stale);
        assert!(matches!(jobs.poll(1), Err(McpError::NotFound(_))));
        assert!(jobs.jobs.is_empty());
    }
}
