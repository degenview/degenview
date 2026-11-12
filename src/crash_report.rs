//! What a user can send us when the app stops.
//!
//! A release build on Windows has no console, so a panic leaves nothing to look at: the window
//! closes and the user is told nothing. A panic hook fixes both halves:
//!
//! - `crash.log` in the data directory collects panics, newest last: message, location, thread and
//!   backtrace. It is capped (`MAX_CRASH_LOG`) so a panic in a loop cannot fill the disk.
//! - A panic on the main thread also calls the notifier the binary passes in, which shows a message
//!   naming the file.
//!
//! Every write is best effort. A diagnostic must never be the reason the app fails.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CRASH_LOG: &str = "crash.log";
/// The log never grows past this: an entry that would not fit starts the file afresh.
const MAX_CRASH_LOG: u64 = 256 * 1024;
/// One entry is cut to this, so a single panic cannot exceed the log's cap on its own.
const MAX_REPORT: usize = 64 * 1024;
/// A panic message is cut to this, before the backtrace and location are added around it.
const MAX_MESSAGE: usize = 4 * 1024;
const TRUNCATED: &str = "\n[truncated]\n";

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `text` cut to at most `max` bytes (marker included) on a character boundary.
fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max.saturating_sub(TRUNCATED.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED}", &text[..end])
}

fn append(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

/// The text of one `crash.log` entry.
fn format_report(
    thread: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
    at: u64,
) -> String {
    format!(
        "--- DegenView {} ({}) on {} {}, unix {at}\nthread: {thread}\npanic: {message}\nat: {}\n{backtrace}\n\n",
        env!("CARGO_PKG_VERSION"),
        crate::host::update::BUILD_ID,
        std::env::consts::OS,
        std::env::consts::ARCH,
        location.unwrap_or("unknown location"),
    )
}

/// Appends an entry, or replaces the log when the entry would take it past `MAX_CRASH_LOG`.
/// Returns whether the entry reached the file: the caller must not say it was saved otherwise.
fn write_report(dir: &Path, report: &str) -> bool {
    let report = truncate(report, MAX_REPORT);
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let path = dir.join(CRASH_LOG);
    let existing = std::fs::metadata(&path).map_or(0, |meta| meta.len());
    if existing + report.len() as u64 > MAX_CRASH_LOG {
        // Truncates in place, so a file that cannot be removed is still replaced.
        std::fs::write(&path, report).is_ok()
    } else {
        append(&path, &report).is_ok()
    }
}

/// What the user is told when the main thread panics.
fn notice(message: &str, dir: Option<&Path>) -> String {
    match dir {
        Some(dir) => format!(
            "DegenView hit an unexpected error and has to close.\n\n{message}\n\nDetails were saved to {}",
            dir.join(CRASH_LOG).display()
        ),
        None => format!("DegenView hit an unexpected error and has to close.\n\n{message}"),
    }
}

/// Records every panic in `crash.log`, and calls `notify` with a message for the user when the
/// main thread is the one that panicked. Worker threads panic and recover on purpose in places
/// (the MCP server, the Pine runners), so those are logged and not announced.
pub fn install_panic_hook(notify: fn(&str)) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let current = std::thread::current();
        let thread = current.name().unwrap_or("unnamed");
        let message = truncate(info.payload_as_str().unwrap_or("(no message)"), MAX_MESSAGE);
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let dir: Option<PathBuf> = crate::store::data_dir();
        // Only a log that was written is named to the user.
        let saved_in = dir.as_deref().filter(|dir| {
            write_report(
                dir,
                &format_report(
                    thread,
                    &message,
                    location.as_deref(),
                    &backtrace,
                    unix_seconds(),
                ),
            )
        });
        previous(info);
        if thread == "main" {
            notify(&notice(&message, saved_in));
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dv-crash-{name}-{}", unix_seconds()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_report_names_the_panic_where_it_happened_and_the_build() {
        let text = format_report("main", "boom", Some("src/app.rs:1:2"), "frames", 7);
        assert!(text.contains("thread: main"));
        assert!(text.contains("panic: boom"));
        assert!(text.contains("at: src/app.rs:1:2"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("unix 7"));
    }

    #[test]
    fn reports_are_appended_in_order_and_the_directory_is_made() {
        let dir = scratch("append");
        assert!(write_report(&dir, "first\n"));
        assert!(write_report(&dir, "second\n"));
        let text = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
        assert_eq!(text, "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_past_the_cap_is_replaced_not_grown() {
        let dir = scratch("cap");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CRASH_LOG), vec![b'x'; MAX_CRASH_LOG as usize + 1]).unwrap();
        assert!(write_report(&dir, "fresh\n"));
        let text = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
        assert_eq!(text, "fresh\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_just_under_the_cap_is_replaced_when_the_entry_would_not_fit() {
        let dir = scratch("near-cap");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CRASH_LOG), vec![b'x'; MAX_CRASH_LOG as usize - 1]).unwrap();
        assert!(write_report(&dir, "fresh\n"));
        let text = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
        assert_eq!(text, "fresh\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_entry_that_fits_exactly_is_appended() {
        let dir = scratch("exact");
        std::fs::create_dir_all(&dir).unwrap();
        let room = MAX_CRASH_LOG as usize - "fresh\n".len();
        std::fs::write(dir.join(CRASH_LOG), vec![b'x'; room]).unwrap();
        assert!(write_report(&dir, "fresh\n"));
        let len = std::fs::metadata(dir.join(CRASH_LOG)).unwrap().len();
        assert_eq!(len, MAX_CRASH_LOG);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_oversized_entry_is_cut_and_never_exceeds_the_cap() {
        let dir = scratch("oversized");
        let huge = "é".repeat(MAX_CRASH_LOG as usize);
        assert!(write_report(&dir, &huge));
        let text = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
        assert!(text.len() <= MAX_REPORT);
        assert!(text.ends_with(TRUNCATED));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncation_keeps_short_text_and_respects_character_boundaries() {
        assert_eq!(truncate("short", 100), "short");
        let cut = truncate(&"é".repeat(100), 51);
        assert!(cut.len() <= 51);
        assert!(cut.ends_with(TRUNCATED));
    }

    #[test]
    fn a_write_that_fails_says_so() {
        let dir = scratch("unwritable");
        std::fs::create_dir_all(&dir).unwrap();
        // A file where the directory should be: creating the directory beneath it fails.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        assert!(!write_report(&blocker.join("data"), "report\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_notice_says_where_the_details_went() {
        let dir = PathBuf::from("data");
        let text = notice("boom", Some(&dir));
        assert!(text.contains("boom"));
        assert!(text.contains(CRASH_LOG));
        assert!(!notice("boom", None).contains(CRASH_LOG));
    }
}
