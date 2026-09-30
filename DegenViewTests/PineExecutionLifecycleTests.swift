import XCTest

@testable import DegenView

/// End to end: history, realtime bar, close, next bar; alerts and REST reconciliation through the
/// same pipeline.
final class PineExecutionLifecycleTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private func ticks(
        _ controller: PineExecutionController, bar index: Int, open: Double, closes: [Double],
        closingLast: Bool = true
    ) -> [PineExecutionUpdate] {
        closes.enumerated().compactMap { offset, close in
            F.update(
                controller.ingest(
                    F.stream(F.bar(index, open: open, close: close, closed: closingLast && offset == closes.count - 1))))
        }
    }

    func testHundredBarsThenARealtimeBarThenTheNext() throws {
        let source = """
            var int n = 0
            varip int seen = 0
            n += 1
            if barstate.isnew
                seen := 1
            else
                seen += 1
            plot(close)
            plot(bar_index)
            plot(n)
            plot(seen)
            plot(open)
            """
        let controller = F.controller(source)
        let closes = (0..<100).map { Double($0) + 1 }
        let loaded = try XCTUnwrap(F.update(controller.rebuild(bars: F.history(closes), live: false)))
        XCTAssertEqual(loaded.executions.count, 100)
        XCTAssertEqual(loaded.output.barCount, 100)
        XCTAssertEqual(F.last(loaded.output, 1), 99)
        XCTAssertEqual(F.last(loaded.output, 2), 100)

        // Realtime bar 100: four executions, the last one closes it.
        let updates = ticks(controller, bar: 100, open: 100, closes: [100, 101, 105, 103])
        XCTAssertEqual(updates.count, 4)
        XCTAssertEqual(updates.map { F.last($0.output, 3) }, [1, 2, 3, 4])
        XCTAssertEqual(updates.map { F.last($0.output, 2) }, [101, 101, 101, 101])
        XCTAssertEqual(updates.map { F.plot($0.output, 0).count }, [101, 101, 101, 101])
        XCTAssertEqual(updates.map(\.executions.first?.isConfirmed), [false, false, false, true])
        let closed = try XCTUnwrap(updates.last)
        XCTAssertEqual(closed.output.barCount, 101)
        XCTAssertEqual(F.last(closed.output, 0), 103)
        XCTAssertEqual(F.last(closed.output, 1), 100)
        XCTAssertEqual(F.last(closed.output, 4), 100)
        XCTAssertEqual(closed.barID, PineBarID(dataset: F.dataset, openTime: F.time(100)))

        // Bar 101 opens: indexes advance by one, varip restarts.
        let next = try XCTUnwrap(ticks(controller, bar: 101, open: 103, closes: [104], closingLast: false).first)
        XCTAssertEqual(F.last(next.output, 1), 101)
        XCTAssertEqual(F.last(next.output, 2), 102)
        XCTAssertEqual(F.last(next.output, 3), 1)
        XCTAssertEqual(F.plot(next.output, 0).count, 102)
    }

    func testLoadingHistoryProducesNoAlerts() throws {
        let controller = F.controller("if close > open\n    alert(\"Green candle\")")
        let bars = (0..<10_000).map { F.bar($0, open: 100, close: 101, closed: true) }
        let update = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: false)))
        XCTAssertTrue(update.alerts.isEmpty, "nothing to deliver from 10,000 historical candles")
        XCTAssertFalse(update.output.alerts.isEmpty, "the report list still records them")
        XCTAssertTrue(update.output.alerts.allSatisfy { !$0.isRealtime })
    }

    private func alertRuns(_ frequency: String) throws -> [Int] {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_\(frequency))")
        let bars = F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)]
        let loaded = controller.rebuild(bars: bars, live: true, now: F.now(during: 3))
        XCTAssertEqual(F.update(loaded)?.alerts.count, 0, "loading never notifies")
        var counts = ticks(controller, bar: 3, open: 100, closes: [101, 102, 103]).map(\.alerts.count)
        // The next bar re-arms the alert.
        counts.append(ticks(controller, bar: 4, open: 103, closes: [105], closingLast: false).first?.alerts.count ?? -1)
        return counts
    }

    func testAlertFrequenciesFollowTheExecutionLifecycle() throws {
        // The forming bar opens at 99 (red). Updates: 101, 102, then the close at 103, then bar 4.
        XCTAssertEqual(try alertRuns("all"), [1, 1, 1, 1])
        XCTAssertEqual(try alertRuns("once_per_bar"), [1, 0, 0, 1])
        XCTAssertEqual(try alertRuns("once_per_bar_close"), [0, 0, 1, 0])
    }

    func testAlertOnAFormingBarAtLoadIsRemembered() throws {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_once_per_bar)")
        let bars = F.history([100, 100]) + [F.bar(2, open: 100, close: 105)]
        let loaded = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: true, now: F.now(during: 2))))
        XCTAssertTrue(loaded.alerts.isEmpty)
        let update = try XCTUnwrap(ticks(controller, bar: 2, open: 100, closes: [106], closingLast: false).first)
        XCTAssertTrue(update.alerts.isEmpty, "the once-per-bar ledger survives rollback")
    }

    // MARK: - REST reconciliation

    func testSnapshotWithNewBarsClosesTheOldAndOpensTheNew() throws {
        let controller = F.controller("plot(close)")
        let closes: [Double] = [1, 2, 3]
        let bars = F.history(closes) + [F.bar(3, open: 100, close: 100)]
        _ = controller.rebuild(bars: bars, live: true, now: F.now(during: 3))
        _ = ticks(controller, bar: 3, open: 100, closes: [104], closingLast: false)

        var snapshot = F.history(closes) + [F.bar(3, open: 100, close: 105), F.bar(4, open: 105, close: 106)]
        let update = try XCTUnwrap(F.update(controller.sync(snapshot: snapshot, now: F.now(during: 4))))
        XCTAssertEqual(update.executions.count, 2, "close bar 3 with final values, then open bar 4")
        XCTAssertEqual(F.plot(update.output), [1, 2, 3, 105, 106])
        XCTAssertTrue(update.executions[0].isConfirmed)

        snapshot[4] = F.bar(4, open: 105, close: 106)
        guard case .unchanged = controller.sync(snapshot: snapshot, now: F.now(during: 4)) else {
            return XCTFail("an identical snapshot changes nothing")
        }
    }

    func testSnapshotDoesNotRegressAStreamBar() throws {
        let controller = F.controller("plot(close)")
        _ = controller.rebuild(
            bars: F.history([1, 2]) + [F.bar(2, open: 100, close: 100)], live: true, now: F.now(during: 2))
        _ = ticks(controller, bar: 2, open: 100, closes: [110], closingLast: false)
        let stale = F.history([1, 2]) + [F.bar(2, open: 100, close: 101)]
        guard case .unchanged = controller.sync(snapshot: stale, now: F.now(during: 2)) else {
            return XCTFail("a cached REST bar must not overwrite the newer stream value")
        }
    }

    func testStaleSnapshotIsIgnored() throws {
        let controller = F.controller("plot(close)")
        _ = controller.rebuild(bars: F.history([1, 2, 3, 4]), live: false)
        guard case .unchanged = controller.sync(snapshot: F.history([1, 2]), now: F.now(during: 1)) else {
            return XCTFail("a snapshot older than committed history is ignored")
        }
    }

    func testCorrectedHistoryRebuilds() throws {
        let controller = F.controller("plot(close)")
        _ = controller.rebuild(bars: F.history([1, 2, 3, 4]), live: false)
        let corrected = F.history([1, 2, 30, 4])
        let update = try XCTUnwrap(F.update(controller.sync(snapshot: corrected, now: F.now(during: 4))))
        XCTAssertEqual(update.executions.count, 4)
        XCTAssertEqual(F.plot(update.output), [1, 2, 30, 4])
    }

    func testMissingBarInsideHistoryRebuilds() throws {
        let controller = F.controller("plot(close)")
        _ = controller.rebuild(bars: F.history([1, 2, 3, 4]), live: false)
        var withGap = F.history([1, 2, 3, 4, 5])
        withGap.remove(at: 2)
        let update = try XCTUnwrap(F.update(controller.sync(snapshot: withGap, now: F.now(during: 5))))
        XCTAssertEqual(F.plot(update.output).count, 4, "rebuilt over the snapshot's bars")
    }

    func testLongerHistoryRebuilds() throws {
        let controller = F.controller("plot(bar_index)")
        let bars = F.history([1, 2, 3, 4, 5])
        _ = controller.rebuild(bars: Array(bars.suffix(3)), live: false)
        let update = try XCTUnwrap(F.update(controller.sync(snapshot: bars, now: F.now(during: 5))))
        XCTAssertEqual(F.plot(update.output), [0, 1, 2, 3, 4])
    }

    func testSlidingWindowKeepsTheSession() throws {
        let controller = F.controller("var int n = 0\nn += 1\nplot(n)")
        let bars = F.history([1, 2, 3, 4, 5])
        _ = controller.rebuild(bars: Array(bars.prefix(4)), live: false)
        // The fetch window slides: the oldest bar drops off, a new one arrives.
        let update = try XCTUnwrap(F.update(controller.sync(snapshot: Array(bars.suffix(4)), now: F.now(during: 5))))
        XCTAssertEqual(update.executions.count, 1)
        XCTAssertEqual(F.last(update.output), 5, "state from the earlier bars is kept, not recalculated")
    }

    // MARK: - Cost

    func testRealtimeTickCostOnLongHistory() throws {
        let controller = F.controller("plot(ta.sma(close, 20))\nplot(close)")
        _ = controller.rebuild(bars: F.history((0..<2_000).map { Double($0 % 50) }), live: false)
        let clock = ContinuousClock()
        var close = 100.0
        let elapsed = clock.measure {
            for _ in 0..<50 {
                close += 1
                _ = controller.ingest(F.stream(F.bar(2_000, open: 100, close: close)))
            }
        }
        // Rollback copies the working plot arrays each tick (O(bars)); documented, and cheap at chart size.
        XCTAssertLessThan(elapsed, .seconds(5))
    }
}
