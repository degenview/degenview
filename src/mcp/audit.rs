//! The recording side of the activity log: one row for every tool call, however it ends.
//!
//! [`AuditGuard`] is created when a call arrives and writes its row when it ends. If the call's
//! future is dropped first (the client hung up, the call was cancelled, the server stopped) the
//! guard's `Drop` records it as cancelled, so no way of leaving goes unrecorded.
//!
//! Rows go into a small bounded in-memory queue ([`AuditQueue`]) that costs a few microseconds
//! from an HTTP or worker thread. The app thread drains it (`McpHost::tick`) and writes the
//! batch to SQLite: nothing on the request path touches the store or waits on it.
//!
//! What a row may hold is fixed here: the tool's name, an outcome and a code, a duration, the
//! client's request id, and ids from a short list of argument names. Never an argument value, an
//! error message, a token or a script.

use crate::mcp::state::{ClientIdentity, McpShared};
use crate::model::mcp::{McpAuditEntry, McpAuditKind, McpAuditOutcome, McpAuditTarget};
use crate::model::watchlist::{new_id, now_ms};
use rmcp::model::{CallToolResult, NumberOrString};
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Rows waiting to be written. The app drains them about once a second, so this is only reached
/// if it stops doing so; the oldest are dropped and a `Gap` row says how many.
pub const QUEUE_LIMIT: usize = 2000;
/// Calls refused for rate limits are counted per client over this long and written as one row.
const RATE_WINDOW_MS: i64 = 60_000;
/// Longest id, request id or code kept.
const FIELD_LIMIT: usize = 32;
const TOOL_LIMIT: usize = 64;
/// The arguments whose values are kept as targets, in the order they are looked at.
const TARGET_KEYS: [&str; 9] = [
    "workspace_id",
    "chart_id",
    "instance_id",
    "alert_id",
    "script_id",
    "watchlist_id",
    "account_id",
    "order_id",
    "portfolio_id",
];
const MAX_TARGETS: usize = 4;
/// Longest request id kept (a UUID is 36).
const REQUEST_ID_LIMIT: usize = 36;

/// Keeps only characters that cannot carry anything but an id or a code, and at most `limit`.
fn clean(raw: &str, limit: usize, extra: &[char]) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || extra.contains(c))
        .take(limit)
        .collect()
}

/// A tool name as it is stored.
pub fn clean_tool(name: &str) -> String {
    clean(name, TOOL_LIMIT, &['.'])
}

/// A client's request id as text: a number, or a short string of letters, digits and hyphens
/// (a counter, a UUID). A longer or odder one is not kept.
pub fn request_id_text(id: &NumberOrString) -> String {
    match id {
        NumberOrString::Number(number) => number.to_string(),
        NumberOrString::String(text)
            if !text.is_empty()
                && text.len() <= REQUEST_ID_LIMIT
                && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') =>
        {
            text.to_string()
        }
        NumberOrString::String(_) => String::new(),
    }
}

/// Whether `value` can only be an id of this app: digits (workspace, chart, alert, account...)
/// or a hex hash (a script). Anything else, free text or a token an agent put in the wrong
/// argument, is not kept, not even cleaned up and cut short.
fn is_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= FIELD_LIMIT && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The ids a call named: up to [`MAX_TARGETS`] of the fixed argument names whose values are
/// ids (see [`is_id`]). Everything else in the arguments is ignored.
pub fn targets_of(arguments: Option<&Map<String, Value>>) -> Vec<McpAuditTarget> {
    let Some(arguments) = arguments else {
        return Vec::new();
    };
    TARGET_KEYS
        .iter()
        .filter_map(|key| {
            let id = match arguments.get(*key)? {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                _ => return None,
            };
            is_id(&id).then(|| McpAuditTarget {
                kind: (*key).to_owned(),
                id,
            })
        })
        .take(MAX_TARGETS)
        .collect()
}

/// How a finished call is recorded: its outcome and code.
pub fn outcome_of(result: &CallToolResult) -> (McpAuditOutcome, String) {
    if result.is_error != Some(true) {
        return (McpAuditOutcome::Succeeded, String::new());
    }
    let code = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("error"))
        .and_then(Value::as_str)
        .map(|code| clean(code, FIELD_LIMIT, &[]))
        .filter(|code| !code.is_empty())
        .unwrap_or_else(|| unstructured_error_code(result).to_owned());
    let outcome = match code.as_str() {
        // Refused: the credential lacks it, or the user said no to this exact call.
        "PERMISSION_DENIED" | "CONFIRMATION_DENIED" => McpAuditOutcome::Denied,
        // Nothing ran: it is waiting for the user.
        "CONFIRMATION_REQUIRED" => McpAuditOutcome::Held,
        _ => McpAuditOutcome::Failed,
    };
    (outcome, code)
}

/// The code for an error result that has no structured `error` code of ours: the router's
/// refusal of arguments that did not match a tool's schema is a plain text result. The text is
/// only compared against, never copied.
fn unstructured_error_code(result: &CallToolResult) -> &'static str {
    let malformed = result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .any(|text| text.text.starts_with("failed to deserialize parameters"));
    if malformed {
        "INVALID_ARGUMENT"
    } else {
        "ERROR"
    }
}

/// Writes one tool call's row when the call ends, or as cancelled if it never does.
pub struct AuditGuard {
    shared: Arc<McpShared>,
    entry: McpAuditEntry,
    started: Instant,
    written: bool,
}

impl AuditGuard {
    /// Starts the record of a call by `client` to `tool` with `arguments`.
    pub fn begin(
        shared: &Arc<McpShared>,
        client: &ClientIdentity,
        tool: &str,
        request_id: &NumberOrString,
        arguments: Option<&Map<String, Value>>,
    ) -> Self {
        Self {
            shared: shared.clone(),
            entry: McpAuditEntry {
                id: new_id(),
                at_ms: now_ms(),
                kind: McpAuditKind::Call,
                client_id: client.id.clone(),
                client_name: client.name.clone(),
                tool: clean_tool(tool),
                request_id: request_id_text(request_id),
                targets: targets_of(arguments),
                count: 1,
                ..McpAuditEntry::default()
            },
            started: Instant::now(),
            written: false,
        }
    }

    /// Records how the call ended.
    pub fn finish(mut self, outcome: McpAuditOutcome, code: &str) {
        self.entry.outcome = outcome;
        self.entry.code = clean(code, FIELD_LIMIT, &[]);
        self.write();
    }

    fn write(&mut self) {
        if self.written {
            return;
        }
        self.written = true;
        self.entry.duration_ms =
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.shared.audit().push(self.entry.clone());
    }
}

impl Drop for AuditGuard {
    fn drop(&mut self) {
        if !self.written {
            // The call's future was dropped before it ended: the client left, or it was cancelled.
            self.entry.outcome = McpAuditOutcome::Cancelled;
            self.write();
        }
    }
}

struct RateWindow {
    started_ms: i64,
    count: u32,
    client_name: String,
}

#[derive(Default)]
struct Inner {
    entries: VecDeque<McpAuditEntry>,
    /// Rows dropped because the queue was full, reported as one `Gap` row.
    lost: u32,
    windows: HashMap<String, RateWindow>,
}

impl Inner {
    fn push(&mut self, entry: McpAuditEntry) {
        if self.entries.len() >= QUEUE_LIMIT {
            self.entries.pop_front();
            self.lost = self.lost.saturating_add(1);
        }
        self.entries.push_back(entry);
    }
}

/// Rows waiting for the app thread to write them.
#[derive(Default)]
pub struct AuditQueue {
    inner: Mutex<Inner>,
}

impl AuditQueue {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn push(&self, entry: McpAuditEntry) {
        self.inner().push(entry);
    }

    /// Counts a call from `client_id` refused for going over its rate limit. Refusals within a
    /// minute become one row with a count, so a client in a tight loop cannot fill the log.
    pub fn rate_limited(&self, client_id: &str, client_name: &str, now_ms: i64) {
        let mut inner = self.inner();
        let window = inner
            .windows
            .entry(client_id.to_owned())
            .or_insert_with(|| RateWindow {
                started_ms: now_ms,
                count: 0,
                client_name: client_name.to_owned(),
            });
        window.count = window.count.saturating_add(1);
    }

    /// Everything waiting, oldest first. Rate-limit counts whose minute is over (or all of them,
    /// with `all`, when the app is quitting) are written as rows first. A `Gap` row leads if rows
    /// were dropped.
    pub fn take(&self, now_ms: i64, all: bool) -> Vec<McpAuditEntry> {
        let mut inner = self.inner();
        let due: Vec<String> = inner
            .windows
            .iter()
            .filter(|(_, window)| all || now_ms - window.started_ms >= RATE_WINDOW_MS)
            .map(|(client, _)| client.clone())
            .collect();
        for client in due {
            if let Some(window) = inner.windows.remove(&client) {
                inner.push(McpAuditEntry {
                    id: new_id(),
                    at_ms: window.started_ms,
                    kind: McpAuditKind::RateLimited,
                    client_id: client,
                    client_name: window.client_name,
                    outcome: McpAuditOutcome::Denied,
                    code: "RATE_LIMITED".into(),
                    count: window.count,
                    ..McpAuditEntry::default()
                });
            }
        }
        let mut taken: Vec<McpAuditEntry> = Vec::with_capacity(inner.entries.len() + 1);
        if inner.lost > 0 {
            taken.push(McpAuditEntry {
                id: new_id(),
                at_ms: now_ms,
                kind: McpAuditKind::Gap,
                count: inner.lost,
                ..McpAuditEntry::default()
            });
            inner.lost = 0;
        }
        taken.extend(inner.entries.drain(..));
        taken
    }

    /// Puts rows back at the front after a failed write, so the next try writes them first. If
    /// that overfills the queue, the oldest are dropped (and counted).
    pub fn requeue(&self, rows: Vec<McpAuditEntry>) {
        let mut inner = self.inner();
        let newer = std::mem::take(&mut inner.entries);
        inner.entries = rows.into();
        for entry in newer {
            inner.entries.push_back(entry);
        }
        while inner.entries.len() > QUEUE_LIMIT {
            inner.entries.pop_front();
            inner.lost = inner.lost.saturating_add(1);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner().entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::state::ClientIdentity;
    use serde_json::json;

    fn client() -> ClientIdentity {
        ClientIdentity {
            id: "c1".into(),
            name: "Claude".into(),
        }
    }

    fn row(n: u64) -> McpAuditEntry {
        McpAuditEntry {
            id: n,
            at_ms: n as i64,
            count: 1,
            ..McpAuditEntry::default()
        }
    }

    #[test]
    fn only_the_listed_arguments_are_kept_and_only_as_clean_ids() {
        let arguments = json!({
            "workspace_id": "46",
            "chart_id": 3,
            "script_id": "ea14e2640a47b76323ac86dbf2b0a476",
            "alert_id": "a-1_b",
            "account_id": "../../etc/passwd\n\"; DROP",
            "note": "a secret note",
            "source": "pine source //@version=6",
            "drawing_ids": ["1", "2"],
            "order_id": {"nested": "x"},
            "watchlist_id": "",
            "portfolio_id": "dvmcp_0123456789abcdef",
        });
        let targets = targets_of(arguments.as_object());
        let pairs: Vec<_> = targets
            .iter()
            .map(|t| (t.kind.as_str(), t.id.as_str()))
            .collect();
        // Only ids are kept. What is not an id is dropped whole, not cleaned up and shortened.
        assert_eq!(
            pairs,
            [
                ("workspace_id", "46"),
                ("chart_id", "3"),
                ("script_id", "ea14e2640a47b76323ac86dbf2b0a476"),
            ]
        );
    }

    #[test]
    fn at_most_four_targets_and_long_values_are_cut() {
        let arguments = json!({
            "workspace_id": "1", "chart_id": "2", "instance_id": "3", "alert_id": "4",
            "script_id": "5", "account_id": "6",
        });
        let targets = targets_of(arguments.as_object());
        assert_eq!(targets.len(), MAX_TARGETS);
        // A hash this long is not an id of ours, so nothing of it is kept.
        assert!(targets_of(json!({"script_id": "a".repeat(500)}).as_object()).is_empty());
        assert!(targets_of(json!({"script_id": "a".repeat(33)}).as_object()).is_empty());
        assert_eq!(
            targets_of(json!({"script_id": "a".repeat(32)}).as_object()).len(),
            1
        );
        assert!(targets_of(None).is_empty());
    }

    #[test]
    fn request_ids_and_tool_names_are_cleaned() {
        assert_eq!(request_id_text(&NumberOrString::Number(9)), "9");
        assert_eq!(request_id_text(&NumberOrString::Number(-3)), "-3");
        let uuid = "123e4567-e89b-12d3-a456-426614174000";
        assert_eq!(request_id_text(&NumberOrString::String(uuid.into())), uuid);
        // Anything else a client might put in an id is not kept.
        for odd in [
            "job \"a\"<b>",
            "has_underscore",
            "",
            &"x".repeat(37),
            "dvmcp_secret",
        ] {
            assert_eq!(
                request_id_text(&NumberOrString::String(odd.into())),
                "",
                "{odd}"
            );
        }
        assert_eq!(clean_tool("degenview_list_charts"), "degenview_list_charts");
        assert_eq!(clean_tool("bad name\n; rm -rf /"), "badnamerm-rf");
        assert_eq!(clean_tool(&"t".repeat(500)).len(), TOOL_LIMIT);
    }

    #[test]
    fn a_result_is_classified_from_its_error_code() {
        let ok = CallToolResult::structured(json!({"a": 1}));
        assert_eq!(outcome_of(&ok), (McpAuditOutcome::Succeeded, String::new()));

        let failed =
            CallToolResult::structured_error(json!({"error": "NOT_FOUND", "message": "No chart"}));
        assert_eq!(
            outcome_of(&failed),
            (McpAuditOutcome::Failed, "NOT_FOUND".into())
        );
        let denied = CallToolResult::structured_error(json!({"error": "PERMISSION_DENIED"}));
        assert_eq!(
            outcome_of(&denied),
            (McpAuditOutcome::Denied, "PERMISSION_DENIED".into())
        );
        // An error with no structured code still says it failed. Arguments that did not match
        // the tool's schema are named as such; the router's text is not copied into the row.
        let malformed = CallToolResult::error(vec![rmcp::model::ContentBlock::text(
            "failed to deserialize parameters: invalid type: map, SECRET_PINE_SOURCE",
        )]);
        let (outcome, code) = outcome_of(&malformed);
        assert_eq!(
            (outcome, code.as_str()),
            (McpAuditOutcome::Failed, "INVALID_ARGUMENT")
        );
        assert!(!code.contains("SECRET"));
        let plain = CallToolResult::error(vec![rmcp::model::ContentBlock::text("something broke")]);
        assert_eq!(
            outcome_of(&plain),
            (McpAuditOutcome::Failed, "ERROR".into())
        );
        // The message is never copied.
        assert!(!format!("{:?}", outcome_of(&failed)).contains("No chart"));
    }

    #[test]
    fn a_finished_call_is_one_row_and_a_dropped_one_is_cancelled() {
        let shared = McpShared::new();
        let args = json!({"chart_id": "7"});
        AuditGuard::begin(
            &shared,
            &client(),
            "degenview_remove_chart",
            &NumberOrString::Number(5),
            args.as_object(),
        )
        .finish(McpAuditOutcome::Failed, "NOT_FOUND");
        drop(AuditGuard::begin(
            &shared,
            &client(),
            "degenview_add_chart",
            &NumberOrString::String("x".into()),
            None,
        ));
        let rows = shared.audit().take(0, false);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (
                rows[0].tool.as_str(),
                rows[0].outcome,
                rows[0].code.as_str()
            ),
            (
                "degenview_remove_chart",
                McpAuditOutcome::Failed,
                "NOT_FOUND"
            )
        );
        assert_eq!(rows[0].client_id, "c1");
        assert_eq!(rows[0].client_name, "Claude");
        assert_eq!(rows[0].request_id, "5");
        assert_eq!(rows[0].targets[0].id, "7");
        assert_eq!(rows[1].outcome, McpAuditOutcome::Cancelled);
        assert_eq!(rows[1].request_id, "x");
        assert_ne!(rows[0].id, rows[1].id);
    }

    #[test]
    fn rows_come_out_oldest_first_and_only_once() {
        let queue = AuditQueue::default();
        for n in 1..=3 {
            queue.push(row(n));
        }
        let first = queue.take(0, false);
        assert_eq!(first.iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(queue.take(0, false).is_empty());
    }

    #[test]
    fn a_full_queue_drops_the_oldest_and_says_how_many() {
        let queue = AuditQueue::default();
        for n in 0..(QUEUE_LIMIT as u64 + 5) {
            queue.push(row(n + 1));
        }
        assert_eq!(queue.len(), QUEUE_LIMIT);
        let rows = queue.take(77, false);
        assert_eq!(rows[0].kind, McpAuditKind::Gap);
        assert_eq!(rows[0].count, 5);
        assert_eq!(rows[1].id, 6, "the five oldest were the ones dropped");
        // Reported once.
        assert!(queue.take(77, false).is_empty());
    }

    #[test]
    fn refusals_within_a_minute_are_one_row_per_client() {
        let queue = AuditQueue::default();
        for _ in 0..500 {
            queue.rate_limited("c1", "Claude", 1_000);
        }
        queue.rate_limited("c2", "Other", 2_000);
        // Still inside the minute: nothing is written yet.
        assert!(queue.take(30_000, false).is_empty());
        let mut rows = queue.take(1_000 + RATE_WINDOW_MS, false);
        assert_eq!(rows.len(), 1, "only c1's minute is over");
        let limited = rows.remove(0);
        assert_eq!(limited.kind, McpAuditKind::RateLimited);
        assert_eq!((limited.client_id.as_str(), limited.count), ("c1", 500));
        assert_eq!(limited.outcome, McpAuditOutcome::Denied);
        assert_eq!(limited.code, "RATE_LIMITED");
        // On quit everything still counting is written.
        let rest = queue.take(1_000 + RATE_WINDOW_MS, true);
        assert_eq!(
            (rest.len(), rest[0].client_id.as_str(), rest[0].count),
            (1, "c2", 1)
        );
    }

    #[test]
    fn rows_put_back_after_a_failed_write_go_first_and_stay_bounded() {
        let queue = AuditQueue::default();
        queue.push(row(3));
        queue.requeue(vec![row(1), row(2)]);
        let rows = queue.take(0, false);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2, 3]);

        let full = AuditQueue::default();
        for n in 0..QUEUE_LIMIT as u64 {
            full.push(row(n + 100));
        }
        full.requeue(vec![row(1), row(2)]);
        let rows = full.take(0, false);
        assert_eq!(rows[0].kind, McpAuditKind::Gap);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows.len(), QUEUE_LIMIT + 1);
        assert_eq!(
            rows[1].id, 100,
            "the requeued rows were the oldest, so they went"
        );
    }
}
