import XCTest

@testable import DegenView

final class PineExecutionSchedulerTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private let phases: [(String, PineBarPhase)] = [
        ("historical", .historical), ("tick", .realtimeTick(isNew: false)),
        ("close", .realtimeClose(isNew: false)),
    ]

    private func declaration(_ header: String) -> PineDeclarationMetadata {
        F.program("plot(close)", header: header).declaration
    }

    func testIndicatorsRunOnEveryEvent() {
        let indicator = declaration("indicator(\"T\")")
        for (name, phase) in phases {
            XCTAssertTrue(PineExecutionScheduler.shouldExecute(indicator, phase: phase), name)
        }
    }

    func testStrategyRunsOnHistoryAndCloseButNotTicksByDefault() {
        let strategy = declaration("strategy(\"T\")")
        XCTAssertEqual(strategy.strategy?.calcOnEveryTick, false)
        XCTAssertTrue(PineExecutionScheduler.shouldExecute(strategy, phase: .historical))
        XCTAssertFalse(PineExecutionScheduler.shouldExecute(strategy, phase: .realtimeTick(isNew: true)))
        XCTAssertTrue(PineExecutionScheduler.shouldExecute(strategy, phase: .realtimeClose(isNew: true)))
    }

    func testCalcOnEveryTickRunsStrategyOnTicks() {
        let strategy = declaration("strategy(\"T\", calc_on_every_tick=true)")
        XCTAssertEqual(strategy.strategy?.calcOnEveryTick, true)
        for (name, phase) in phases {
            XCTAssertTrue(PineExecutionScheduler.shouldExecute(strategy, phase: phase), name)
        }
    }

    func testDefaultStrategyOnlyExecutesAtTheClosingUpdate() throws {
        let controller = F.controller("plot(close)", header: "strategy(\"T\")")
        let bars = F.history([1, 2, 3]) + [F.bar(3, open: 100, close: 100)]
        let loaded = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: true, now: F.now(during: 3))))
        XCTAssertEqual(loaded.executions.count, 3, "the forming bar waits for its close")
        XCTAssertTrue(loaded.executions[2].isLast, "no live bar executes, so history ends the dataset")

        for close in [101.0, 102] {
            guard case .unchanged = controller.ingest(F.stream(F.bar(3, open: 100, close: close))) else {
                return XCTFail("a strategy must not execute on a realtime tick")
            }
        }
        let closing = F.bar(3, open: 100, close: 103, closed: true)
        let update = try XCTUnwrap(F.update(controller.ingest(F.stream(closing))))
        XCTAssertEqual(update.executions.count, 1)
        let flags = try XCTUnwrap(update.executions.first)
        XCTAssertTrue(flags.isNew && flags.isConfirmed && flags.isRealtime, "its only execution is also its first")
        XCTAssertEqual(F.last(update.output), 103)
    }

    func testEveryTickStrategyRecalculatesOnTicksWithRollback() throws {
        let controller = F.controller(
            "var int n = 0\nn += 1\nplot(n)", header: "strategy(\"T\", calc_on_every_tick=true)")
        let bars = F.history([1, 2, 3]) + [F.bar(3, open: 100, close: 100)]
        let loaded = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: true, now: F.now(during: 3))))
        XCTAssertEqual(F.last(loaded.output), 4)
        for close in [101.0, 102] {
            let update = try XCTUnwrap(F.update(controller.ingest(F.stream(F.bar(3, open: 100, close: close)))))
            XCTAssertEqual(update.executions.count, 1)
            XCTAssertEqual(F.last(update.output), 4, "rollback: n does not accumulate per tick")
        }
    }
}
