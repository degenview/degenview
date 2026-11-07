//! Searching the MCP activity log: the filters the log sheet offers (client, how it ended, when,
//! a word in the tool or an id) and paging through what they match.
//!
//! Pure values over the rows the host already holds in memory. The log is capped at
//! [`AUDIT_LOG_LIMIT`](crate::model::mcp::AUDIT_LOG_LIMIT) rows, so filtering them all on every
//! frame costs microseconds and needs no query, no cache and no store access.

use crate::model::mcp::{McpAuditEntry, McpAuditKind, McpAuditOutcome};
use std::collections::HashMap;
use std::ops::Range;

/// Rows on one page of the log sheet.
pub const ACTIVITY_PAGE_SIZE: usize = 25;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// A custom range's end is a minute on a clock face: it includes that whole minute.
const MINUTE_MS: i64 = 60_000;

// MARK: Status

/// How a row ended, in the words the sheet filters by. Rows that are not about a call (the
/// "log cleared" and "rows lost" markers) have none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum McpActivityStatus {
    /// The call ran and answered.
    Done,
    /// It ran, or tried to, and ended in an error.
    Failed,
    /// Refused before it did anything: permission, an unknown tool.
    Denied,
    /// Waiting for the user's approval; nothing ran.
    Held,
    /// The user approved a held call.
    Approved,
    /// The user declined a held call.
    Declined,
    /// Nobody answered a held call in time.
    Expired,
    /// The client hung up before an answer was sent.
    Cancelled,
    /// The client was refused for going over its rate limit.
    RateLimited,
}

impl McpActivityStatus {
    /// Every status, in the order the filter lists them.
    pub const ALL: [Self; 9] = [
        Self::Done,
        Self::Failed,
        Self::Denied,
        Self::Held,
        Self::Approved,
        Self::Declined,
        Self::Expired,
        Self::Cancelled,
        Self::RateLimited,
    ];

    /// The word on a row's tag and in the filter.
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Failed => "Failed",
            Self::Denied => "Denied",
            Self::Held => "Held",
            Self::Approved => "Approved",
            Self::Declined => "Declined",
            Self::Expired => "Expired",
            Self::Cancelled => "Cancelled",
            Self::RateLimited => "Limited",
        }
    }
}

impl McpAuditEntry {
    /// How this row ended, or `None` for a marker that is not about a call.
    pub fn status(&self) -> Option<McpActivityStatus> {
        match self.kind {
            McpAuditKind::RateLimited => Some(McpActivityStatus::RateLimited),
            McpAuditKind::Cleared | McpAuditKind::Gap => None,
            McpAuditKind::Call | McpAuditKind::Confirmation => Some(match self.outcome {
                McpAuditOutcome::Succeeded => McpActivityStatus::Done,
                McpAuditOutcome::Failed => McpActivityStatus::Failed,
                McpAuditOutcome::Denied => McpActivityStatus::Denied,
                McpAuditOutcome::Cancelled => McpActivityStatus::Cancelled,
                McpAuditOutcome::Held => McpActivityStatus::Held,
                McpAuditOutcome::Approved => McpActivityStatus::Approved,
                McpAuditOutcome::Declined => McpActivityStatus::Declined,
                McpAuditOutcome::Expired => McpActivityStatus::Expired,
            }),
        }
    }
}

// MARK: Date range

/// When a row happened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpActivityRange {
    #[default]
    Any,
    LastHour,
    /// Since midnight, in the viewer's time zone.
    Today,
    Last24Hours,
    Last7Days,
    /// Between two moments the user picked; either end may be open. The end includes its whole
    /// minute, because the picker names minutes.
    Custom {
        from_ms: Option<i64>,
        to_ms: Option<i64>,
    },
}

impl McpActivityRange {
    /// The presets, in the order the filter lists them; `Custom` follows them there.
    pub const PRESETS: [Self; 5] = [
        Self::Any,
        Self::LastHour,
        Self::Today,
        Self::Last24Hours,
        Self::Last7Days,
    ];

    /// The preset's name in the filter.
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any time",
            Self::LastHour => "Last hour",
            Self::Today => "Today",
            Self::Last24Hours => "Last 24 hours",
            Self::Last7Days => "Last 7 days",
            Self::Custom { .. } => "Custom range…",
        }
    }

    /// `(from, to)` in epoch milliseconds, both inclusive, `None` where open. `now_ms` and the
    /// viewer's offset east of UTC resolve the presets that move with the clock.
    pub fn bounds(self, now_ms: i64, utc_offset_secs: i64) -> (Option<i64>, Option<i64>) {
        match self {
            Self::Any => (None, None),
            Self::LastHour => (Some(now_ms - HOUR_MS), None),
            Self::Last24Hours => (Some(now_ms - DAY_MS), None),
            Self::Last7Days => (Some(now_ms - 7 * DAY_MS), None),
            Self::Today => {
                let offset_ms = utc_offset_secs * 1_000;
                let local = now_ms + offset_ms;
                (Some(local - local.rem_euclid(DAY_MS) - offset_ms), None)
            }
            Self::Custom { from_ms, to_ms } => (from_ms, to_ms.map(|to| to + MINUTE_MS - 1)),
        }
    }

    /// A custom range that ends before it starts matches nothing; the sheet says so.
    pub fn is_inverted(self) -> bool {
        matches!(self, Self::Custom { from_ms: Some(from), to_ms: Some(to) } if to < from)
    }
}

// MARK: Filter

/// What the log sheet is narrowed to. Every part that is left open matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpActivityFilter {
    /// One client, by id (a name can be renamed or reused; an id is neither).
    pub client_id: Option<String>,
    pub status: Option<McpActivityStatus>,
    pub range: McpActivityRange,
    /// A word to find in the tool name, a target id or the error code, in any letter case.
    pub search: String,
}

impl McpActivityFilter {
    /// Whether anything narrows the log.
    pub fn is_active(&self) -> bool {
        self.client_id.is_some()
            || self.status.is_some()
            || self.range != McpActivityRange::Any
            || !self.search.trim().is_empty()
    }

    /// The rows that match, in the order given (newest first, as the host keeps them).
    pub fn select<'a>(
        &self,
        entries: &'a [McpAuditEntry],
        now_ms: i64,
        utc_offset_secs: i64,
    ) -> Vec<&'a McpAuditEntry> {
        if self.range.is_inverted() {
            return Vec::new();
        }
        let (from, to) = self.range.bounds(now_ms, utc_offset_secs);
        let needle = self.search.trim().to_lowercase();
        entries
            .iter()
            .filter(|entry| {
                from.is_none_or(|from| entry.at_ms >= from)
                    && to.is_none_or(|to| entry.at_ms <= to)
                    && self.matches_who_and_how(entry, &needle)
            })
            .collect()
    }

    fn matches_who_and_how(&self, entry: &McpAuditEntry, needle: &str) -> bool {
        let status = entry.status();
        // A marker belongs to no client and has no status or tool: it shows only while nothing
        // narrows the log by any of those.
        if status.is_none() {
            return self.client_id.is_none() && self.status.is_none() && needle.is_empty();
        }
        if self.status.is_some_and(|wanted| status != Some(wanted)) {
            return false;
        }
        if self
            .client_id
            .as_deref()
            .is_some_and(|id| entry.client_id != id)
        {
            return false;
        }
        needle.is_empty() || searchable(entry).contains(needle)
    }
}

/// The lowercase text a search looks through.
fn searchable(entry: &McpAuditEntry) -> String {
    let mut text = entry.tool.to_lowercase();
    // "get_candles" is found by "get candles", as the sheet shows it.
    text.push(' ');
    text.push_str(&entry.tool.replace('_', " ").to_lowercase());
    text.push(' ');
    text.push_str(&entry.code.to_lowercase());
    for target in &entry.targets {
        text.push(' ');
        text.push_str(&target.id.to_lowercase());
    }
    text
}

// MARK: Clients

/// A client that appears in the log, for the client filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpActivityClient {
    pub id: String,
    /// The name to list it by: its newest name in the log, with a short id added when two
    /// clients share one.
    pub label: String,
    /// How many rows it has in the log.
    pub rows: usize,
}

/// The clients that have rows in `entries` (newest first), by name. A client revoked since is
/// still listed, under the name it had.
pub fn clients_in(entries: &[McpAuditEntry]) -> Vec<McpActivityClient> {
    let mut seen: HashMap<&str, (String, usize)> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for entry in entries {
        if entry.client_id.is_empty() || entry.status().is_none() {
            continue;
        }
        let slot = seen.entry(entry.client_id.as_str()).or_insert_with(|| {
            order.push(entry.client_id.as_str());
            (entry.client_name.clone(), 0)
        });
        slot.1 += 1;
    }
    let mut clients: Vec<McpActivityClient> = order
        .into_iter()
        .map(|id| {
            let (name, rows) = &seen[id];
            McpActivityClient {
                id: id.to_owned(),
                label: if name.trim().is_empty() {
                    "Unnamed client".to_owned()
                } else {
                    name.clone()
                },
                rows: *rows,
            }
        })
        .collect();
    // Two clients with one name stay tellable apart.
    let mut by_label: HashMap<String, usize> = HashMap::new();
    for client in &clients {
        *by_label.entry(client.label.to_lowercase()).or_default() += 1;
    }
    for client in &mut clients {
        if by_label[&client.label.to_lowercase()] > 1 {
            let short: String = client.id.chars().take(6).collect();
            client.label = format!("{} ({short})", client.label);
        }
    }
    clients.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    clients
}

// MARK: Paging

/// Where `page` (counted from 0) falls in `total` items of `size` per page, with the page held
/// to the pages that exist: asking for page 9 of 3 gives page 2, and nothing gives page 0 of 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paging {
    pub page: usize,
    pub pages: usize,
    pub total: usize,
    /// The items on this page, as indices into the whole.
    pub items: Range<usize>,
}

impl Paging {
    pub fn new(total: usize, page: usize, size: usize) -> Self {
        let size = size.max(1);
        let pages = total.div_ceil(size).max(1);
        let page = page.min(pages - 1);
        let start = page * size;
        Self {
            page,
            pages,
            total,
            items: start..(start + size).min(total),
        }
    }

    pub fn has_previous(&self) -> bool {
        self.page > 0
    }

    pub fn has_next(&self) -> bool {
        self.page + 1 < self.pages
    }

    /// "26–50 of 163", or "3 of 3", or "0".
    pub fn range_text(&self) -> String {
        match self.items.len() {
            0 => "0".to_owned(),
            1 => format!("{} of {}", self.items.start + 1, self.total),
            _ => format!(
                "{}–{} of {}",
                self.items.start + 1,
                self.items.end,
                self.total
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mcp::McpAuditTarget;

    fn call(
        id: u64,
        client: &str,
        name: &str,
        outcome: McpAuditOutcome,
        at_ms: i64,
    ) -> McpAuditEntry {
        McpAuditEntry {
            id,
            at_ms,
            client_id: client.into(),
            client_name: name.into(),
            tool: "degenview_get_candles".into(),
            outcome,
            count: 1,
            ..McpAuditEntry::default()
        }
    }

    fn ids(rows: &[&McpAuditEntry]) -> Vec<u64> {
        rows.iter().map(|row| row.id).collect()
    }

    #[test]
    fn every_outcome_maps_to_one_status_and_markers_to_none() {
        use McpActivityStatus as S;
        let status = |outcome| call(1, "a", "A", outcome, 0).status();
        assert_eq!(status(McpAuditOutcome::Succeeded), Some(S::Done));
        assert_eq!(status(McpAuditOutcome::Failed), Some(S::Failed));
        assert_eq!(status(McpAuditOutcome::Denied), Some(S::Denied));
        assert_eq!(status(McpAuditOutcome::Cancelled), Some(S::Cancelled));
        assert_eq!(status(McpAuditOutcome::Held), Some(S::Held));
        assert_eq!(status(McpAuditOutcome::Approved), Some(S::Approved));
        assert_eq!(status(McpAuditOutcome::Declined), Some(S::Declined));
        assert_eq!(status(McpAuditOutcome::Expired), Some(S::Expired));
        let marker = |kind| McpAuditEntry {
            kind,
            ..McpAuditEntry::default()
        };
        assert_eq!(
            marker(McpAuditKind::RateLimited).status(),
            Some(S::RateLimited)
        );
        assert_eq!(marker(McpAuditKind::Cleared).status(), None);
        assert_eq!(marker(McpAuditKind::Gap).status(), None);
        // The filter lists each status once, each with its own word.
        let labels: std::collections::HashSet<_> =
            S::ALL.iter().map(|status| status.label()).collect();
        assert_eq!(labels.len(), S::ALL.len());
    }

    #[test]
    fn an_open_filter_keeps_every_row_in_order() {
        let rows = [
            call(3, "a", "A", McpAuditOutcome::Succeeded, 30),
            call(2, "b", "B", McpAuditOutcome::Failed, 20),
            call(1, "a", "A", McpAuditOutcome::Denied, 10),
        ];
        let filter = McpActivityFilter::default();
        assert!(!filter.is_active());
        assert_eq!(ids(&filter.select(&rows, 100, 0)), [3, 2, 1]);
    }

    #[test]
    fn client_and_status_narrow_together() {
        let rows = [
            call(4, "a", "A", McpAuditOutcome::Failed, 40),
            call(3, "b", "B", McpAuditOutcome::Failed, 30),
            call(2, "a", "A", McpAuditOutcome::Succeeded, 20),
            call(1, "a", "A", McpAuditOutcome::Failed, 10),
        ];
        let mut filter = McpActivityFilter {
            client_id: Some("a".into()),
            ..McpActivityFilter::default()
        };
        assert!(filter.is_active());
        assert_eq!(ids(&filter.select(&rows, 100, 0)), [4, 2, 1]);
        filter.status = Some(McpActivityStatus::Failed);
        assert_eq!(ids(&filter.select(&rows, 100, 0)), [4, 1]);
        filter.client_id = Some("nobody".into());
        assert!(filter.select(&rows, 100, 0).is_empty());
    }

    #[test]
    fn markers_show_only_while_nothing_narrows_who_or_how() {
        let cleared = McpAuditEntry {
            id: 9,
            at_ms: 50,
            kind: McpAuditKind::Cleared,
            ..McpAuditEntry::default()
        };
        let rows = [cleared, call(1, "a", "A", McpAuditOutcome::Succeeded, 10)];
        let select = |filter: &McpActivityFilter| ids(&filter.select(&rows, 100, 0));
        assert_eq!(select(&McpActivityFilter::default()), [9, 1]);
        // A date range still applies to them.
        let recent = McpActivityFilter {
            range: McpActivityRange::Custom {
                from_ms: Some(40),
                to_ms: None,
            },
            ..McpActivityFilter::default()
        };
        assert_eq!(select(&recent), [9]);
        for narrowed in [
            McpActivityFilter {
                client_id: Some("a".into()),
                ..McpActivityFilter::default()
            },
            McpActivityFilter {
                status: Some(McpActivityStatus::Done),
                ..McpActivityFilter::default()
            },
            McpActivityFilter {
                search: "candles".into(),
                ..McpActivityFilter::default()
            },
        ] {
            assert_eq!(select(&narrowed), [1], "{narrowed:?}");
        }
    }

    #[test]
    fn the_ranges_resolve_against_the_clock_and_the_time_zone() {
        let now = 10 * DAY_MS + 15 * HOUR_MS + 30 * MINUTE_MS;
        assert_eq!(McpActivityRange::Any.bounds(now, 0), (None, None));
        assert_eq!(
            McpActivityRange::LastHour.bounds(now, 0),
            (Some(now - HOUR_MS), None)
        );
        assert_eq!(
            McpActivityRange::Last24Hours.bounds(now, 0),
            (Some(now - DAY_MS), None)
        );
        assert_eq!(
            McpActivityRange::Last7Days.bounds(now, 0),
            (Some(now - 7 * DAY_MS), None)
        );
        // Midnight UTC, and midnight two hours east of it (which is 22:00 UTC the day before).
        assert_eq!(
            McpActivityRange::Today.bounds(now, 0),
            (Some(10 * DAY_MS), None)
        );
        assert_eq!(
            McpActivityRange::Today.bounds(now, 2 * 3_600),
            (Some(10 * DAY_MS - 2 * HOUR_MS), None)
        );
        // 22:00 UTC is already tomorrow three hours east: today began at 21:00 UTC.
        let late = 10 * DAY_MS + 22 * HOUR_MS;
        assert_eq!(
            McpActivityRange::Today.bounds(late, 3 * 3_600),
            (Some(10 * DAY_MS + 21 * HOUR_MS), None)
        );
        // West of UTC, before the local midnight, it is still yesterday there.
        assert_eq!(
            McpActivityRange::Today.bounds(10 * DAY_MS + HOUR_MS, -5 * 3_600),
            (Some(9 * DAY_MS + 5 * HOUR_MS), None)
        );
    }

    #[test]
    fn a_custom_range_includes_its_last_minute_and_may_be_open_ended() {
        let range = McpActivityRange::Custom {
            from_ms: Some(1_000),
            to_ms: Some(2 * MINUTE_MS),
        };
        assert_eq!(range.bounds(0, 0), (Some(1_000), Some(3 * MINUTE_MS - 1)));
        let rows = [
            call(3, "a", "A", McpAuditOutcome::Succeeded, 3 * MINUTE_MS - 1),
            call(2, "a", "A", McpAuditOutcome::Succeeded, 3 * MINUTE_MS),
            call(1, "a", "A", McpAuditOutcome::Succeeded, 999),
        ];
        let filter = McpActivityFilter {
            range,
            ..McpActivityFilter::default()
        };
        assert_eq!(ids(&filter.select(&rows, 0, 0)), [3]);
        let open_start = McpActivityFilter {
            range: McpActivityRange::Custom {
                from_ms: None,
                to_ms: Some(0),
            },
            ..McpActivityFilter::default()
        };
        assert_eq!(ids(&open_start.select(&rows, 0, 0)), [1]);
    }

    #[test]
    fn a_range_that_ends_before_it_starts_matches_nothing() {
        let range = McpActivityRange::Custom {
            from_ms: Some(10 * MINUTE_MS),
            to_ms: Some(5 * MINUTE_MS),
        };
        assert!(range.is_inverted());
        let rows = [call(1, "a", "A", McpAuditOutcome::Succeeded, 7 * MINUTE_MS)];
        let filter = McpActivityFilter {
            range,
            ..McpActivityFilter::default()
        };
        assert!(filter.select(&rows, 0, 0).is_empty());
        // The same minute is fine: it is one minute long.
        assert!(
            !McpActivityRange::Custom {
                from_ms: Some(MINUTE_MS),
                to_ms: Some(MINUTE_MS)
            }
            .is_inverted()
        );
    }

    #[test]
    fn search_finds_tool_words_codes_and_target_ids_in_any_case() {
        let mut failed = call(2, "a", "A", McpAuditOutcome::Failed, 20);
        failed.code = "NOT_FOUND".into();
        failed.targets = vec![McpAuditTarget {
            kind: "chart_id".into(),
            id: "AB12".into(),
        }];
        let mut other = call(1, "a", "A", McpAuditOutcome::Succeeded, 10);
        other.tool = "degenview_list_alerts".into();
        let rows = [failed, other];
        let find = |word: &str| {
            let filter = McpActivityFilter {
                search: word.into(),
                ..McpActivityFilter::default()
            };
            ids(&filter.select(&rows, 100, 0))
        };
        assert_eq!(find("CANDLES"), [2]);
        assert_eq!(find("get candles"), [2]);
        assert_eq!(find("  alerts "), [1]);
        assert_eq!(find("not_found"), [2]);
        assert_eq!(find("ab12"), [2]);
        assert_eq!(find("degenview"), [2, 1]);
        assert!(find("zzz").is_empty());
        // Blank is no search at all.
        assert_eq!(find("   "), [2, 1]);
        assert!(
            !McpActivityFilter {
                search: "  ".into(),
                ..McpActivityFilter::default()
            }
            .is_active()
        );
    }

    #[test]
    fn clients_are_listed_by_name_with_their_newest_name_and_a_count() {
        let marker = McpAuditEntry {
            kind: McpAuditKind::Cleared,
            ..McpAuditEntry::default()
        };
        let rows = [
            call(5, "b1", "automation", McpAuditOutcome::Succeeded, 50),
            call(4, "a1", "Claude (renamed)", McpAuditOutcome::Succeeded, 40),
            marker,
            call(3, "a1", "Claude", McpAuditOutcome::Failed, 30),
            call(2, "b1", "automation", McpAuditOutcome::Denied, 20),
            call(1, "c1", "", McpAuditOutcome::Succeeded, 10),
        ];
        let clients = clients_in(&rows);
        let summary: Vec<_> = clients
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str(), c.rows))
            .collect();
        // By name, ignoring case: automation, Claude (renamed), Unnamed client.
        assert_eq!(
            summary,
            [
                ("b1", "automation", 2),
                ("a1", "Claude (renamed)", 2),
                ("c1", "Unnamed client", 1),
            ]
        );
        assert!(clients_in(&[]).is_empty());
    }

    #[test]
    fn two_clients_with_one_name_are_told_apart_by_their_ids() {
        let rows = [
            call(2, "abcdef123", "Claude", McpAuditOutcome::Succeeded, 20),
            call(1, "uvwxyz987", "claude", McpAuditOutcome::Succeeded, 10),
        ];
        let labels: Vec<_> = clients_in(&rows).into_iter().map(|c| c.label).collect();
        assert_eq!(labels, ["Claude (abcdef)", "claude (uvwxyz)"]);
    }

    #[test]
    fn paging_holds_the_page_to_the_pages_that_exist() {
        let paging = Paging::new(163, 0, 25);
        assert_eq!((paging.page, paging.pages), (0, 7));
        assert_eq!(paging.items, 0..25);
        assert!(!paging.has_previous() && paging.has_next());
        assert_eq!(paging.range_text(), "1–25 of 163");

        let last = Paging::new(163, 6, 25);
        assert_eq!(last.items, 150..163);
        assert!(last.has_previous() && !last.has_next());
        assert_eq!(last.range_text(), "151–163 of 163");

        // Past the end is the last page: a filter that shrank the log cannot strand the sheet.
        assert_eq!(Paging::new(163, 99, 25), last);
        // Exactly full pages do not grow an empty one.
        let exact = Paging::new(50, 1, 25);
        assert_eq!((exact.pages, exact.items.clone()), (2, 25..50));
        assert_eq!(Paging::new(50, 2, 25), exact);
    }

    #[test]
    fn paging_nothing_and_one_row_are_still_a_page() {
        let none = Paging::new(0, 3, 25);
        assert_eq!((none.page, none.pages, none.items.clone()), (0, 1, 0..0));
        assert!(!none.has_previous() && !none.has_next());
        assert_eq!(none.range_text(), "0");
        let one = Paging::new(1, 0, 25);
        assert_eq!(one.range_text(), "1 of 1");
        // A zero page size cannot divide by zero or loop.
        assert_eq!(Paging::new(5, 0, 0).pages, 5);
    }
}
