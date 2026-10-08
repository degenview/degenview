import XCTest

@testable import DegenView

@MainActor
final class FocusedChartTests: XCTestCase {
    private func content() throws -> ContentViewModel {
        let db = try AppDatabase.makeInMemory()
        return ContentViewModel(
            tabID: UUID(), savedViews: SavedViewStore(database: db), tabs: TabsStore(database: db))
    }

    private func chart(_ ticker: String, _ source: DataSourceType = .binance) -> ChartViewModel {
        let vm = ChartViewModel(ticker: ticker, source: source, drawingStore: DrawingStore(database: try! .makeInMemory()))
        vm.serviceResolver = { StubMarketSource($0) }
        return vm
    }

    private func item(_ symbol: String, _ source: DataSourceType = .binance) -> WatchlistInstrument {
        WatchlistInstrument(instrument: InstrumentID(source: source, symbol: symbol), name: symbol, label: symbol)
    }

    func testWithNothingFocusedTheFirstMarketChartIsUsed() throws {
        let model = try content()
        let first = chart("BTCUSDT")
        model.chartViewModels = [first, chart("ETHUSDT")]

        XCTAssertEqual(model.resolvedFocusedChart?.chartID, first.chartID)
    }

    func testTheLastChartClickedIsTheFocusedOne() throws {
        let model = try content()
        let second = chart("ETHUSDT")
        model.chartViewModels = [chart("BTCUSDT"), second]

        model.focusChart(second.chartID)

        XCTAssertEqual(model.resolvedFocusedChart?.chartID, second.chartID)
    }

    func testFocusFallsBackWhenTheFocusedChartIsRemoved() throws {
        let model = try content()
        let first = chart("BTCUSDT")
        let second = chart("ETHUSDT")
        model.chartViewModels = [first, second]
        model.focusChart(second.chartID)

        model.removeTicker(second)

        XCTAssertNil(model.focusedChartID)
        XCTAssertEqual(model.resolvedFocusedChart?.chartID, first.chartID)
    }

    func testUnknownChartsCannotTakeFocus() throws {
        let model = try content()
        model.chartViewModels = [chart("BTCUSDT")]

        model.focusChart(UUID())

        XCTAssertNil(model.focusedChartID)
    }

    func testTheFocusRingOnlyShowsWhenThereIsMoreThanOneCard() throws {
        let model = try content()
        let only = chart("BTCUSDT")
        model.chartViewModels = [only]
        XCTAssertFalse(model.isFocused(only))

        let other = chart("ETHUSDT")
        model.chartViewModels = [only, other]
        model.focusChart(other.chartID)
        XCTAssertTrue(model.isFocused(other))
        XCTAssertFalse(model.isFocused(only))
    }

    func testAWatchlistClickSwitchesOnlyTheFocusedChart() async throws {
        let model = try content()
        let btc = chart("BTCUSDT")
        let eth = chart("ETHUSDT")
        model.chartViewModels = [btc, eth]
        model.focusChart(eth.chartID)

        model.openWatchlistInstrument(item("SOLUSDT"))

        XCTAssertEqual(btc.ticker, "BTCUSDT")
        XCTAssertEqual(eth.ticker, "SOLUSDT")
        XCTAssertEqual(eth.chartID, model.focusedChartID)
    }

    func testClickingAMarketAlreadyOnScreenJustFocusesIt() throws {
        let model = try content()
        let btc = chart("BTCUSDT")
        let eth = chart("ETHUSDT")
        model.chartViewModels = [btc, eth]
        model.focusChart(eth.chartID)

        model.openWatchlistInstrument(item("BTCUSDT"))

        XCTAssertEqual(btc.ticker, "BTCUSDT")
        XCTAssertEqual(eth.ticker, "ETHUSDT", "no duplicate and no replacement")
        XCTAssertEqual(model.focusedChartID, btc.chartID)
        XCTAssertEqual(model.chartViewModels.count, 2)
    }

    func testAddAsNewChartForAMarketOnScreenFocusesItInsteadOfDuplicating() throws {
        let model = try content()
        let btc = chart("BTCUSDT")
        model.chartViewModels = [btc, chart("ETHUSDT")]

        model.addWatchlistInstrumentAsChart(item("BTC"))

        XCTAssertEqual(model.chartViewModels.count, 2)
        XCTAssertEqual(model.focusedChartID, btc.chartID)
    }

    func testSwitchingMarketsDirtiesTheSavedLayoutButFocusAndWatchlistEditsDoNot() async throws {
        let model = try content()
        let btc = chart("BTCUSDT")
        let eth = chart("ETHUSDT")
        model.chartViewModels = [btc, eth]
        model.layout.refresh()
        let clean = model.layout.isDirty

        model.focusChart(eth.chartID)
        let store = WatchlistStore(database: try AppDatabase.makeInMemory())
        try store.addInstrument(item("SOLUSDT"), to: try XCTUnwrap(store.favorites).id)
        model.layout.refresh()
        XCTAssertEqual(model.layout.isDirty, clean, "focus and watchlist edits are not layout changes")

        model.switchChart(eth, to: item("SOLUSDT"))
        model.layout.refresh()
        XCTAssertTrue(model.layout.isDirty, "a chart showing another market is a layout change")
    }
}

/// Answers any request with one candle, so a switch's refetch never reaches the network.
private final class StubMarketSource: TickerDataSource, @unchecked Sendable {
    let type: DataSourceType
    init(_ type: DataSourceType) { self.type = type }
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        [KlineData(time: Date(), price: 1)]
    }
    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}
