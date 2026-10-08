import Foundation

/// Drives one script over a market feed: history once, then one execution per qualifying event.
///
/// Pipeline: market update → `PineCandleAggregator` (bar lifecycle) → `PineExecutionScheduler` (does
/// this script run for it?) → `PineRuntimeSession.execute` (rollback, run, commit when confirmed).
/// Everything is driven by the events handed in; nothing here polls or keeps time.
///
/// Confined to one thread at a time, like the session it owns. `PineExecutionHost` provides that.
final class PineExecutionController {
    let program: PineCompiledProgram
    let dataset: PineDatasetKey
    private let session: PineRuntimeSession
    private var aggregator = PineCandleAggregator()
    private var firstOpenTime: Date?
    /// Every committed bar as executed, oldest first, to verify a REST snapshot against.
    private var committedBars: [KlineData] = []
    private var isFailed = false

    init(
        program: PineCompiledProgram, dataset: PineDatasetKey, inputs: [String: PineInputValue] = [:],
        theme: PineChartTheme = .dark, symbol: PineSymbolInfo = PineSymbolInfo(),
        securityData: PineSecurityDataProvider? = nil, visibleRange: ClosedRange<Date>? = nil
    ) {
        self.program = program
        self.dataset = dataset
        session = PineRuntimeSession(
            program: program, inputs: inputs, theme: theme, symbol: symbol, securityData: securityData,
            visibleRange: visibleRange)
    }

    // MARK: - Loading

    /// Discards all state and recalculates over `bars`. Closed bars execute as history. With `live`,
    /// a final bar that is still open (by its open time and the bar length) executes as the first
    /// realtime tick of a new bar. Loading never notifies: alerts raised while rebuilding are dropped
    /// from the update, though the once-per-bar ledger remembers them.
    func rebuild(bars: [KlineData], live: Bool, now: Date = Date()) -> PineExecutionOutcome {
        isFailed = false
        var history = bars
        var forming: KlineData?
        let spacing = Self.spacing(of: bars)
        if live, let last = bars.last, !last.isClosed,
            spacing == 0 || last.openTime.addingTimeInterval(spacing) > now
        {
            forming = history.removeLast()
        }
        let formingRuns =
            forming != nil
            && PineExecutionScheduler.shouldExecute(program.declaration, phase: .realtimeTick(isNew: true))
        do {
            let states = try session.load(history: history, precedesLiveBar: formingRuns, barSeconds: spacing)
            aggregator = PineCandleAggregator(barSeconds: session.barSeconds, lastClosed: history.last)
            firstOpenTime = bars.first?.openTime
            committedBars = history
            var update = PineExecutionUpdate(output: session.output(), executions: states)
            update.barID = history.last.map(barID)
            guard let forming else { return .updated(update) }
            if case .updated(var opened) = process(aggregator.ingest(.init(bar: forming, origin: .poll))) {
                opened.executions = states + opened.executions
                opened.alerts = []
                return .updated(opened)
            }
            return .updated(update)
        } catch {
            return fail(error)
        }
    }

    // MARK: - Realtime

    /// Feeds one observation of a candle.
    func ingest(_ update: PineMarketUpdate) -> PineExecutionOutcome {
        // Until loaded, or after a failure, wait for the next snapshot to rebuild rather than
        // rebuilding on every tick.
        guard !isFailed, firstOpenTime != nil else { return .unchanged }
        return process(aggregator.ingest(update))
    }

    /// Reconciles a REST snapshot with what has been committed: new bars become events, a snapshot
    /// that disagrees with committed history triggers a rebuild, and a stale one is ignored.
    func sync(snapshot: [KlineData], now: Date = Date()) -> PineExecutionOutcome {
        guard let first = snapshot.first, let last = snapshot.last else { return .unchanged }
        guard !isFailed, let firstOpenTime else { return rebuild(bars: snapshot, live: true, now: now) }
        // Older history than the session has (the window grew): bar indexes shift, so start over.
        if first.openTime < firstOpenTime { return rebuild(bars: snapshot, live: true, now: now) }

        let tail: ArraySlice<KlineData>
        if let closed = aggregator.lastClosed {
            guard let index = snapshot.lastIndex(where: { $0.openTime == closed.openTime }) else {
                if last.openTime < closed.openTime { return .unchanged }
                return rebuild(bars: snapshot, live: true, now: now)
            }
            // Committed history must be exactly this snapshot's closed prefix: same bars, same values.
            guard let start = committedBars.firstIndex(where: { $0.openTime == first.openTime }),
                committedBars[start...].elementsEqual(
                    snapshot[...index], by: { $0.openTime == $1.openTime && $0.sameValues($1) })
            else { return rebuild(bars: snapshot, live: true, now: now) }
            tail = snapshot[(index + 1)...]
        } else if let forming = aggregator.forming {
            tail = snapshot.drop(while: { $0.openTime < forming.openTime })
        } else {
            return rebuild(bars: snapshot, live: true, now: now)
        }

        // A bar with a later one after it is final, though REST never says so.
        var outputs: [PineCandleAggregator.Output] = []
        for (offset, var bar) in tail.enumerated() {
            if offset < tail.count - 1 { bar.isClosed = true }
            outputs += aggregator.ingest(.init(bar: bar, origin: .poll))
        }
        return process(outputs)
    }

    // MARK: - Execution

    private func process(_ outputs: [PineCandleAggregator.Output]) -> PineExecutionOutcome {
        var executions: [PineBarFlags] = []
        var alerts: [PineAlertEvent] = []
        var barID: PineBarID?
        for output in outputs {
            let bar: KlineData
            let phase: PineBarPhase
            switch output {
            case .rebuild(let reason):
                return .needsRebuild(reason)
            case .tick(let tickBar, let isNew):
                bar = tickBar
                phase = .realtimeTick(isNew: isNew)
            case .close(let closedBar):
                bar = closedBar
                phase = .realtimeClose(isNew: session.lastOpenTime != closedBar.openTime)
            }
            guard PineExecutionScheduler.shouldExecute(program.declaration, phase: phase) else { continue }
            do {
                executions.append(try session.execute(.init(candle: bar, phase: phase)))
                if phase.isConfirmed { committedBars.append(bar) }
                alerts += session.emittedAlerts
                barID = self.barID(for: bar)
            } catch is PineExecutionRejection {
                continue
            } catch {
                return fail(error)
            }
        }
        guard !executions.isEmpty else { return .unchanged }
        return .updated(
            PineExecutionUpdate(output: session.output(), executions: executions, alerts: alerts, barID: barID))
    }

    private func fail(_ error: Error) -> PineExecutionOutcome {
        isFailed = true
        return .failed(
            error as? PineDiagnostic ?? .error("PINE4999", .runtime, error.localizedDescription, .zero))
    }

    private func barID(for bar: KlineData) -> PineBarID {
        PineBarID(dataset: dataset, openTime: bar.openTime)
    }

    /// Seconds between the last two bars; zero when there are fewer than two.
    private static func spacing(of bars: [KlineData]) -> Double {
        guard bars.count > 1 else { return 0 }
        return bars[bars.count - 1].openTime.timeIntervalSince(bars[bars.count - 2].openTime)
    }
}
