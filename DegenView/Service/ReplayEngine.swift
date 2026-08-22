import Foundation
import Combine

/// One deterministic replay clock for a tab. Charts may have different symbols,
/// but are always gated by this single timestamp rather than independent indices.
@MainActor
final class ReplayEngine: ObservableObject {
    @Published private(set) var status: ReplayStatus = .inactive
    @Published private(set) var session: ReplaySession?

    private var timeline: [Date] = []
    private var playbackTask: Task<Void, Never>?
    var onStateChange: (() -> Void)?

    var isActive: Bool { status != .inactive }
    var canAdvance: Bool { status == .paused || status == .playing }
    var currentTimestamp: Date? { session?.currentTimestamp }

    deinit { playbackTask?.cancel() }

    func beginSelecting() {
        pauseLoop()
        status = .selectingStart
        if var session {
            session.status = .selectingStart
            self.session = session
        }
        changed()
    }

    func cancelSelection() {
        if var session {
            session.status = .paused
            self.session = session
            status = .paused
        } else {
            status = .inactive
        }
        changed()
    }

    func start(
        at requested: Date,
        initialCursorAt requestedCursor: Date? = nil,
        symbol: String,
        timeframe: TimeRange,
        timeline rawTimeline: [Date]
    ) {
        let valid = Self.normalized(rawTimeline)
        guard !valid.isEmpty else { return }
        timeline = valid
        let index = Self.index(atOrBefore: requestedCursor ?? requested, in: valid) ?? 0
        let timestamp = valid[index]
        pauseLoop()
        session = ReplaySession(
            status: .paused,
            symbol: symbol,
            chartTimeframe: timeframe,
            startTimestamp: requested,
            currentTimestamp: timestamp,
            currentBarIndex: index,
            replayInterval: session?.replayInterval ?? .automatic,
            playbackSpeed: session?.playbackSpeed ?? .normal,
            sessionStartedAt: Date()
        )
        status = .paused
        changed()
    }

    func restore(_ saved: ReplaySession, timeline rawTimeline: [Date]) {
        let valid = Self.normalized(rawTimeline)
        guard !valid.isEmpty,
              let index = Self.index(atOrBefore: saved.currentTimestamp, in: valid)
        else { stop(); return }
        timeline = valid
        var restored = saved.restoredPaused
        restored.currentBarIndex = index
        restored.currentTimestamp = valid[index]
        session = restored
        status = restored.status
        changed()
    }

    func updateTimeline(_ rawTimeline: [Date], symbol: String, timeframe: TimeRange) {
        guard var session else { return }
        let valid = Self.normalized(rawTimeline)
        guard !valid.isEmpty else { return }
        timeline = valid
        let index = Self.index(atOrBefore: session.currentTimestamp, in: valid) ?? 0
        session.currentBarIndex = index
        session.currentTimestamp = valid[index]
        session.symbol = symbol
        session.chartTimeframe = timeframe
        self.session = session
        changed()
    }

    func play() {
        guard canAdvance, !timeline.isEmpty, playbackTask == nil else { return }
        guard var session else { return }
        session.status = .playing
        self.session = session
        status = .playing
        changed()

        playbackTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                guard let delay = self.session?.playbackSpeed.delayNanoseconds else { return }
                try? await Task.sleep(nanoseconds: delay)
                guard !Task.isCancelled else { return }
                guard self.stepForward(whilePlaying: true) else { return }
                await Task.yield()
            }
        }
    }

    func pause() {
        guard status == .playing else { return }
        pauseLoop()
        guard var session else { return }
        session.status = .paused
        self.session = session
        status = .paused
        changed()
    }

    func togglePlayback() { status == .playing ? pause() : play() }

    @discardableResult
    func stepForward() -> Bool { stepForward(whilePlaying: false) }

    @discardableResult
    private func stepForward(whilePlaying: Bool) -> Bool {
        guard var session, canAdvance else { return false }
        let next = session.currentBarIndex + 1
        guard timeline.indices.contains(next) else {
            pauseLoop()
            session.status = .completed
            self.session = session
            status = .completed
            changed()
            return false
        }
        session.currentBarIndex = next
        session.currentTimestamp = timeline[next]
        session.status = whilePlaying ? .playing : .paused
        self.session = session
        status = session.status
        changed()
        return true
    }

    func seek(to date: Date) {
        guard var session, let index = Self.index(atOrBefore: date, in: timeline) else { return }
        pauseLoop()
        session.currentBarIndex = index
        session.currentTimestamp = timeline[index]
        session.status = index == timeline.count - 1 ? .completed : .paused
        self.session = session
        status = session.status
        changed()
    }

    func restart() {
        guard let start = session?.startTimestamp else { return }
        seek(to: start)
    }

    func setSpeed(_ speed: ReplaySpeed) {
        guard var session else { return }
        session.playbackSpeed = speed
        self.session = session
        if status == .playing { pause(); play() } else { changed() }
    }

    func setInterval(_ interval: ReplayInterval) {
        guard var session else { return }
        session.replayInterval = interval
        self.session = session
        changed()
    }

    func jumpToLatest() { stop() }

    func stop() {
        pauseLoop()
        session = nil
        timeline = []
        status = .inactive
        changed()
    }

    private func pauseLoop() {
        playbackTask?.cancel()
        playbackTask = nil
    }

    private func changed() { onStateChange?() }

    static func normalized(_ dates: [Date]) -> [Date] {
        Array(Set(dates)).sorted()
    }

    static func index(atOrBefore date: Date, in dates: [Date]) -> Int? {
        guard !dates.isEmpty, date >= dates[0] else { return nil }
        var low = 0, high = dates.count
        while low < high {
            let mid = (low + high) / 2
            if dates[mid] <= date { low = mid + 1 } else { high = mid }
        }
        return low - 1
    }

    /// Deterministic OHLCV aggregation for providers that can supply a supported
    /// lower-resolution series. No intrabar ordering is inferred.
    static func aggregate(_ candles: ArraySlice<KlineData>, bucketStart: Date) -> KlineData? {
        guard let first = candles.first, let last = candles.last else { return nil }
        return KlineData(
            openTime: bucketStart,
            openPrice: first.openPrice,
            highPrice: candles.reduce(first.highPrice) { max($0, $1.highPrice) },
            lowPrice: candles.reduce(first.lowPrice) { min($0, $1.lowPrice) },
            closePrice: last.closePrice,
            volume: candles.reduce(0) { $0 + $1.volume },
            quoteVolume: candles.reduce(0) { $0 + $1.quoteVolume }
        )
    }
}
