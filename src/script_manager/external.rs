//! Following a script's file when it is edited outside the app: by a person in another editor, or by
//! a coding agent working in the Scripts folder.
//!
//! The Script Manager already rescans the folder, so the list follows the files. What this adds is the
//! open script's *text*: a clean editor shows the file's new text, and an editor with unsaved typing
//! is never overwritten but asks. The rules are a plain state machine here (times are passed in), so
//! each is a test; `mod.rs` does the I/O and `disk_banner` draws the question.
//!
//! Two rules matter most. **A file is acted on only once it has stopped changing** (the same text seen
//! twice, `SETTLE` apart), so a file another tool is still writing is never loaded half-written. And
//! **typing is never replaced without asking**, and while the question is open Save is held, so
//! saving cannot overwrite the outside edit.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use super::store::{FileStamp, with_trailing_newline};

/// How often the open script's file is looked at (a metadata read, not a read of the file).
pub const FAST_POLL: Duration = Duration::from_millis(400);
/// How long a changed file must stay as it is before it is acted on.
pub const SETTLE: Duration = Duration::from_millis(300);
/// How long "Reloaded" stays in the status strip.
pub const NOTICE_FOR: Duration = Duration::from_secs(6);

/// Equal as the app saves text: a file always ends in a newline, so one that does not and the same
/// text with one are the same script.
pub fn same_text(a: &str, b: &str) -> bool {
    a == b || with_trailing_newline(a) == with_trailing_newline(b)
}

pub fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// The question asked when the file changed under unsaved typing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conflict {
    /// The file's text.
    pub text: String,
    pub hash: u64,
}

/// What the state machine asks the manager to do about the file's text.
#[derive(Debug, Eq, PartialEq)]
pub enum Verdict {
    Nothing,
    /// The editor has no unsaved typing: show this text, as the saved text.
    Reload(String),
    /// The editor already shows the file's text; only the saved baseline moves.
    Baseline,
    /// The editor has typing and the file differs: `Watch::conflict` holds the question.
    Conflict,
}

#[derive(Default)]
pub struct Watch {
    stamp: Option<FileStamp>,
    last_poll: Option<Instant>,
    /// A text of the file (by hash) waiting to be seen unchanged for `SETTLE`.
    seen: Option<(u64, Instant)>,
    /// Open while the file and the typing disagree. Save is held meanwhile.
    pub conflict: Option<Conflict>,
    /// Look now and do not wait for the file to settle: a save found it changed.
    pub force: bool,
}

impl Watch {
    /// Whether it is time to look at the file's metadata.
    pub fn poll_due(&self, now: Instant) -> bool {
        self.last_poll
            .is_none_or(|at| now.saturating_duration_since(at) >= FAST_POLL)
    }

    pub fn note_poll(&mut self, now: Instant) {
        self.last_poll = Some(now);
    }

    /// Given the file's stamp, whether the folder is worth rescanning now: it was rewritten, a new
    /// text is still settling, or a check was forced.
    pub fn wants_scan(&mut self, stamp: Option<FileStamp>) -> bool {
        let moved = stamp != self.stamp;
        self.stamp = stamp;
        moved || self.seen.is_some() || self.force
    }

    /// Compares the file's text with the editor's: `saved` is what the editor last read from or wrote
    /// to the file, `source` what is in the editor now.
    pub fn observe(&mut self, now: Instant, disk: &str, saved: &str, source: &str) -> Verdict {
        let force = std::mem::take(&mut self.force);
        if same_text(disk, saved) {
            self.seen = None;
            self.conflict = None;
            return Verdict::Nothing;
        }
        if same_text(disk, source) {
            self.seen = None;
            self.conflict = None;
            return Verdict::Baseline;
        }
        let hash = text_hash(disk);
        if !force {
            match self.seen {
                Some((seen, at)) if seen == hash => {
                    if now.saturating_duration_since(at) < SETTLE {
                        return Verdict::Nothing;
                    }
                }
                _ => {
                    self.seen = Some((hash, now));
                    return Verdict::Nothing;
                }
            }
        }
        self.seen = None;
        if same_text(source, saved) {
            self.conflict = None;
            Verdict::Reload(disk.to_owned())
        } else {
            self.conflict = Some(Conflict {
                text: disk.to_owned(),
                hash,
            });
            Verdict::Conflict
        }
    }

    /// The person chose their own version. The caller moves the editor's saved baseline to the file's
    /// text, so the file and the baseline agree and the question stays closed until the file changes
    /// again.
    pub fn keep(&mut self) {
        self.conflict = None;
        self.seen = None;
    }

    /// The file's text is now the editor's.
    pub fn reloaded(&mut self) {
        self.conflict = None;
        self.seen = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "//@version=6\nindicator(\"A\")\n";
    const B: &str = "//@version=6\nindicator(\"B\")\n";
    const C: &str = "//@version=6\nindicator(\"C\")\n";

    fn later(from: Instant, ms: u64) -> Instant {
        from + Duration::from_millis(ms)
    }

    #[test]
    fn a_missing_final_newline_is_not_a_change() {
        let mut watch = Watch::default();
        let now = Instant::now();
        let bare = A.trim_end();
        assert_eq!(watch.observe(now, A, bare, bare), Verdict::Nothing);
        assert_eq!(watch.observe(now, bare, A, A), Verdict::Nothing);
        // Typing that only adds the newline is not a conflict either.
        assert_eq!(watch.observe(now, B, A, A.trim_end()), Verdict::Nothing);
    }

    #[test]
    fn a_file_the_editor_already_has_asks_for_nothing() {
        let mut watch = Watch::default();
        let now = Instant::now();
        assert_eq!(watch.observe(now, A, A, A), Verdict::Nothing);
        // Typing over an unchanged file is not the file's business.
        assert_eq!(watch.observe(now, A, A, B), Verdict::Nothing);
        assert!(watch.conflict.is_none());
    }

    #[test]
    fn a_changed_file_is_loaded_only_once_it_has_stopped_changing() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        // First sighting: wait.
        assert_eq!(watch.observe(t0, B, A, A), Verdict::Nothing);
        // Seen again too soon: still wait.
        assert_eq!(watch.observe(later(t0, 100), B, A, A), Verdict::Nothing);
        // Settled.
        assert_eq!(
            watch.observe(later(t0, 400), B, A, A),
            Verdict::Reload(B.to_owned())
        );
    }

    #[test]
    fn a_file_that_keeps_changing_is_not_loaded_until_it_stops() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        assert_eq!(watch.observe(t0, B, A, A), Verdict::Nothing);
        // Another tool wrote more: the clock starts again from this text.
        assert_eq!(watch.observe(later(t0, 400), C, A, A), Verdict::Nothing);
        assert_eq!(watch.observe(later(t0, 500), C, A, A), Verdict::Nothing);
        assert_eq!(
            watch.observe(later(t0, 800), C, A, A),
            Verdict::Reload(C.to_owned())
        );
    }

    #[test]
    fn a_forced_check_does_not_wait() {
        let mut watch = Watch {
            force: true,
            ..Watch::default()
        };
        assert_eq!(
            watch.observe(Instant::now(), B, A, A),
            Verdict::Reload(B.to_owned())
        );
        assert!(!watch.force);
    }

    #[test]
    fn typing_is_never_replaced_the_question_is_asked_instead() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        let typed = "typed\n";
        assert_eq!(watch.observe(t0, B, A, typed), Verdict::Nothing);
        assert_eq!(
            watch.observe(later(t0, 400), B, A, typed),
            Verdict::Conflict
        );
        let conflict = watch.conflict.clone().expect("a question");
        assert_eq!(conflict.text, B);
        assert_eq!(conflict.hash, text_hash(B));
    }

    #[test]
    fn keeping_my_version_closes_the_question_until_the_file_changes_again() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        let typed = "typed\n";
        watch.force = true;
        assert_eq!(watch.observe(t0, B, A, typed), Verdict::Conflict);
        watch.keep();
        assert!(watch.conflict.is_none());
        // The editor now measures against the file's text (B), so the same file is no news.
        assert_eq!(
            watch.observe(later(t0, 1000), B, B, typed),
            Verdict::Nothing
        );
        assert_eq!(
            watch.observe(later(t0, 2000), B, B, typed),
            Verdict::Nothing
        );
        // The file changes again: the question comes back.
        watch.force = true;
        assert_eq!(
            watch.observe(later(t0, 3000), C, B, typed),
            Verdict::Conflict
        );
    }

    #[test]
    fn typing_the_files_text_or_undoing_the_typing_settles_it_without_a_question() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        watch.force = true;
        assert_eq!(watch.observe(t0, B, A, "typed\n"), Verdict::Conflict);
        // The person's text became exactly the file's text: only the baseline moves.
        assert_eq!(watch.observe(later(t0, 1), B, A, B), Verdict::Baseline);
        assert!(watch.conflict.is_none());
        // Or they undid their typing: the editor is clean, so the file just loads.
        watch.force = true;
        assert_eq!(
            watch.observe(later(t0, 2), C, A, A),
            Verdict::Reload(C.to_owned())
        );
    }

    #[test]
    fn the_file_going_back_to_what_the_editor_read_closes_the_question() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        watch.force = true;
        watch.observe(t0, B, A, "typed\n");
        assert!(watch.conflict.is_some());
        assert_eq!(
            watch.observe(later(t0, 1), A, A, "typed\n"),
            Verdict::Nothing
        );
        assert!(watch.conflict.is_none());
    }

    #[test]
    fn the_folder_is_rescanned_when_the_file_moves_while_settling_or_when_forced() {
        let mut watch = Watch::default();
        let stamp = |ms, len| Some(crate::script_manager::store::FileStamp::for_test(ms, len));
        // The first look always scans (nothing was known).
        assert!(watch.wants_scan(stamp(1, 10)));
        assert!(!watch.wants_scan(stamp(1, 10)));
        // The file was rewritten.
        assert!(watch.wants_scan(stamp(2, 12)));
        assert!(!watch.wants_scan(stamp(2, 12)));
        // A text is settling: keep looking until it is decided.
        watch.observe(Instant::now(), B, A, A);
        assert!(watch.wants_scan(stamp(2, 12)));
        // Forced.
        let mut forced = Watch::default();
        forced.wants_scan(stamp(1, 10));
        forced.force = true;
        assert!(forced.wants_scan(stamp(1, 10)));
    }

    #[test]
    fn polling_is_paced() {
        let mut watch = Watch::default();
        let t0 = Instant::now();
        assert!(watch.poll_due(t0));
        watch.note_poll(t0);
        assert!(!watch.poll_due(later(t0, 100)));
        assert!(watch.poll_due(later(t0, 400)));
    }
}
