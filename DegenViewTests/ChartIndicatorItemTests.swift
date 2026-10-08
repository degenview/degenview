import XCTest

@testable import DegenView

@MainActor
final class ChartIndicatorItemTests: XCTestCase {
    private final class StubSource: TickerDataSource {
        let type: DataSourceType
        init(type: DataSourceType = .binance) { self.type = type }
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private func makeModel(source: DataSourceType = .binance) -> ChartViewModel {
        ChartViewModel(ticker: "T", source: source, api: StubSource(type: source))
    }

    func testEmptyChartHasNoItems() {
        XCTAssertTrue(makeModel().indicatorItems.isEmpty)
    }

    func testBuiltInsListInFixedOrderWithParameterInTitle() {
        let model = makeModel()
        model.showTrendFlips = true
        model.showEMA = true
        model.emaPeriod = 50
        model.showRSI = true
        XCTAssertEqual(model.indicatorItems.map(\.title), ["RSI \(RSI.period)", "EMA 50", "Trend Flips"])
    }

    func testSetOnRoundTripsToTheViewModelFlag() {
        let model = makeModel()
        for indicator in BuiltInIndicator.allCases {
            indicator.setOn(true, in: model)
            XCTAssertTrue(indicator.isOn(in: model), "\(indicator)")
            indicator.setOn(false, in: model)
            XCTAssertFalse(indicator.isOn(in: model), "\(indicator)")
        }
    }

    func testAddableBuiltInsExcludeTheOnesAlreadyApplied() {
        let model = makeModel()
        model.showRSI = true
        XCTAssertFalse(model.addableBuiltIns.map(\.indicator).contains(.rsi))
        XCTAssertTrue(model.addableBuiltIns.map(\.indicator).contains(.ema))
    }

    func testVolumeIsHiddenOnLineCharts() {
        let model = makeModel(source: .polymarket)
        XCTAssertEqual(BuiltInIndicator.volume.availability(in: model), .hidden)
        XCTAssertFalse(model.addableBuiltIns.map(\.indicator).contains(.volume))
    }

    func testSplitFoldsChipsBeyondTheLimitIntoOverflow() {
        func item(_ n: Int) -> ChartIndicatorItem {
            ChartIndicatorItem(kind: .script(UUID()), title: "S\(n)", isVisible: true, hasError: false)
        }
        let few = (0..<ChartIndicatorItem.visibleChipLimit).map(item)
        XCTAssertEqual(ChartIndicatorItem.split(few).overflow.count, 0)
        let many = (0..<(ChartIndicatorItem.visibleChipLimit + 2)).map(item)
        let split = ChartIndicatorItem.split(many)
        XCTAssertEqual(split.inline.count, ChartIndicatorItem.visibleChipLimit)
        XCTAssertEqual(split.overflow.map(\.title), many.suffix(2).map(\.title))
    }

    func testHidingABuiltInKeepsItAppliedButNotDrawn() {
        let model = makeModel()
        model.showRSI = true
        BuiltInIndicator.rsi.setHidden(true, in: model)
        XCTAssertTrue(model.showRSI)
        XCTAssertFalse(model.isDrawn(.rsi))
        XCTAssertEqual(model.indicatorItems.map(\.isVisible), [false])
        BuiltInIndicator.rsi.setHidden(false, in: model)
        XCTAssertTrue(model.isDrawn(.rsi))
    }

    func testRemovingAHiddenBuiltInClearsItsHiddenState() {
        let model = makeModel()
        model.showEMA = true
        BuiltInIndicator.ema.setHidden(true, in: model)
        model.showEMA = false
        model.showEMA = true
        XCTAssertTrue(model.isDrawn(.ema))
    }

    func testHiddenBuiltInsRoundTripThroughTickerConfig() {
        let config = TickerConfig(symbol: "T", source: .binance, showRSI: true, hiddenIndicators: ["rsi"])
        let decoded = try? JSONDecoder().decode(TickerConfig.self, from: JSONEncoder().encode(config))
        XCTAssertEqual(decoded?.hiddenIndicators, ["rsi"])
        let model = makeModel()
        model.applyConfig(config)
        XCTAssertEqual(model.hiddenBuiltIns, [.rsi])
    }
}
