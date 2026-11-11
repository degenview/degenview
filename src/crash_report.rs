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
/// A crash log past this size is started afresh rather than appended to.
const MAX_CRASH_LOG: u64 = 256 * 1024;

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn append(path: &Path, text: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(text.as_bytes());
    }
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
        crate::host::update_host::BUILD_ID,
        std::env::consts::OS,
        std::env::consts::ARCH,
        location.unwrap_or("unknown location"),
    )
}

/// Appends an entry, or replaces a log that has outgrown `MAX_CRASH_LOG`.
fn write_report(dir: &Path, report: &str) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(CRASH_LOG);
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_CRASH_LOG) {
        let _ = std::fs::remove_file(&path);
    }
    append(&path, report);
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
        let message = info.payload_as_str().unwrap_or("(no message)");
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let dir: Option<PathBuf> = crate::store::data_dir();
        if let Some(dir) = &dir {
            write_report(
                dir,
                &format_report(
                    thread,
                    message,
                    location.as_deref(),
                    &backtrace,
                    unix_seconds(),
                ),
            );
        }
        previous(info);
        if thread == "main" {
            notify(&notice(message, dir.as_deref()));
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
        write_report(&dir, "first\n");
        write_report(&dir, "second\n");
        let text = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
        assert_eq!(text, "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_past_the_cap_is_replaced_not_grown() {
        let dir = scratch("cap");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CRASH_LOG), vec![b'x'; MAX_CRASH_LOG as usize + 1]).unwrap();
        write_report(&dir, "fresh\n");
        let text = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
        assert_eq!(text, "fresh\n");
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
