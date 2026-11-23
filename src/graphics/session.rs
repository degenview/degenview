//! One launch's graphics starts. The `Record` lives here, in memory, and every move through it is
//! persisted best effort and never read back: a launch's own progress cannot depend on whether its
//! file could be written. The file is read twice only: once when the launch opens, to learn what
//! earlier launches left, and when an attempt is confirmed, because other launches write it too.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::lock::lock;

use super::ladder::Rung;
use super::persist;
use super::record::{Attempt, Choice, Record};

/// A name for this launch of the app, so an attempt can be told from another launch's.
pub fn new_launch_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

pub struct Session {
    launch: String,
    /// Where `graphics.json` lives; without one nothing is remembered between launches.
    dir: Option<PathBuf>,
    ladder: &'static [Rung],
    record: Mutex<Record>,
    /// The attempt handed out last and not yet settled.
    current: Mutex<Option<Attempt>>,
}

impl Session {
    pub fn open(
        dir: Option<PathBuf>,
        ladder: &'static [Rung],
        build: &str,
        launch: String,
    ) -> Self {
        let previous = dir.as_deref().map(persist::load).unwrap_or_default();
        let session = Self {
            launch,
            record: Mutex::new(Record::open(previous, build)),
            dir,
            ladder,
            current: Mutex::new(None),
        };
        session.persist(&lock(&session.record));
        session
    }

    /// How many rungs a launch can go through.
    pub fn rungs(&self) -> usize {
        self.ladder.len()
    }

    /// The rung for the next try, marked in progress under this launch's name.
    pub fn select(&self) -> Choice<'static> {
        let mut record = lock(&self.record);
        let choice = record.select(self.ladder, &self.launch);
        self.persist(&record);
        *lock(&self.current) = Some(choice.attempt.clone());
        choice
    }

    /// The attempt did not start: its rung is out for this build.
    pub fn fail(&self, attempt: &Attempt) {
        let mut record = lock(&self.record);
        record.fail(attempt);
        self.persist(&record);
        self.settle(attempt);
    }

    /// The attempt started. The mark is cleared if it is still this attempt's: the file is judged
    /// as it is on disk, since a launch that began after this one may have replaced the mark.
    pub fn confirm(&self, attempt: &Attempt) {
        lock(&self.record).confirm(attempt);
        self.settle(attempt);
        if let Some(dir) = &self.dir {
            let mut shared = persist::load(dir);
            if shared.confirm(attempt) {
                persist::save(dir, &shared);
            }
        }
    }

    /// Confirms the attempt in progress, if there is one: what the window's frames call.
    pub fn confirm_current(&self) {
        let current = lock(&self.current).clone();
        if let Some(attempt) = current {
            self.confirm(&attempt);
        }
    }

    fn settle(&self, attempt: &Attempt) {
        let mut current = lock(&self.current);
        if current.as_ref() == Some(attempt) {
            *current = None;
        }
    }

    fn persist(&self, record: &Record) {
        if let Some(dir) = &self.dir {
            persist::save(dir, record);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use eframe::wgpu::Backends;

    use super::*;

    const TWO: &[Rung] = &[
        Rung {
            name: "first",
            backends: Backends::DX12,
        },
        Rung {
            name: "second",
            backends: Backends::GL,
        },
    ];

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dv-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn open(dir: &Path, launch: &str) -> Session {
        Session::open(Some(dir.to_path_buf()), TWO, "build", launch.to_owned())
    }

    #[test]
    fn a_launch_that_never_confirms_makes_the_next_one_fall_back() {
        let dir = scratch("never-confirms");
        let first = open(&dir, "one");
        assert_eq!(first.select().rung.name, "first");
        // The process died in the driver: no confirm.
        let second = open(&dir, "two");
        let choice = second.select();
        assert_eq!(choice.rung.name, "second");
        assert!(choice.note().contains("did not start on this build: first"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_confirmed_rung_is_kept_by_the_next_launch() {
        let dir = scratch("confirmed");
        let first = open(&dir, "one");
        first.select();
        first.confirm_current();
        assert_eq!(open(&dir, "two").select().rung.name, "first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fallback_stays_after_it_is_confirmed() {
        let dir = scratch("sticks");
        open(&dir, "one").select();
        let second = open(&dir, "two");
        second.select();
        second.confirm_current();
        assert_eq!(open(&dir, "three").select().rung.name, "second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_try_moves_the_same_launch_to_the_next_rung_even_if_nothing_can_be_written() {
        let dir = scratch("unwritable");
        // A directory where the file belongs: it can be neither read nor written.
        std::fs::create_dir_all(dir.join("graphics.json")).unwrap();
        let session = open(&dir, "one");
        let first = session.select();
        assert_eq!(first.rung.name, "first");
        session.fail(&first.attempt);
        assert_eq!(session.select().rung.name, "second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_without_a_directory_still_walks_the_ladder() {
        let session = Session::open(None, TWO, "build", "one".to_owned());
        let first = session.select();
        session.fail(&first.attempt);
        assert_eq!(session.select().rung.name, "second");
        assert_eq!(session.rungs(), 2);
    }

    #[test]
    fn a_late_confirm_does_not_clear_the_mark_of_a_launch_that_started_after() {
        let dir = scratch("late-confirm");
        let a = open(&dir, "a");
        a.select();
        a.confirm_current();
        // B starts, records its attempt, and dies in the driver.
        let b = open(&dir, "b");
        let theirs = b.select().attempt;
        // A quits cleanly and confirms again.
        a.confirm_current();
        let shared = persist::load(&dir);
        assert_eq!(shared.attempting.as_ref(), Some(&theirs));
        // So the next launch sees B's failure.
        assert_eq!(open(&dir, "c").select().rung.name, "second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_launchs_own_late_confirm_is_harmless() {
        let dir = scratch("twice");
        let a = open(&dir, "a");
        a.select();
        a.confirm_current();
        a.confirm_current();
        assert_eq!(persist::load(&dir).attempting, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_shows_the_attempt_in_progress_and_the_failures() {
        let dir = scratch("file");
        let session = open(&dir, "one");
        let first = session.select();
        assert_eq!(
            persist::load(&dir).attempting.as_ref(),
            Some(&first.attempt)
        );
        session.fail(&first.attempt);
        let shared = persist::load(&dir);
        assert_eq!(
            (shared.failed, shared.attempting),
            (vec!["first".to_owned()], None)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
