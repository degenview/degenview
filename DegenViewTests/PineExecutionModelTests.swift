import XCTest

@testable import DegenView

/// Historical execution, realtime rollback, commit, `var` / `varip` and `barstate.*`.
final class PineExecutionModelTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    /// Loads `closes` as history plus one forming bar, returning the controller.
    private func live(
        _ controller: PineExecutionController, history closes: [Double]
    ) throws -> PineExecutionUpdate {
        let forming = F.bar(closes.count, open: 100, close: 100)
        let bars = F.history(closes) + [forming]
        let outcome = controller.rebuild(bars: bars, live: true, now: F.now(during: closes.count))
        return try XCTUnwrap(F.update(outcome))
    }

    private func tick(
        _ controller: PineExecutionController, bar index: Int, open: Double = 100, close: Double,
        closed: Bool = false
    ) throws -> PineExecutionUpdate {
        let outcome = controller.ingest(F.stream(F.bar(index, open: open, close: close, closed: closed)))
        return try XCTUnwrap(F.update(outcome), "no execution for close \(close)")
    }

    // MARK: - Historical

    func testHistoricalBarsExecuteOncePerBarAndSeriesResolve() throws {
        let controller = F.controller("x = close * 2\nplot(x)\nplot(x[1])\nplot(close[2])\nplot(bar_index)")
        let closes = [1.0, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        let update = try XCTUnwrap(F.update(controller.rebuild(bars: F.history(closes), live: false)))
        XCTAssertEqual(update.executions.count, 10)
        XCTAssertEqual(update.output.barCount, 10)
        XCTAssertEqual(F.plot(update.output, 0), closes.map { $0 * 2 })
        XCTAssertEqual(F.plot(update.output, 1), [nil] + closes.dropLast().map { $0 * 2 })
        XCTAssertEqual(F.plot(update.output, 2), [nil, nil] + closes.dropLast(2))
        XCTAssertEqual(F.plot(update.output, 3), (0..<10).map(Double.init))
        XCTAssertTrue(update.executions.allSatisfy { $0.isHistory && $0.isConfirmed && !$0.isRealtime })
        XCTAssertEqual(update.executions.map(\.isLast), Array(repeating: false, count: 9) + [true])
        XCTAssertEqual(update.executions.filter(\.isFirst).count, 1)
    }

    // MARK: - Realtime rollback and commit

    func testRealtimeUpdatesRecalculateOneBarAndCommitOnce() throws {
        let controller = F.controller("x = close\nplot(x)\nplot(x[1])")
        let start = try live(controller, history: [10, 11, 12])
        // The forming bar ran as the first tick: one working sample after three committed ones.
        XCTAssertEqual(F.plot(start.output, 0).count, 4)

        var runs = start.executions.count - 3  // the forming bar's first tick
        for close in [101.0, 105] {
            let update = try tick(controller, bar: 3, close: close)
            runs += update.executions.count
            XCTAssertEqual(F.plot(update.output, 0).count, 4, "a tick must not append history")
            XCTAssertEqual(F.last(update.output, 0), close)
            XCTAssertEqual(F.last(update.output, 1), 12, "x[1] stays the last committed bar")
        }
        let closing = try tick(controller, bar: 3, close: 103, closed: true)
        runs += closing.executions.count
        XCTAssertEqual(runs, 4)
        XCTAssertEqual(closing.output.barCount, 4)
        XCTAssertEqual(F.plot(closing.output, 0), [10, 11, 12, 103])

        // Exactly one committed entry for the bar: the next bar sees it at [1].
        let next = try tick(controller, bar: 4, close: 104)
        XCTAssertEqual(F.last(next.output, 1), 103)
        XCTAssertEqual(F.plot(next.output, 0), [10, 11, 12, 103, 104])
    }

    func testOrdinaryVarRollsBackBetweenTicks() throws {
        let controller = F.controller("var int x = 0\nx += 1\nplot(x)")
        let start = try live(controller, history: [1, 2, 3])
        XCTAssertEqual(F.last(start.output), 4)
        for close in [101.0, 102, 103] {
            XCTAssertEqual(F.last(try tick(controller, bar: 3, close: close).output), 4)
        }
        XCTAssertEqual(F.last(try tick(controller, bar: 3, close: 104, closed: true).output), 4)
        XCTAssertEqual(F.last(try tick(controller, bar: 4, close: 100).output), 5)
    }

    func testVaripSurvivesRollbackWithinABar() throws {
        let controller = F.controller(
            "varip int updates = 0\nif barstate.isnew\n    updates := 1\nelse\n    updates += 1\nplot(updates)")
        let start = try live(controller, history: [1, 2, 3])
        var seen = [F.last(start.output)]
        for close in [101.0, 102] { seen.append(F.last(try tick(controller, bar: 3, close: close).output)) }
        seen.append(F.last(try tick(controller, bar: 3, close: 103, closed: true).output))
        XCTAssertEqual(seen, [1, 2, 3, 4])
        // A new bar starts the count again.
        XCTAssertEqual(F.last(try tick(controller, bar: 4, close: 100).output), 1)
        XCTAssertEqual(F.last(try tick(controller, bar: 4, close: 101).output), 2)
    }

    func testVaripValuesAreNotReproducibleFromHistoryAlone() throws {
        let source = "varip int updates = 0\nif barstate.isnew\n    updates := 1\nelse\n    updates += 1\nplot(updates)"
        let live = F.controller(source)
        _ = try self.live(live, history: [1, 2, 3])
        _ = try tick(live, bar: 3, close: 101)
        _ = try tick(live, bar: 3, close: 102)
        let observed = F.last(try tick(live, bar: 3, close: 103, closed: true).output)

        let reloaded = F.controller(source)
        let recalculated = F.update(reloaded.rebuild(bars: F.history([1, 2, 3, 103]), live: false))
        XCTAssertEqual(observed, 4)
        XCTAssertEqual(F.last(try XCTUnwrap(recalculated).output), 1, "history holds one execution per bar")
    }

    // MARK: - barstate

    func testBarstateTransitionsAcrossTheBarLifecycle() throws {
        let controller = F.controller("plot(close)")
        let start = try live(controller, history: [1, 2, 3])
        let history = Array(start.executions.prefix(3))
        XCTAssertTrue(history.allSatisfy { $0.isHistory && !$0.isRealtime && $0.isConfirmed && $0.isNew })
        XCTAssertFalse(history[2].isLast, "the forming bar is the last one")
        XCTAssertTrue(history[2].isLastConfirmedHistory)

        let first = try XCTUnwrap(start.executions.last)
        XCTAssertTrue(first.isRealtime && first.isNew && !first.isConfirmed && !first.isHistory && first.isLast)

        let second = try XCTUnwrap(try tick(controller, bar: 3, close: 101).executions.last)
        XCTAssertTrue(second.isRealtime && !second.isNew && !second.isConfirmed)

        let closing = try XCTUnwrap(try tick(controller, bar: 3, close: 102, closed: true).executions.last)
        XCTAssertTrue(closing.isRealtime && !closing.isNew && closing.isConfirmed)

        let next = try XCTUnwrap(try tick(controller, bar: 4, close: 100).executions.last)
        XCTAssertTrue(next.isRealtime && next.isNew && !next.isConfirmed)
        XCTAssertEqual(next.isFirst, false)
    }

    func testABarThatFirstArrivesClosedIsNewAndConfirmed() throws {
        let controller = F.controller("plot(close)")
        _ = controller.rebuild(bars: F.history([1, 2, 3]), live: false)
        let flags = try XCTUnwrap(try tick(controller, bar: 3, close: 5, closed: true).executions.last)
        XCTAssertTrue(flags.isNew && flags.isConfirmed && flags.isRealtime)
    }

    // MARK: - Bar identity

    func testSessionRejectsStaleAndDuplicateBars() throws {
        let session = PineRuntimeSession(program: F.program("plot(close)"))
        let bars = F.history([1, 2, 3])
        for bar in bars { try session.execute(.init(candle: bar, phase: .historical)) }
        XCTAssertThrowsError(try session.execute(.init(candle: bars[1], phase: .historical))) {
            XCTAssertEqual($0 as? PineExecutionRejection, .staleBar)
        }
        XCTAssertThrowsError(try session.execute(.init(candle: bars[2], phase: .realtimeClose(isNew: false)))) {
            XCTAssertEqual($0 as? PineExecutionRejection, .duplicateBar)
        }
        XCTAssertEqual(session.output().plots[0].values.count, 3)
    }

    func testDuplicateTransportMessagesDoNotAdvanceSeries() throws {
        let controller = F.controller("plot(close)\nplot(close[1])")
        _ = try live(controller, history: [1, 2, 3])
        _ = try tick(controller, bar: 3, close: 101)
        let repeated = controller.ingest(F.stream(F.bar(3, open: 100, close: 101)))
        guard case .unchanged = repeated else { return XCTFail("duplicate should change nothing") }
        let closed = try tick(controller, bar: 3, close: 102, closed: true)
        let again = controller.ingest(F.stream(F.bar(3, open: 100, close: 102, closed: true)))
        guard case .unchanged = again else { return XCTFail("closed duplicate should change nothing") }
        let late = controller.ingest(F.stream(F.bar(3, open: 100, close: 99)))
        guard case .unchanged = late else { return XCTFail("a late tick must not reopen a closed bar") }
        XCTAssertEqual(closed.output.barCount, 4)
    }

    // MARK: - Isolation, reload, dataset change

    func testScriptInstancesShareNoState() throws {
        let counter = F.controller("var int n = 0\nn += 1\nplot(n)")
        let other = F.controller("varip int n = 100\nn += 5\nplot(n)")
        let bars = F.history([1, 2, 3, 4])
        let first = try XCTUnwrap(F.update(counter.rebuild(bars: bars, live: false)))
        let second = try XCTUnwrap(F.update(other.rebuild(bars: bars, live: false)))
        XCTAssertEqual(F.plot(first.output), [1, 2, 3, 4])
        XCTAssertEqual(F.plot(second.output), [105, 110, 115, 120])
        // Feeding one leaves the other exactly where it was.
        let fed = try XCTUnwrap(F.update(counter.ingest(F.stream(F.bar(4, open: 5, close: 5)))))
        XCTAssertEqual(F.plot(fed.output), [1, 2, 3, 4, 5])
        guard case .unchanged = other.ingest(F.stream(F.bar(3, open: 4, close: 4))) else {
            return XCTFail("a bar the second script already committed must change nothing")
        }
        XCTAssertEqual(F.plot(second.output), [105, 110, 115, 120])
    }

    func testRebuildDiscardsPreviousState() throws {
        let controller = F.controller("var int n = 0\nn += 1\nplot(n)")
        _ = controller.rebuild(bars: F.history([1, 2, 3, 4, 5]), live: false)
        let rebuilt = try XCTUnwrap(F.update(controller.rebuild(bars: F.history([9, 8]), live: false)))
        XCTAssertEqual(F.plot(rebuilt.output), [1, 2])
        XCTAssertEqual(rebuilt.output.barCount, 2)
    }

    func testEditedScriptRecalculatesFromScratch() throws {
        let bars = F.history([1, 2, 3])
        let first = F.controller("plot(close)")
        let edited = F.controller("plot(close * 10)")
        XCTAssertEqual(F.plot(try XCTUnwrap(F.update(first.rebuild(bars: bars, live: false))).output), [1, 2, 3])
        XCTAssertEqual(F.plot(try XCTUnwrap(F.update(edited.rebuild(bars: bars, live: false))).output), [10, 20, 30])
    }

    func testRecalculationIsDeterministic() throws {
        let source = "var float total = 0.0\ntotal += close\nplot(total)\nplot(ta.sma(close, 3))"
        let bars = F.history((1...50).map { Double($0 % 7) })
        let one = try XCTUnwrap(F.update(F.controller(source).rebuild(bars: bars, live: false)))
        let two = try XCTUnwrap(F.update(F.controller(source).rebuild(bars: bars, live: false)))
        XCTAssertEqual(F.plot(one.output, 0), F.plot(two.output, 0))
        XCTAssertEqual(F.plot(one.output, 1), F.plot(two.output, 1))
        let session = PineRuntimeSession(program: F.program(source))
        let direct = try session.evaluate(bars: bars).output
        XCTAssertEqual(F.plot(direct, 0), F.plot(one.output, 0))
    }

    func testRealtimeTransientStateDoesNotLeakIntoRecalculation() throws {
        let source = "varip int n = 0\nn += 1\nplot(n)"
        let controller = F.controller(source)
        _ = try live(controller, history: [1, 2, 3])
        for close in [101.0, 102, 103] { _ = try tick(controller, bar: 3, close: close) }
        let bars = F.history([1, 2, 3, 100])
        let rebuilt = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: false)))
        XCTAssertEqual(F.plot(rebuilt.output), [1, 2, 3, 4])
    }

    // MARK: - Host

    func testHostSerializesRebuildAndIngest() async throws {
        let host = PineExecutionHost(
            program: F.program("plot(close)"), dataset: F.dataset, inputs: [:], theme: .dark,
            symbol: PineSymbolInfo())
        let bars = F.history([1, 2, 3]) + [F.bar(3, open: 100, close: 100)]
        let rebuilt = await host.rebuild(bars: bars, live: false)
        XCTAssertEqual(F.update(rebuilt)?.output.barCount, 4)
        let outcome = await host.ingest(F.stream(F.bar(4, open: 5, close: 6)))
        XCTAssertEqual(F.update(outcome)?.executions.count, 1)
    }
}
