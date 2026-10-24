//! Pine Logs: the messages `log.info()`, `log.warning()` and `log.error()` write while a script
//! runs, and the bounded buffer they are kept in.
//!
//! A message belongs to the run that wrote it, not to the bar state, so the sink lives on the
//! [`Session`](super::session::Session) next to the emitted alerts: rolling a realtime bar back
//! discards the bar's variables but never its messages.
//!
//! TradingView keeps the last 10,000 messages of the historical bars and appends realtime messages
//! after them; the realtime and per-message limits are DegenView's own safety bounds.

use std::collections::VecDeque;

use super::SourceRange;

/// The most messages of historical bars a script keeps (TradingView's documented limit).
pub const HISTORICAL_LIMIT: usize = 10_000;
/// The most realtime messages a script keeps. Not a TradingView figure.
pub const REALTIME_LIMIT: usize = 50_000;
/// The most messages one realtime execution may write. Not a TradingView figure.
pub const REALTIME_PER_EXECUTION: usize = 1_000;
/// The longest message in characters; a longer one is cut and ends in `…`. Not a TradingView figure.
pub const MESSAGE_LIMIT: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

impl LogLevel {
    pub fn from_function(name: &str) -> Option<Self> {
        match name {
            "log.info" => Some(Self::Info),
            "log.warning" => Some(Self::Warning),
            "log.error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// The data context a message was written in, when it was not the chart's own.
#[derive(Clone, Debug, PartialEq)]
pub struct LogRequest {
    pub symbol: String,
    pub interval_seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogEvent {
    /// Counts every call of the run, including the ones a limit dropped.
    pub sequence: u64,
    pub level: LogLevel,
    pub message: String,
    /// The bar's open time for a historical bar, the wall clock for a realtime one (ms).
    pub time_ms: i64,
    /// The open time of the bar the call ran on (ms): the requested bar inside a request.
    pub bar_time_ms: i64,
    /// The chart bar's index; None inside a request, whose bars are not the chart's.
    pub bar_index: Option<i64>,
    pub realtime: bool,
    pub confirmed: bool,
    /// The library (`user/name/version`) whose code made the call; None for the script itself.
    pub library: Option<String>,
    /// Where the call is in the source of the script or library.
    pub range: SourceRange,
    pub request: Option<LogRequest>,
}

/// The messages written since the last time they were taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogBatch {
    /// The run started over: whatever the reader holds is stale.
    pub reset: bool,
    /// Sequence numbers this batch covers, `from..to`. A reader whose last `to` differs from
    /// `from` missed a batch.
    pub from: u64,
    pub to: u64,
    pub events: Vec<LogEvent>,
    /// Messages a limit dropped in this range.
    pub dropped: u64,
}

impl LogBatch {
    pub fn is_empty(&self) -> bool {
        !self.reset && self.events.is_empty() && self.dropped == 0
    }
}

/// Historical messages first, then realtime ones, each in the order they were written: what a
/// reader shows top to bottom. Both kinds are bounded.
#[derive(Clone, Debug, Default)]
pub struct LogBuffer {
    events: VecDeque<LogEvent>,
    historical: usize,
}

impl LogBuffer {
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn historical_len(&self) -> usize {
        self.historical
    }

    pub fn realtime_len(&self) -> usize {
        self.events.len() - self.historical
    }

    pub fn get(&self, index: usize) -> Option<&LogEvent> {
        self.events.get(index)
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &LogEvent> + ExactSizeIterator {
        self.events.iter()
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.historical = 0;
    }

    /// Adds `event` and returns how many older messages the limits pushed out.
    pub fn push(&mut self, event: LogEvent) -> u64 {
        let mut evicted = 0;
        if event.realtime {
            self.events.push_back(event);
            if self.realtime_len() > REALTIME_LIMIT {
                // A tenth at a time, so a script that logs on every tick pays for the shift rarely.
                let chunk = (REALTIME_LIMIT / 10).max(1);
                self.events.drain(self.historical..self.historical + chunk);
                evicted += chunk as u64;
            }
        } else {
            // A bar that closes while realtime messages exist still belongs before them.
            self.events.insert(self.historical, event);
            self.historical += 1;
            if self.historical > HISTORICAL_LIMIT {
                self.events.pop_front();
                self.historical -= 1;
                evicted += 1;
            }
        }
        evicted
    }
}

/// Where `log.*()` calls are recorded while a script runs. Off until a reader asks for it, so a
/// run nobody looks at (an alert subscription, a test) neither formats nor keeps any message.
#[derive(Clone, Debug, Default)]
pub struct LogSink {
    enabled: bool,
    buffer: LogBuffer,
    next_sequence: u64,
    taken_to: u64,
    reset: bool,
    dropped: u64,
    /// The wall clock the host last reported (ms): the time of a realtime message.
    clock_ms: i64,
    bar: Option<BarFrame>,
    realtime_this_execution: usize,
    /// Positive while a request's expression runs where its repeated evaluations would write the
    /// same message again.
    muted: usize,
    /// The requests whose expressions are running, innermost last.
    requests: Vec<ActiveRequest>,
}

#[derive(Clone, Debug)]
struct ActiveRequest {
    request: LogRequest,
    bar_time_ms: i64,
    muted: bool,
}

#[derive(Clone, Copy, Debug)]
struct BarFrame {
    open_time_ms: i64,
    bar_index: i64,
    realtime: bool,
    confirmed: bool,
}

impl LogSink {
    pub fn enable(&mut self) {
        self.enabled = true;
        // Whoever reads next has to start from this run.
        self.reset = true;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_clock(&mut self, now_ms: i64) {
        self.clock_ms = now_ms;
    }

    /// The run starts over: what was written belongs to the previous run.
    pub fn restart(&mut self) {
        self.buffer.clear();
        self.reset = true;
        self.dropped = 0;
        self.bar = None;
        self.realtime_this_execution = 0;
        self.muted = 0;
        self.requests.clear();
        self.taken_to = self.next_sequence;
    }

    /// A bar of the chart starts executing.
    pub fn begin_bar(
        &mut self,
        open_time_ms: i64,
        bar_index: i64,
        realtime: bool,
        confirmed: bool,
    ) {
        self.bar = Some(BarFrame {
            open_time_ms,
            bar_index,
            realtime,
            confirmed,
        });
        self.realtime_this_execution = 0;
    }

    /// Whether the running chart bar is a realtime one: a request's developing evaluations on
    /// historical bars repeat themselves, on realtime ones they are the live reading.
    pub fn chart_bar_is_realtime(&self) -> bool {
        self.bar.is_some_and(|bar| bar.realtime)
    }

    /// A request's expression is about to run on the bar opened at `bar_time_ms`.
    /// `committed` is false for a developing evaluation, which a historical chart bar repeats.
    pub fn enter_request(&mut self, request: LogRequest, bar_time_ms: i64, committed: bool) {
        let muted = !committed && !self.chart_bar_is_realtime();
        self.muted += usize::from(muted);
        self.requests.push(ActiveRequest {
            request,
            bar_time_ms,
            muted,
        });
    }

    pub fn leave_request(&mut self) {
        if let Some(left) = self.requests.pop() {
            self.muted -= usize::from(left.muted);
        }
    }

    pub fn record(
        &mut self,
        level: LogLevel,
        message: String,
        library: Option<String>,
        range: SourceRange,
    ) {
        if !self.enabled || self.muted > 0 {
            return;
        }
        let Some(bar) = self.bar else { return };
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        if bar.realtime {
            self.realtime_this_execution += 1;
            if self.realtime_this_execution > REALTIME_PER_EXECUTION {
                self.dropped += 1;
                return;
            }
        }
        let active = self.requests.last();
        let event = LogEvent {
            sequence,
            level,
            message: truncated(message),
            time_ms: if bar.realtime {
                self.clock_ms
            } else {
                active.map_or(bar.open_time_ms, |active| active.bar_time_ms)
            },
            bar_time_ms: active.map_or(bar.open_time_ms, |active| active.bar_time_ms),
            bar_index: active.is_none().then_some(bar.bar_index),
            realtime: bar.realtime,
            confirmed: bar.confirmed,
            library,
            range,
            request: active.map(|active| active.request.clone()),
        };
        self.dropped += self.buffer.push(event);
    }

    /// Hands over what was written since the last call.
    pub fn take(&mut self) -> LogBatch {
        let mut events: Vec<LogEvent> = self.buffer.iter().cloned().collect();
        events.sort_by_key(|event| event.sequence);
        self.buffer.clear();
        let batch = LogBatch {
            reset: std::mem::take(&mut self.reset),
            from: self.taken_to,
            to: self.next_sequence,
            events,
            dropped: std::mem::take(&mut self.dropped),
        };
        self.taken_to = self.next_sequence;
        batch
    }
}

fn truncated(message: String) -> String {
    if message.chars().count() <= MESSAGE_LIMIT {
        return message;
    }
    let mut cut: String = message.chars().take(MESSAGE_LIMIT - 1).collect();
    cut.push('…');
    cut
}

/// What a reader holds of one run's messages: the last [`LogBatch`]es applied in order.
#[derive(Clone, Debug, Default)]
pub struct LogHistory {
    buffer: LogBuffer,
    expected_from: u64,
    dropped: u64,
    /// A batch went missing, so the messages before it were thrown away.
    pub gap: bool,
}

impl LogHistory {
    pub fn apply(&mut self, batch: LogBatch) {
        if batch.reset || batch.from != self.expected_from {
            self.buffer.clear();
            self.dropped = 0;
            self.gap = !batch.reset;
        }
        self.expected_from = batch.to;
        self.dropped += batch.dropped;
        for event in batch.events {
            self.dropped += self.buffer.push(event);
        }
    }

    pub fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Messages the limits kept out of this history.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(sequence: u64, realtime: bool) -> LogEvent {
        LogEvent {
            sequence,
            level: LogLevel::Info,
            message: format!("m{sequence}"),
            time_ms: sequence as i64,
            bar_time_ms: sequence as i64,
            bar_index: Some(sequence as i64),
            realtime,
            confirmed: !realtime,
            library: None,
            range: SourceRange::ZERO,
            request: None,
        }
    }

    fn enabled_sink() -> LogSink {
        let mut sink = LogSink::default();
        sink.enable();
        sink
    }

    #[test]
    fn a_disabled_sink_records_nothing() {
        let mut sink = LogSink::default();
        sink.begin_bar(0, 0, false, true);
        sink.record(LogLevel::Info, "x".into(), None, SourceRange::ZERO);
        assert!(sink.take().events.is_empty());
    }

    #[test]
    fn the_historical_buffer_keeps_the_last_ten_thousand() {
        let mut buffer = LogBuffer::default();
        for index in 0..(HISTORICAL_LIMIT as u64 + 25) {
            buffer.push(event(index, false));
        }
        assert_eq!(buffer.historical_len(), HISTORICAL_LIMIT);
        assert_eq!(buffer.get(0).unwrap().sequence, 25);
    }

    #[test]
    fn realtime_messages_follow_the_historical_ones_even_when_a_bar_closes_late() {
        let mut buffer = LogBuffer::default();
        buffer.push(event(0, false));
        buffer.push(event(1, true));
        buffer.push(event(2, false));
        let order: Vec<u64> = buffer.iter().map(|event| event.sequence).collect();
        assert_eq!(order, [0, 2, 1]);
    }

    #[test]
    fn the_realtime_buffer_is_bounded_and_keeps_the_newest() {
        let mut buffer = LogBuffer::default();
        buffer.push(event(0, false));
        for index in 1..=(REALTIME_LIMIT as u64 + 1) {
            buffer.push(event(index, true));
        }
        assert!(buffer.realtime_len() <= REALTIME_LIMIT);
        assert_eq!(buffer.historical_len(), 1);
        assert_eq!(
            buffer.iter().last().unwrap().sequence,
            REALTIME_LIMIT as u64 + 1
        );
    }

    #[test]
    fn a_realtime_execution_may_only_write_so_many() {
        let mut sink = enabled_sink();
        sink.begin_bar(0, 0, true, false);
        for _ in 0..(REALTIME_PER_EXECUTION + 5) {
            sink.record(LogLevel::Info, "x".into(), None, SourceRange::ZERO);
        }
        let batch = sink.take();
        assert_eq!(batch.events.len(), REALTIME_PER_EXECUTION);
        assert_eq!(batch.dropped, 5);
        sink.begin_bar(60_000, 1, true, false);
        sink.record(LogLevel::Info, "y".into(), None, SourceRange::ZERO);
        assert_eq!(sink.take().events.len(), 1);
    }

    #[test]
    fn a_historical_bar_may_write_any_number_up_to_the_buffer() {
        let mut sink = enabled_sink();
        sink.begin_bar(0, 0, false, true);
        for _ in 0..(REALTIME_PER_EXECUTION * 3) {
            sink.record(LogLevel::Info, "x".into(), None, SourceRange::ZERO);
        }
        assert_eq!(sink.take().events.len(), REALTIME_PER_EXECUTION * 3);
    }

    #[test]
    fn a_long_message_is_cut_with_an_ellipsis() {
        let cut = truncated("é".repeat(MESSAGE_LIMIT + 10));
        assert_eq!(cut.chars().count(), MESSAGE_LIMIT);
        assert!(cut.ends_with('…'));
        assert_eq!(truncated("short".into()), "short");
    }

    #[test]
    fn times_follow_the_bar_for_history_and_the_clock_for_realtime() {
        let mut sink = enabled_sink();
        sink.set_clock(999);
        sink.begin_bar(1_000, 3, false, true);
        sink.record(LogLevel::Warning, "h".into(), None, SourceRange::ZERO);
        sink.begin_bar(2_000, 4, true, false);
        sink.record(LogLevel::Error, "r".into(), None, SourceRange::ZERO);
        let events = sink.take().events;
        assert_eq!((events[0].time_ms, events[0].bar_time_ms), (1_000, 1_000));
        assert_eq!((events[1].time_ms, events[1].bar_time_ms), (999, 2_000));
        assert_eq!(events[1].bar_index, Some(4));
        assert!(events[1].realtime && !events[1].confirmed);
    }

    #[test]
    fn a_request_labels_its_messages_and_drops_the_chart_bar_index() {
        let mut sink = enabled_sink();
        sink.begin_bar(1_000, 3, false, true);
        sink.enter_request(
            LogRequest {
                symbol: "X".into(),
                interval_seconds: 3_600.0,
            },
            500,
            true,
        );
        sink.record(LogLevel::Info, "q".into(), None, SourceRange::ZERO);
        sink.leave_request();
        let event = sink.take().events.remove(0);
        assert_eq!(event.bar_index, None);
        assert_eq!(event.bar_time_ms, 500);
        assert_eq!(event.request.unwrap().symbol, "X");
    }

    #[test]
    fn a_developing_request_evaluation_on_a_historical_bar_is_muted() {
        let mut sink = enabled_sink();
        sink.begin_bar(1_000, 3, false, true);
        let request = LogRequest {
            symbol: "X".into(),
            interval_seconds: 60.0,
        };
        sink.enter_request(request.clone(), 0, false);
        sink.record(LogLevel::Info, "again".into(), None, SourceRange::ZERO);
        sink.leave_request();
        assert!(sink.take().events.is_empty());
        sink.begin_bar(2_000, 4, true, false);
        sink.enter_request(request, 0, false);
        sink.record(LogLevel::Info, "live".into(), None, SourceRange::ZERO);
        sink.leave_request();
        assert_eq!(sink.take().events.len(), 1);
    }

    #[test]
    fn batches_cover_consecutive_sequence_ranges() {
        let mut sink = enabled_sink();
        sink.begin_bar(0, 0, false, true);
        sink.record(LogLevel::Info, "a".into(), None, SourceRange::ZERO);
        let first = sink.take();
        sink.record(LogLevel::Info, "b".into(), None, SourceRange::ZERO);
        let second = sink.take();
        assert!(first.reset && !second.reset);
        assert_eq!((first.from, first.to), (0, 1));
        assert_eq!((second.from, second.to), (1, 2));
    }

    #[test]
    fn restarting_marks_the_next_batch_a_reset_and_forgets_the_old_messages() {
        let mut sink = enabled_sink();
        sink.begin_bar(0, 0, false, true);
        sink.record(LogLevel::Info, "old".into(), None, SourceRange::ZERO);
        sink.restart();
        let batch = sink.take();
        assert!(batch.reset);
        assert!(batch.events.is_empty());
    }

    #[test]
    fn a_history_applies_batches_in_order_and_resets_on_demand() {
        let mut sink = enabled_sink();
        let mut history = LogHistory::default();
        sink.begin_bar(0, 0, false, true);
        sink.record(LogLevel::Info, "a".into(), None, SourceRange::ZERO);
        history.apply(sink.take());
        sink.record(LogLevel::Info, "b".into(), None, SourceRange::ZERO);
        history.apply(sink.take());
        assert_eq!(history.len(), 2);
        assert!(!history.gap);
        sink.restart();
        sink.begin_bar(0, 0, false, true);
        sink.record(LogLevel::Info, "c".into(), None, SourceRange::ZERO);
        history.apply(sink.take());
        assert_eq!(history.len(), 1);
        assert_eq!(history.buffer().get(0).unwrap().message, "c");
    }

    #[test]
    fn a_missing_batch_is_detected_and_the_stale_messages_are_not_kept() {
        let mut sink = enabled_sink();
        let mut history = LogHistory::default();
        sink.begin_bar(0, 0, false, true);
        sink.record(LogLevel::Info, "a".into(), None, SourceRange::ZERO);
        history.apply(sink.take());
        sink.record(LogLevel::Info, "lost".into(), None, SourceRange::ZERO);
        let _lost = sink.take();
        sink.record(LogLevel::Info, "c".into(), None, SourceRange::ZERO);
        history.apply(sink.take());
        assert!(history.gap);
        assert_eq!(history.len(), 1);
    }
}
