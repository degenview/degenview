import XCTest

@testable import DegenView

@MainActor
final class ReplayBarFlowTests: XCTestCase {
    private func model() throws -> (ContentViewModel, ChartViewModel) {
        let db = try AppDatabase.makeInMemory()
        let content = ContentViewModel(
            tabID: UUID(), savedViews: SavedViewStore(database: db), tabs: TabsStore(database: db))
        let vm = ChartViewModel(ticker: "BTCUSDT", source: .binance, drawingStore: DrawingStore(database: db))
        let base = Date(timeIntervalSince1970: 1_700_000_000)
        vm.klineData = (0..<30).map {
            KlineData(time: base.addingTimeInterval(Double($0) * 3600), price: 100 + Double($0))
        }
        content.chartViewModels = [vm]
        return (content, vm)
    }

    func testFirstBarFromReadyStartsReplay() async throws {
        let (content, vm) = try model()
        content.openReplayBar()
        XCTAssertEqual(content.replay.status, .ready)
        content.selectFirstReplayBar()
        for _ in 0..<100 where content.replay.status == .ready { try await Task.sleep(nanoseconds: 100_000_000) }
        XCTAssertEqual(content.replay.status, .paused)
        XCTAssertNotNil(vm.replayTimestamp)
    }

    func testRandomBarFromReadyStartsReplay() async throws {
        let (content, _) = try model()
        content.openReplayBar()
        content.selectRandomReplayBar()
        for _ in 0..<100 where content.replay.status == .ready { try await Task.sleep(nanoseconds: 100_000_000) }
        XCTAssertEqual(content.replay.status, .paused)
    }
}
