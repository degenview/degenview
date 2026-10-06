import XCTest

@testable import DegenView

final class LayoutSnapshotTests: XCTestCase {
    private func configs(_ symbols: [String]) -> [TickerConfig] {
        symbols.map { TickerConfig(symbol: $0, source: .binance) }
    }

    private func view(columns: [ChartColumn]? = nil, configs: [TickerConfig], candleCount: Int = 100) -> SavedView {
        SavedView(
            name: "A", tickers: configs.map(\.symbol), timeRange: .oneDay, createdAt: Date(),
            tickerConfigs: configs, chartColumns: columns, candleCount: candleCount)
    }

    func testDefaultGridResolvesToTheSameSnapshotEveryTime() {
        let saved = view(configs: configs(["BTCUSDT", "ETHUSDT", "SOLUSDT"]))
        // `ChartColumn.resolved` mints fresh column ids for a view with no explicit columns.
        XCTAssertEqual(LayoutSnapshot(view: saved), LayoutSnapshot(view: saved))
    }

    func testLiveTabMatchesItsSavedView() {
        let list = configs(["BTCUSDT", "ETHUSDT"])
        let saved = view(configs: list)
        let live = LayoutSnapshot(
            timeRange: saved.timeRange, configs: list,
            columns: ChartColumn.resolved(saved.chartColumns, chartIDs: list.map(\.chartID)))
        XCTAssertEqual(live, LayoutSnapshot(view: saved))
    }

    func testZoomIsNotPartOfTheSnapshot() {
        let list = configs(["BTCUSDT"])
        XCTAssertEqual(
            LayoutSnapshot(view: view(configs: list, candleCount: 50)),
            LayoutSnapshot(view: view(configs: list, candleCount: 400)))
    }

    func testChartSettingChangesTheSnapshot() {
        var list = configs(["BTCUSDT"])
        let before = LayoutSnapshot(view: view(configs: list))
        list[0].showRSI = true
        XCTAssertNotEqual(before, LayoutSnapshot(view: view(configs: list)))
    }

    func testMovingAChartBetweenColumnsChangesTheSnapshot() {
        let list = configs(["BTCUSDT", "ETHUSDT"])
        let ids = list.map(\.chartID)
        let sideBySide = view(
            columns: [ChartColumn(chartIDs: [ids[0]]), ChartColumn(chartIDs: [ids[1]])], configs: list)
        let stacked = view(columns: [ChartColumn(chartIDs: ids)], configs: list)
        XCTAssertNotEqual(LayoutSnapshot(view: sideBySide), LayoutSnapshot(view: stacked))
    }

    func testTimeframeChangesTheSnapshot() {
        let list = configs(["BTCUSDT"])
        var snapshot = LayoutSnapshot(view: view(configs: list))
        let original = snapshot
        snapshot.timeRange = .oneWeek
        XCTAssertNotEqual(original, snapshot)
    }
}
