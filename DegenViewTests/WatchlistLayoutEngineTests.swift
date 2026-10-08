import XCTest

@testable import DegenView

final class WatchlistLayoutEngineTests: XCTestCase {
    private func item(_ symbol: String, _ source: DataSourceType = .binance, name: String? = nil) -> WatchlistInstrument {
        WatchlistInstrument(
            instrument: InstrumentID(source: source, symbol: symbol), name: name ?? symbol, label: symbol)
    }

    /// ROOT | Majors: BTC ETH SOL | Watching: LINK  (insertion order is the manual order)
    private func sample() throws -> Watchlist {
        var list = Watchlist(name: "Crypto")
        try list.add(item("ROOT"))
        let majors = try list.addSection(title: "Majors")
        for symbol in ["BTCUSDT", "ETHUSDT", "SOLUSDT"] { try list.add(item(symbol), toSection: majors.id) }
        let watching = try list.addSection(title: "Watching")
        try list.add(item("LINKUSDT"), toSection: watching.id)
        return list
    }

    private func describe(_ rows: [WatchlistLayoutEngine.Row]) -> [String] {
        rows.map {
            switch $0 {
            case .section(let section, let count): return "#\(section.title)(\(count))"
            case .instrument(let item, _): return item.instrument.symbol
            case .emptySection: return "<empty>"
            }
        }
    }

    private func quotes(_ values: [String: Double?]) -> (InstrumentID) -> WatchlistQuote? {
        { id in
            guard let entry = values[id.symbol] else { return nil }
            return WatchlistQuote(last: entry, changePercent: entry, freshness: .live)
        }
    }

    func testManualOrderIsTheStoredOrder() throws {
        XCTAssertEqual(
            describe(WatchlistLayoutEngine.rows(in: try sample())),
            ["ROOT", "#Majors(3)", "BTCUSDT", "ETHUSDT", "SOLUSDT", "#Watching(1)", "LINKUSDT"])
    }

    func testSortingReordersInsideEachSectionAndLeavesHeadersAlone() throws {
        let rows = WatchlistLayoutEngine.rows(
            in: try sample(), sort: WatchlistSort(key: .changePercent, ascending: false),
            quote: quotes(["BTCUSDT": 1, "ETHUSDT": 5, "SOLUSDT": -2, "LINKUSDT": 9, "ROOT": 3]))

        XCTAssertEqual(
            describe(rows), ["ROOT", "#Majors(3)", "ETHUSDT", "BTCUSDT", "SOLUSDT", "#Watching(1)", "LINKUSDT"])
    }

    func testMarketsWithoutAValueSortLastInEitherDirection() throws {
        let values = quotes(["BTCUSDT": 1, "SOLUSDT": 3])
        for ascending in [true, false] {
            let rows = WatchlistLayoutEngine.rows(
                in: try sample(), sort: WatchlistSort(key: .last, ascending: ascending), quote: values)
            XCTAssertEqual(describe(rows).last, "LINKUSDT")
            XCTAssertEqual(describe(rows)[4], "ETHUSDT", "ETH has no quote so it goes last in Majors")
        }
    }

    func testSortingBySymbolIsAlphabeticalAndNeverMutatesTheList() throws {
        let list = try sample()
        let before = list
        let rows = WatchlistLayoutEngine.rows(in: list, sort: WatchlistSort(key: .symbol, ascending: true))

        XCTAssertEqual(describe(rows)[2...4].map { $0 }, ["BTCUSDT", "ETHUSDT", "SOLUSDT"])
        XCTAssertEqual(list, before)
        // Back to manual gives the stored order again.
        XCTAssertEqual(
            describe(WatchlistLayoutEngine.rows(in: list, sort: .manual)),
            describe(WatchlistLayoutEngine.rows(in: before)))
    }

    func testFilterMatchesSymbolNameAndProviderAndHidesEmptySections() throws {
        var list = try sample()
        try list.add(item("AAPL", .alpaca, name: "Apple Inc."))
        let engine = WatchlistLayoutEngine.self

        XCTAssertEqual(describe(engine.rows(in: list, filter: .init(text: "eth"))), ["#Majors(1)", "ETHUSDT"])
        // AAPL was appended, so it sits at the end of the last section.
        XCTAssertEqual(describe(engine.rows(in: list, filter: .init(text: "apple"))), ["#Watching(1)", "AAPL"])
        XCTAssertEqual(describe(engine.rows(in: list, filter: .init(text: "alpaca"))), ["#Watching(1)", "AAPL"])
        XCTAssertTrue(engine.rows(in: list, filter: .init(text: "nothing-like-this")).isEmpty)
    }

    func testFlagFilterUsesTheGlobalFlagMap() throws {
        let list = try sample()
        let flags = [InstrumentID(source: .binance, symbol: "SOLUSDT").key: WatchlistFlag.red]

        let rows = WatchlistLayoutEngine.rows(in: list, filter: .init(flag: .red), flags: flags)

        XCTAssertEqual(describe(rows), ["#Majors(1)", "SOLUSDT"])
    }

    func testCollapsedSectionKeepsItsHeaderAndCount() throws {
        var list = try sample()
        try list.setSectionCollapsed(try XCTUnwrap(list.sections.first).id, true)

        XCTAssertEqual(describe(WatchlistLayoutEngine.rows(in: list)), ["ROOT", "#Majors(3)", "#Watching(1)", "LINKUSDT"])
    }

    func testEmptyExpandedSectionShowsAPlaceholderButNotWhileFiltering() throws {
        var list = Watchlist(name: "Empty")
        try list.addSection(title: "Ideas")

        XCTAssertEqual(describe(WatchlistLayoutEngine.rows(in: list)), ["#Ideas(0)", "<empty>"])
        XCTAssertTrue(WatchlistLayoutEngine.rows(in: list, filter: .init(text: "x")).isEmpty)
    }
}
