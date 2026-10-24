//! What the Pine logs pane shows of a script's messages: which of them pass the filters, where
//! the search matched inside them, and how tall their rows are.
//!
//! Pure data, no egui. The pane owns the widgets; this owns the questions about the messages, so
//! each can be answered once per change instead of once per frame.

use crate::pine::logs::{LogBuffer, LogEvent, LogLevel};
use crate::pine::regex_lite::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;

/// The most matches one message highlights.
pub const MAX_HIGHLIGHTS: usize = 200;

/// Where a message's call is, for the Script Manager to open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceTarget {
    /// The saved script the chart's instance came from.
    pub script_id: Option<String>,
    /// The library (by name) whose code made the call, instead.
    pub library: Option<String>,
    /// The call's position in that source, in UTF-16 units.
    pub offset: usize,
    /// The text the script ran, to tell when the saved script has changed since.
    pub ran_source: Option<String>,
}

// MARK: Search

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// What separates a whole word from its neighbours besides white space (TradingView's set).
fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '.' | ',' | ':' | ';' | '\'' | '"')
}

/// A search compiled once and run over many messages.
pub enum Search {
    /// Nothing to look for: every message matches.
    Off,
    /// A regular expression that does not compile: it filters nothing, and the field says so.
    Invalid,
    Text {
        /// Folded to lower case unless the search matches case.
        needle: String,
        match_case: bool,
        whole_word: bool,
    },
    Pattern {
        regex: Regex,
        whole_word: bool,
    },
}

impl Search {
    pub fn new(query: &str, options: SearchOptions) -> Self {
        if query.is_empty() {
            return Self::Off;
        }
        if options.regex {
            return match Regex::new(query) {
                Some(regex) => Self::Pattern {
                    regex,
                    whole_word: options.whole_word,
                },
                None => Self::Invalid,
            };
        }
        Self::Text {
            needle: if options.match_case {
                query.to_owned()
            } else {
                query.to_ascii_lowercase()
            },
            match_case: options.match_case,
            whole_word: options.whole_word,
        }
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid)
    }

    /// Whether `text` passes: always, for a search that is off or does not compile.
    pub fn admits(&self, text: &str) -> bool {
        match self {
            Self::Off | Self::Invalid => true,
            _ => !self.find(text, 1).is_empty(),
        }
    }

    /// The byte ranges of the matches in `text`, at most `limit`, on character boundaries.
    pub fn find(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        match self {
            Self::Off | Self::Invalid => Vec::new(),
            Self::Text {
                needle,
                match_case,
                whole_word,
            } => find_text(text, needle, *match_case, *whole_word, limit),
            Self::Pattern { regex, whole_word } => {
                // Whole words drop matches, so ask for more than the limit and cut afterwards.
                let ask = if *whole_word {
                    limit.saturating_mul(4).max(16)
                } else {
                    limit
                };
                let mut found = regex.find_all(text, ask);
                if *whole_word {
                    found.retain(|range| is_whole_word(text, range));
                }
                found.truncate(limit);
                found
            }
        }
    }
}

fn is_whole_word(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let after = text[range.end..].chars().next();
    before.is_none_or(is_separator) && after.is_none_or(is_separator)
}

fn find_text(
    text: &str,
    needle: &str,
    match_case: bool,
    whole_word: bool,
    limit: usize,
) -> Vec<Range<usize>> {
    // A term that holds a separator can never stand between two of them.
    if needle.is_empty() || (whole_word && needle.chars().any(is_separator)) {
        return Vec::new();
    }
    // ASCII folding keeps every byte offset, and a valid needle can only match at a character
    // boundary of valid text.
    let folded;
    let haystack = if match_case {
        text
    } else {
        folded = text.to_ascii_lowercase();
        folded.as_str()
    };
    let mut found = Vec::new();
    let mut from = 0;
    while found.len() < limit
        && let Some(at) = haystack[from..].find(needle)
    {
        let range = from + at..from + at + needle.len();
        from = range.end;
        if !whole_word || is_whole_word(text, &range) {
            found.push(range);
        }
    }
    found
}

// MARK: Filters

/// The levels the pane lists.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Levels {
    pub info: bool,
    pub warning: bool,
    pub error: bool,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            info: true,
            warning: true,
            error: true,
        }
    }
}

impl Levels {
    pub fn shows(self, level: LogLevel) -> bool {
        match level {
            LogLevel::Info => self.info,
            LogLevel::Warning => self.warning,
            LogLevel::Error => self.error,
        }
    }

    pub fn toggle(&mut self, level: LogLevel) {
        let flag = match level {
            LogLevel::Info => &mut self.info,
            LogLevel::Warning => &mut self.warning,
            LogLevel::Error => &mut self.error,
        };
        *flag = !*flag;
    }

    pub fn all(self) -> bool {
        self.info && self.warning && self.error
    }
}

/// The options kept between launches.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogsPrefs {
    pub levels: Levels,
    pub search: SearchOptions,
}

/// What a message must satisfy to be listed. Filtering never touches the messages themselves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogFilter {
    pub levels: Levels,
    /// Only messages at or after this instant (ms since the epoch).
    pub start_ms: Option<i64>,
    pub query: String,
    pub options: SearchOptions,
}

impl LogFilter {
    fn admits(&self, event: &LogEvent) -> bool {
        self.levels.shows(event.level) && self.start_ms.is_none_or(|start| event.time_ms >= start)
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.levels.hash(&mut hasher);
        self.start_ms.hash(&mut hasher);
        self.query.hash(&mut hasher);
        self.options.hash(&mut hasher);
        hasher.finish()
    }
}

/// The listed messages of one history, as indices into its buffer, kept between frames and
/// brought up to date only when something changed: new messages (scanned alone when they were
/// appended), a filter, another script, or a restart.
pub struct FilteredLogs {
    key: Option<(u64, u64)>,
    first_sequence: Option<u64>,
    /// How many buffer messages were looked at, and the sequence of the last of them.
    scanned: usize,
    last_scanned: Option<u64>,
    search: Search,
    indices: Vec<u32>,
    /// Counts the times the list changed, so a reader can tell without comparing it.
    revision: u64,
    /// Whether the last change rebuilt the list, or only added rows to its end.
    rebuilt: bool,
}

impl Default for FilteredLogs {
    fn default() -> Self {
        Self {
            key: None,
            first_sequence: None,
            scanned: 0,
            last_scanned: None,
            search: Search::Off,
            indices: Vec::new(),
            revision: 0,
            rebuilt: true,
        }
    }
}

impl FilteredLogs {
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn search(&self) -> &Search {
        &self.search
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the last change rebuilt the list. False: rows were only added to its end.
    pub fn rebuilt(&self) -> bool {
        self.rebuilt
    }

    /// Brings the list up to date. True when it changed.
    pub fn refresh(&mut self, epoch: u64, buffer: &LogBuffer, filter: &LogFilter) -> bool {
        let key = (epoch, filter.fingerprint());
        let first_sequence = buffer.get(0).map(|event| event.sequence);
        let appended = self.key == Some(key)
            && self.first_sequence == first_sequence
            && self.scanned > 0
            && self.scanned <= buffer.len()
            && buffer.get(self.scanned - 1).map(|event| event.sequence) == self.last_scanned;
        if appended && self.scanned == buffer.len() {
            return false;
        }
        let start = if appended {
            self.scanned
        } else {
            self.search = Search::new(&filter.query, filter.options);
            self.indices.clear();
            0
        };
        for index in start..buffer.len() {
            let Some(event) = buffer.get(index) else {
                break;
            };
            if filter.admits(event) && self.search.admits(&event.message) {
                self.indices.push(index as u32);
            }
        }
        self.rebuilt = !appended;
        self.key = Some(key);
        self.first_sequence = first_sequence;
        self.scanned = buffer.len();
        self.last_scanned = buffer.get(buffer.len().wrapping_sub(1)).map(|e| e.sequence);
        self.revision += 1;
        true
    }
}

// MARK: Row heights

/// The heights of the listed rows. A message's lines are only known once it is laid out, so a
/// row starts at an estimate and takes its measured height when it is drawn; what was measured is
/// remembered by message, so a filter that lists it again does not forget.
#[derive(Default)]
pub struct RowHeights {
    width: f32,
    measured: HashMap<u64, f32>,
    heights: Vec<f32>,
    /// `prefix[i]` is the top of row `i`; the last entry is the total.
    prefix: Vec<f32>,
    dirty: bool,
}

/// The most measured heights kept.
const MEASURED_LIMIT: usize = 100_000;

impl RowHeights {
    /// Starts over for these rows (message sequence, estimated height). A new width forgets what
    /// was measured: wrapping changed.
    pub fn reset(&mut self, width: f32, rows: impl Iterator<Item = (u64, f32)>) {
        if (width - self.width).abs() > 0.5 || self.measured.len() > MEASURED_LIMIT {
            self.measured.clear();
            self.width = width;
        }
        self.heights.clear();
        for (sequence, estimate) in rows {
            self.heights
                .push(self.measured.get(&sequence).copied().unwrap_or(estimate));
        }
        self.dirty = true;
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    /// Adds rows to the end.
    pub fn extend(&mut self, rows: impl Iterator<Item = (u64, f32)>) {
        for (sequence, estimate) in rows {
            self.heights
                .push(self.measured.get(&sequence).copied().unwrap_or(estimate));
        }
        self.dirty = true;
    }

    /// Records the laid-out height of row `index`.
    pub fn measure(&mut self, index: usize, sequence: u64, height: f32) {
        let Some(slot) = self.heights.get_mut(index) else {
            return;
        };
        self.measured.insert(sequence, height);
        if (*slot - height).abs() > 0.01 {
            *slot = height;
            self.dirty = true;
        }
    }

    fn rebuild(&mut self) {
        if !self.dirty {
            return;
        }
        self.prefix.clear();
        self.prefix.reserve(self.heights.len() + 1);
        let mut top = 0.0;
        self.prefix.push(top);
        for height in &self.heights {
            top += height;
            self.prefix.push(top);
        }
        self.dirty = false;
    }

    pub fn total(&mut self) -> f32 {
        self.rebuild();
        self.prefix.last().copied().unwrap_or(0.0)
    }

    pub fn top_of(&mut self, index: usize) -> f32 {
        self.rebuild();
        self.prefix
            .get(index)
            .or(self.prefix.last())
            .copied()
            .unwrap_or(0.0)
    }

    pub fn height_of(&self, index: usize) -> f32 {
        self.heights.get(index).copied().unwrap_or(0.0)
    }

    /// The rows a window from `top` to `bottom` touches.
    pub fn rows_between(&mut self, top: f32, bottom: f32) -> Range<usize> {
        self.rebuild();
        let count = self.heights.len();
        if count == 0 {
            return 0..0;
        }
        // The first row whose bottom is below `top`, and the first whose top is below `bottom`.
        let first = self.prefix[1..].partition_point(|end| *end <= top);
        let last = self.prefix[..count].partition_point(|start| *start < bottom);
        first.min(count)..last.max(first.min(count)).min(count)
    }
}

/// An instant as ISO 8601 in UTC, to the millisecond: `2026-10-24T12:00:00.000Z`.
pub fn iso_utc(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = crate::format::civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60,
        epoch_ms.rem_euclid(1_000)
    )
}

/// The height a message's row takes before it is laid out: its lines, wrapped roughly.
pub fn estimated_height(message: &str, line: f32, padding: f32, chars_per_line: usize) -> f32 {
    let lines: usize = message
        .split('\n')
        .map(|part| part.chars().count().div_ceil(chars_per_line.max(1)).max(1))
        .sum();
    lines as f32 * line + padding
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::SourceRange;

    fn event(sequence: u64, level: LogLevel, message: &str, time_ms: i64) -> LogEvent {
        LogEvent {
            sequence,
            level,
            message: message.to_owned(),
            time_ms,
            bar_time_ms: time_ms,
            bar_index: Some(sequence as i64),
            realtime: false,
            confirmed: true,
            library: None,
            range: SourceRange::ZERO,
            request: None,
        }
    }

    fn buffer(messages: &[(LogLevel, &str)]) -> LogBuffer {
        let mut buffer = LogBuffer::default();
        for (index, (level, message)) in messages.iter().enumerate() {
            buffer.push(event(index as u64, *level, message, index as i64 * 1_000));
        }
        buffer
    }

    fn opts(match_case: bool, whole_word: bool, regex: bool) -> SearchOptions {
        SearchOptions {
            match_case,
            whole_word,
            regex,
        }
    }

    fn texts(search: &Search, text: &str) -> Vec<String> {
        search
            .find(text, MAX_HIGHLIGHTS)
            .into_iter()
            .map(|range| text[range].to_owned())
            .collect()
    }

    // MARK: Search

    #[test]
    fn the_default_search_ignores_ascii_case_and_matches_inside_words() {
        let search = Search::new("bar", SearchOptions::default());
        assert_eq!(texts(&search, "Bar 1, crowbar, BAR"), ["Bar", "bar", "BAR"]);
    }

    #[test]
    fn match_case_makes_the_search_exact() {
        let search = Search::new("Bar", opts(true, false, false));
        assert_eq!(texts(&search, "Bar bar BAR"), ["Bar"]);
    }

    #[test]
    fn case_folding_is_ascii_only() {
        // `É` and `é` differ, as TradingView's non-ASCII letters do.
        let search = Search::new("é", SearchOptions::default());
        assert_eq!(texts(&search, "É é"), ["é"]);
    }

    #[test]
    fn highlight_ranges_fall_on_character_boundaries_in_unicode_text() {
        let text = "日本 ✓ log ✓ 日本";
        let search = Search::new("✓", SearchOptions::default());
        let found = search.find(text, 10);
        assert_eq!(found.len(), 2);
        for range in found {
            assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
            assert_eq!(&text[range], "✓");
        }
    }

    #[test]
    fn a_whole_word_is_set_off_by_white_space_or_the_listed_punctuation() {
        let search = Search::new("bar", opts(false, true, false));
        assert_eq!(
            texts(&search, "bar, (bar) foo.bar 'bar' crowbar bars bar:1"),
            ["bar", "bar", "bar", "bar"]
        );
        // "(bar)" is not set off by the brackets, so only the four set off ones match.
        assert!(!search.admits("crowbar bars"));
    }

    #[test]
    fn a_whole_word_term_that_holds_a_separator_never_matches() {
        let search = Search::new("a.b", opts(false, true, false));
        assert!(!search.admits("a.b"));
        assert!(Search::new("a.b", SearchOptions::default()).admits("a.b"));
    }

    #[test]
    fn a_regex_finds_every_match_with_its_range() {
        let search = Search::new("[0-9]+", opts(false, false, true));
        assert_eq!(texts(&search, "bar 12, close=3456, x"), ["12", "3456"]);
    }

    #[test]
    fn a_regex_can_be_a_whole_word_search_too() {
        let search = Search::new("[a-z]+[0-9]", opts(false, true, true));
        assert_eq!(texts(&search, "ab1 xab1x cd2."), ["ab1", "cd2"]);
    }

    #[test]
    fn a_regex_that_does_not_compile_is_flagged_and_filters_nothing() {
        let search = Search::new("(unclosed", opts(false, false, true));
        assert!(search.is_invalid());
        assert!(search.admits("anything"));
        assert!(search.find("anything", 10).is_empty());
    }

    #[test]
    fn a_pathological_regex_gives_up_instead_of_hanging() {
        let search = Search::new("(a+)+b", opts(false, false, true));
        let text = "a".repeat(40);
        let started = std::time::Instant::now();
        let _ = search.find(&text, 10);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn the_number_of_highlights_is_capped() {
        let search = Search::new("a", SearchOptions::default());
        assert_eq!(search.find(&"a".repeat(1_000), 7).len(), 7);
    }

    #[test]
    fn an_empty_query_is_off() {
        let search = Search::new("", SearchOptions::default());
        assert!(!search.is_invalid());
        assert!(search.admits("x"));
        assert!(search.find("x", 5).is_empty());
    }

    // MARK: Filters

    fn listed(filtered: &FilteredLogs) -> Vec<u32> {
        filtered.indices().to_vec()
    }

    #[test]
    fn levels_filter_without_touching_the_messages() {
        let data = buffer(&[
            (LogLevel::Info, "a"),
            (LogLevel::Warning, "b"),
            (LogLevel::Error, "c"),
        ]);
        let mut filtered = FilteredLogs::default();
        let mut filter = LogFilter::default();
        filtered.refresh(1, &data, &filter);
        assert_eq!(listed(&filtered), [0, 1, 2]);
        filter.levels.toggle(LogLevel::Warning);
        filtered.refresh(1, &data, &filter);
        assert_eq!(listed(&filtered), [0, 2]);
        filter.levels = Levels {
            info: false,
            warning: false,
            error: false,
        };
        filtered.refresh(1, &data, &filter);
        assert!(filtered.indices().is_empty());
        assert_eq!(data.len(), 3);
    }

    #[test]
    fn the_start_instant_compares_absolute_times() {
        let data = buffer(&[
            (LogLevel::Info, "a"),
            (LogLevel::Info, "b"),
            (LogLevel::Info, "c"),
        ]);
        let mut filtered = FilteredLogs::default();
        let filter = LogFilter {
            start_ms: Some(1_000),
            ..LogFilter::default()
        };
        filtered.refresh(1, &data, &filter);
        assert_eq!(listed(&filtered), [1, 2]);
    }

    #[test]
    fn filters_compose() {
        let data = buffer(&[
            (LogLevel::Info, "bar 1"),
            (LogLevel::Error, "bar 2"),
            (LogLevel::Error, "baz 3"),
            (LogLevel::Error, "bar 4"),
        ]);
        let filter = LogFilter {
            levels: Levels {
                info: false,
                ..Levels::default()
            },
            start_ms: Some(2_000),
            query: "bar".into(),
            options: SearchOptions::default(),
        };
        let mut filtered = FilteredLogs::default();
        filtered.refresh(1, &data, &filter);
        assert_eq!(listed(&filtered), [3]);
    }

    #[test]
    fn appended_messages_are_scanned_alone() {
        let mut data = buffer(&[(LogLevel::Info, "a"), (LogLevel::Error, "b")]);
        let filter = LogFilter {
            levels: Levels {
                info: false,
                ..Levels::default()
            },
            ..LogFilter::default()
        };
        let mut filtered = FilteredLogs::default();
        assert!(filtered.refresh(1, &data, &filter));
        let revision = filtered.revision();
        assert!(!filtered.refresh(1, &data, &filter), "nothing changed");
        assert_eq!(filtered.revision(), revision);
        data.push(event(2, LogLevel::Error, "c", 2_000));
        data.push(event(3, LogLevel::Info, "d", 3_000));
        assert!(filtered.refresh(1, &data, &filter));
        assert_eq!(listed(&filtered), [1, 2]);
    }

    #[test]
    fn another_run_or_filter_rebuilds() {
        let data = buffer(&[(LogLevel::Info, "a"), (LogLevel::Info, "b")]);
        let mut filtered = FilteredLogs::default();
        let mut filter = LogFilter::default();
        filtered.refresh(1, &data, &filter);
        filter.query = "b".into();
        assert!(filtered.refresh(1, &data, &filter));
        assert_eq!(listed(&filtered), [1]);
        let other = buffer(&[(LogLevel::Info, "b")]);
        assert!(filtered.refresh(2, &other, &filter));
        assert_eq!(listed(&filtered), [0]);
    }

    #[test]
    fn an_eviction_from_the_front_rebuilds_instead_of_shifting_indices_wrongly() {
        let mut data = LogBuffer::default();
        for sequence in 0..(crate::pine::logs::HISTORICAL_LIMIT as u64) {
            data.push(event(sequence, LogLevel::Info, "m", 0));
        }
        let filter = LogFilter {
            query: "m".into(),
            ..LogFilter::default()
        };
        let mut filtered = FilteredLogs::default();
        filtered.refresh(1, &data, &filter);
        data.push(event(10_000, LogLevel::Info, "m", 0));
        filtered.refresh(1, &data, &filter);
        assert_eq!(
            filtered.indices().len(),
            crate::pine::logs::HISTORICAL_LIMIT
        );
        assert_eq!(filtered.indices().last().copied(), Some(9_999));
    }

    #[test]
    fn a_late_historical_message_inserted_before_realtime_ones_rebuilds() {
        let mut data = LogBuffer::default();
        data.push(event(0, LogLevel::Info, "h0", 0));
        let mut realtime = event(1, LogLevel::Info, "r1", 0);
        realtime.realtime = true;
        data.push(realtime);
        let filter = LogFilter {
            query: "h".into(),
            ..LogFilter::default()
        };
        let mut filtered = FilteredLogs::default();
        filtered.refresh(1, &data, &filter);
        assert_eq!(listed(&filtered), [0]);
        data.push(event(2, LogLevel::Info, "h2", 0));
        filtered.refresh(1, &data, &filter);
        // Buffer order is h0, h2, r1.
        assert_eq!(listed(&filtered), [0, 1]);
    }

    #[test]
    fn high_volume_filtering_is_one_pass() {
        let mut data = LogBuffer::default();
        for sequence in 0..10_000u64 {
            data.push(event(sequence, LogLevel::Info, "close 101.5 bar", 0));
        }
        let filter = LogFilter {
            query: "BAR".into(),
            ..LogFilter::default()
        };
        let mut filtered = FilteredLogs::default();
        let started = std::time::Instant::now();
        filtered.refresh(1, &data, &filter);
        assert_eq!(filtered.indices().len(), 10_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    // MARK: Row heights

    #[test]
    fn rows_between_finds_the_rows_a_window_touches() {
        let mut heights = RowHeights::default();
        heights.reset(
            300.0,
            [(0, 10.0), (1, 20.0), (2, 10.0), (3, 30.0)].into_iter(),
        );
        assert_eq!(heights.total(), 70.0);
        assert_eq!(heights.rows_between(0.0, 5.0), 0..1);
        assert_eq!(heights.rows_between(9.0, 11.0), 0..2);
        assert_eq!(heights.rows_between(30.0, 70.0), 2..4);
        assert_eq!(heights.rows_between(500.0, 600.0), 4..4);
        assert_eq!(heights.top_of(3), 40.0);
    }

    #[test]
    fn a_measured_height_replaces_the_estimate_and_is_remembered() {
        let mut heights = RowHeights::default();
        heights.reset(300.0, [(7, 20.0), (8, 20.0)].into_iter());
        heights.measure(0, 7, 55.0);
        assert_eq!(heights.total(), 75.0);
        // The same message listed again after a filter change keeps its measurement.
        heights.reset(300.0, [(8, 20.0), (7, 20.0)].into_iter());
        assert_eq!(heights.height_of(1), 55.0);
    }

    #[test]
    fn a_new_width_forgets_what_was_measured() {
        let mut heights = RowHeights::default();
        heights.reset(300.0, [(1, 20.0)].into_iter());
        heights.measure(0, 1, 60.0);
        heights.reset(200.0, [(1, 20.0)].into_iter());
        assert_eq!(heights.height_of(0), 20.0);
    }

    #[test]
    fn empty_heights_touch_no_rows() {
        let mut heights = RowHeights::default();
        assert_eq!(heights.rows_between(0.0, 100.0), 0..0);
        assert_eq!(heights.total(), 0.0);
    }

    #[test]
    fn the_estimate_counts_lines_and_wraps() {
        assert_eq!(estimated_height("one", 16.0, 4.0, 40), 20.0);
        assert_eq!(estimated_height("a\nb\nc", 16.0, 4.0, 40), 52.0);
        assert_eq!(estimated_height(&"x".repeat(100), 16.0, 4.0, 40), 52.0);
        assert_eq!(estimated_height("", 16.0, 4.0, 40), 20.0);
    }

    #[test]
    fn instants_read_as_iso_8601_in_utc() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_700_000_123_456), "2023-11-14T22:15:23.456Z");
        assert_eq!(iso_utc(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn the_list_says_whether_it_was_rebuilt_or_extended() {
        let mut data = buffer(&[(LogLevel::Info, "a")]);
        let filter = LogFilter::default();
        let mut filtered = FilteredLogs::default();
        filtered.refresh(1, &data, &filter);
        assert!(filtered.rebuilt());
        data.push(event(1, LogLevel::Info, "b", 0));
        filtered.refresh(1, &data, &filter);
        assert!(!filtered.rebuilt());
        filtered.refresh(2, &data, &filter);
        assert!(filtered.rebuilt());
    }

    #[test]
    fn prefs_read_with_missing_or_unknown_fields() {
        let prefs: LogsPrefs =
            serde_json::from_str(r#"{"levels":{"warning":false},"future":1}"#).unwrap();
        assert!(prefs.levels.info && !prefs.levels.warning && prefs.levels.error);
        assert!(!prefs.search.regex);
    }
}
