import XCTest

@testable import DegenView

final class ChartColumnTests: XCTestCase {
    func testLegacyLayoutReproducesTwoColumnRowMajorGrid() {
        let ids = (0..<5).map { _ in UUID() }

        let columns = ChartColumn.resolved(nil, chartIDs: ids)

        XCTAssertEqual(columns.count, 2)
        XCTAssertEqual(columns[0].chartIDs, [ids[0], ids[2], ids[4]])
        XCTAssertEqual(columns[1].chartIDs, [ids[1], ids[3]])
    }

    func testResolutionRemovesStaleDuplicateAndEmptyMembership() {
        let ids = (0..<4).map { _ in UUID() }
        let stale = UUID()
        let columns = [
            ChartColumn(chartIDs: [ids[0], ids[1], stale]),
            ChartColumn(chartIDs: [ids[1]]),
            ChartColumn(chartIDs: [ids[2]]),
        ]

        let resolved = ChartColumn.resolved(columns, chartIDs: ids)

        XCTAssertEqual(Set(resolved.flatMap(\.chartIDs)), Set(ids))
        XCTAssertEqual(resolved.flatMap(\.chartIDs).count, ids.count)
        XCTAssertFalse(resolved.contains { $0.chartIDs.isEmpty })
        XCTAssertFalse(resolved.flatMap(\.chartIDs).contains(stale))
    }

    func testChartTabColumnLayoutRoundTrips() throws {
        let configs = (0..<3).map { TickerConfig(symbol: "S\($0)", source: .binance) }
        let columns = [
            ChartColumn(chartIDs: [configs[0].chartID, configs[2].chartID]),
            ChartColumn(chartIDs: [configs[1].chartID]),
        ]
        let tab = ChartTab(tickerConfigs: configs, chartColumns: columns, layoutMode: .grid)

        let decoded = try JSONDecoder().decode(ChartTab.self, from: JSONEncoder().encode(tab))

        XCTAssertEqual(decoded.chartColumns, columns)
    }

    func testResponsiveColumnLimitUsesMinimumUsableWidth() {
        XCTAssertTrue(ChartLayout.canAddColumn(availableWidth: 900, currentColumnCount: 2))
        XCTAssertFalse(ChartLayout.canAddColumn(availableWidth: 839, currentColumnCount: 2))
    }

    func testUnevenDynamicColumnsSizeForTallestColumn() {
        let height = ChartLayout.gridCardHeight(available: 500, cardCount: 9, columnCount: 3)
        XCTAssertEqual(height, 156, accuracy: 0.0001)
        XCTAssertLessThanOrEqual(
            ChartLayout.gridOccupancy(available: 500, cardCount: 9, columnCount: 3),
            500
        )
    }
}
