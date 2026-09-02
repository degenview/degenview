import XCTest

@testable import DegenView

final class ChartViewModelFetchTests: XCTestCase {
    actor SequencedSource: TickerDataSource {
        nonisolated let type = DataSourceType.binance
        private var call = 0

        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
            call += 1
            let current = call
            try await Task.sleep(for: current == 1 ? .milliseconds(150) : .milliseconds(5))
            return [KlineData(time: Date(timeIntervalSince1970: Double(current)), price: Double(current))]
        }

        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    @MainActor
    func testFetchDataWaitsForSourceCompletion() async {
        let source = SequencedSource()
        let viewModel = ChartViewModel(ticker: "BTC", api: source)
        let started = ContinuousClock.now

        await viewModel.fetchData(for: .oneDay, count: 1)

        XCTAssertGreaterThanOrEqual(started.duration(to: .now), .milliseconds(100))
        XCTAssertEqual(viewModel.currentPrice, 1)
    }

    @MainActor
    func testNewerFetchCancelsAndSupersedesOlderFetch() async {
        let source = SequencedSource()
        let viewModel = ChartViewModel(ticker: "BTC", api: source)
        let first = Task { @MainActor in await viewModel.fetchData(for: .oneDay, count: 1) }
        try? await Task.sleep(for: .milliseconds(20))

        await viewModel.fetchData(for: .oneWeek, count: 1)
        await first.value

        XCTAssertEqual(viewModel.currentPrice, 2)
        XCTAssertNil(viewModel.errorMessage)
    }

    @MainActor
    func testReplacingPolymarketTickerClearsOldSeriesAndPlottedData() {
        let viewModel = ChartViewModel(ticker: "old", source: .polymarket)
        viewModel.pmSeries = [
            PmSeriesConfig(tokenID: "old-a", label: "Old A", enabled: true),
            PmSeriesConfig(tokenID: "old-b", label: "Old B", enabled: true),
        ]
        viewModel.klineData = [KlineData(time: .now, price: 0.75)]
        viewModel.currentPrice = 0.75

        viewModel.updateTicker(
            symbol: "new", source: .polymarket, displayName: "New Market", pmSeries: nil)

        XCTAssertEqual(viewModel.ticker, "new")
        XCTAssertEqual(viewModel.title, "New Market")
        XCTAssertTrue(viewModel.pmSeries.isEmpty)
        XCTAssertTrue(viewModel.klineData.isEmpty)
        XCTAssertNil(viewModel.currentPrice)
    }

    @MainActor
    func testSinglePolymarketChoiceDrivesSubtitleWithoutReplacingEventTitle() {
        let viewModel = ChartViewModel(
            ticker: "bird", source: .polymarket, displayName: "AGT Winner")
        viewModel.pmSeries = [
            PmSeriesConfig(tokenID: "bird", label: "Bird & Byron", enabled: true)
        ]
        viewModel.currentPrice = 0.51

        XCTAssertEqual(viewModel.title, "AGT Winner")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.label, "Bird & Byron")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.price, 0.51)
    }
}
