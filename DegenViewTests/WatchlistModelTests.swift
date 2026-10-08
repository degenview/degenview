import XCTest

@testable import DegenView

final class WatchlistModelTests: XCTestCase {
    private func item(_ symbol: String, _ source: DataSourceType = .binance) -> WatchlistInstrument {
        WatchlistInstrument(instrument: InstrumentID(source: source, symbol: symbol), name: symbol, label: symbol)
    }

    private func symbols(_ list: Watchlist) -> [String] {
        list.entries.map { entry in
            switch entry {
            case .instrument(let item): return item.instrument.symbol
            case .section(let section): return "#" + section.title
            }
        }
    }

    private func sample() throws -> Watchlist {
        var list = Watchlist(name: "Crypto")
        try list.add(item("ROOT"))
        let majors = try list.addSection(title: "Majors")
        try list.add(item("BTCUSDT"), toSection: majors.id)
        try list.add(item("ETHUSDT"), toSection: majors.id)
        let watching = try list.addSection(title: "Watching")
        try list.add(item("LINKUSDT"), toSection: watching.id)
        return list
    }

    func testAddAppendsAndInsertsAtTheEndOfASection() throws {
        var list = try sample()
        let majors = try XCTUnwrap(list.sections.first)
        try list.add(item("SOLUSDT"), toSection: majors.id)

        XCTAssertEqual(
            symbols(list), ["ROOT", "#Majors", "BTCUSDT", "ETHUSDT", "SOLUSDT", "#Watching", "LINKUSDT"])
    }

    func testOneMarketCannotAppearTwiceInAListButCanInTwoLists() throws {
        var first = try sample()
        XCTAssertThrowsError(try first.add(item("btcusdt"))) {
            XCTAssertEqual($0 as? WatchlistError, .duplicate("btcusdt"))
        }
        // Same text on another provider is another market.
        XCTAssertNoThrow(try first.add(item("BTCUSDT", .coinbase)))

        var second = Watchlist(name: "Ideas")
        XCTAssertNoThrow(try second.add(item("BTCUSDT")))
    }

    func testMoveInstrumentBetweenSections() throws {
        var list = try sample()
        let link = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "LINKUSDT")))
        let eth = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "ETHUSDT")))

        try list.move(link.id, before: eth.id)

        XCTAssertEqual(symbols(list), ["ROOT", "#Majors", "BTCUSDT", "LINKUSDT", "ETHUSDT", "#Watching"])
        XCTAssertEqual(list.owningSection(of: link.id)?.title, "Majors")
    }

    func testMoveToSectionAndToRoot() throws {
        var list = try sample()
        let btc = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "BTCUSDT")))
        let watching = try XCTUnwrap(list.sections.last)

        try list.moveInstrument(btc.id, toSection: watching.id)
        XCTAssertEqual(symbols(list), ["ROOT", "#Majors", "ETHUSDT", "#Watching", "LINKUSDT", "BTCUSDT"])

        try list.moveInstrument(btc.id, toSection: nil)
        XCTAssertEqual(symbols(list), ["ROOT", "BTCUSDT", "#Majors", "ETHUSDT", "#Watching", "LINKUSDT"])
    }

    func testMovingASectionCarriesItsInstruments() throws {
        var list = try sample()
        let majors = try XCTUnwrap(list.sections.first)

        try list.move(majors.id, before: nil)

        XCTAssertEqual(symbols(list), ["ROOT", "#Watching", "LINKUSDT", "#Majors", "BTCUSDT", "ETHUSDT"])
    }

    func testDroppingASectionIntoItselfChangesNothing() throws {
        var list = try sample()
        let majors = try XCTUnwrap(list.sections.first)
        let btc = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "BTCUSDT")))
        let before = list

        try list.move(majors.id, before: btc.id)
        XCTAssertEqual(list, before)
        try list.move(majors.id, before: majors.id)
        XCTAssertEqual(list, before)
    }

    func testDeletingASectionKeepsItsInstruments() throws {
        var list = try sample()
        let majors = try XCTUnwrap(list.sections.first)

        try list.deleteSection(majors.id)

        XCTAssertEqual(symbols(list), ["ROOT", "BTCUSDT", "ETHUSDT", "#Watching", "LINKUSDT"])
        XCTAssertEqual(list.instruments.count, 4)
    }

    func testCollapsedSectionsHideTheirInstrumentsFromVisibility() throws {
        var list = try sample()
        let majors = try XCTUnwrap(list.sections.first)
        try list.setSectionCollapsed(majors.id, true)

        XCTAssertEqual(list.visibleInstruments.map(\.instrument.symbol), ["ROOT", "LINKUSDT"])
    }

    func testDuplicatedListGetsNewIdentitiesButSameContent() throws {
        var list = try sample()
        list.display.sort = WatchlistSort(key: .changePercent, ascending: false)
        let copy = list.duplicated(name: "Crypto copy")

        XCTAssertNotEqual(copy.id, list.id)
        XCTAssertTrue(Set(copy.entries.map(\.id)).isDisjoint(with: list.entries.map(\.id)))
        XCTAssertEqual(symbols(copy), symbols(list))
        XCTAssertEqual(copy.display, list.display)
        XCTAssertFalse(copy.isFavorites)
        XCTAssertTrue(copy.validate().isEmpty)
    }

    func testBlankNamesAreRejected() throws {
        var list = try sample()
        XCTAssertThrowsError(try list.addSection(title: "   "))
        let majors = try XCTUnwrap(list.sections.first)
        XCTAssertThrowsError(try list.renameSection(majors.id, to: ""))
    }

    func testCodableRoundTripAndOldRowsDecodeWithDefaults() throws {
        let list = try sample()
        let decoded = try JSONDecoder().decode(Watchlist.self, from: JSONEncoder().encode(list))
        XCTAssertEqual(decoded, list)

        let id = UUID()
        let minimal = Data(#"{"id":"\#(id.uuidString)","name":"Old"}"#.utf8)
        let old = try JSONDecoder().decode(Watchlist.self, from: minimal)
        XCTAssertEqual(old.id, id)
        XCTAssertTrue(old.entries.isEmpty)
        XCTAssertEqual(old.display, WatchlistDisplaySettings())
        XCTAssertFalse(old.isFavorites)
    }

    func testSortNeverTouchesStoredOrder() throws {
        var list = try sample()
        let order = symbols(list)
        list.display.sort = WatchlistSort(key: .symbol, ascending: false)
        XCTAssertEqual(symbols(list), order)
    }

    func testSeveralEntriesMoveTogetherInTheirOwnOrder() throws {
        var list = try sample()
        let ids = try ["ETHUSDT", "BTCUSDT"].map { symbol in
            try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: symbol))).id
        }
        let link = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "LINKUSDT")))

        try list.move(ids, before: link.id)

        XCTAssertEqual(symbols(list), ["ROOT", "#Majors", "#Watching", "ETHUSDT", "BTCUSDT", "LINKUSDT"])
    }

    func testMovingSeveralToTheEndAndAnUnknownIdMovesNothing() throws {
        var list = try sample()
        let root = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "ROOT")))
        try list.move([root.id], before: nil)
        XCTAssertEqual(symbols(list).last, "ROOT")

        let before = list
        XCTAssertThrowsError(try list.move([root.id, UUID()], before: nil))
        XCTAssertEqual(list, before)
    }

    func testSubtitleNamesTheProviderInsteadOfRepeatingTheTitle() {
        let crypto = WatchlistInstrument(
            instrument: InstrumentID(source: .binance, symbol: "BTCUSDT"), name: "BTC/USDT", label: "BTC/USDT")
        XCTAssertEqual(crypto.subtitle, "Binance")

        let sameIgnoringCase = WatchlistInstrument(
            instrument: InstrumentID(source: .coinbase, symbol: "ETH-USD"), name: "ETH/USD", label: "eth/usd")
        XCTAssertEqual(sameIgnoringCase.subtitle, "Coinbase")
    }

    func testSubtitleKeepsALabelThatSaysSomethingTheTitleDoesNot() {
        let stock = WatchlistInstrument(
            instrument: InstrumentID(source: .alpaca, symbol: "AAPL"), name: "Apple Inc.", label: "AAPL")
        XCTAssertEqual(stock.subtitle, "AAPL · Alpaca (IEX)")

        let outcome = WatchlistInstrument(
            instrument: InstrumentID(source: .polymarket, symbol: "123"), name: "Tweets this week?", label: "280-299")
        XCTAssertEqual(outcome.subtitle, "280-299 · Polymarket")

        let empty = WatchlistInstrument(instrument: InstrumentID(source: .kalshi, symbol: "A/B"), name: "X", label: "")
        XCTAssertEqual(empty.subtitle, "Kalshi")
    }

    func testSeveralInstrumentsMoveToTheEndOfASectionInOrder() throws {
        var list = try sample()
        let watching = try XCTUnwrap(list.sections.last)
        let ids = try ["ETHUSDT", "BTCUSDT"].map { try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: $0))).id }

        try list.moveInstruments(ids, toSection: watching.id)

        XCTAssertEqual(symbols(list), ["ROOT", "#Majors", "#Watching", "LINKUSDT", "ETHUSDT", "BTCUSDT"])
    }

    func testSeveralInstrumentsMoveToTheRoot() throws {
        var list = try sample()
        let link = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "LINKUSDT")))

        try list.moveInstruments([link.id], toSection: nil)

        XCTAssertEqual(symbols(list), ["ROOT", "LINKUSDT", "#Majors", "BTCUSDT", "ETHUSDT", "#Watching"])
    }

    func testMovingInstrumentsMovesNothingIfAnythingIsUnknown() throws {
        var list = try sample()
        let before = list
        let root = try XCTUnwrap(list.entry(for: InstrumentID(source: .binance, symbol: "ROOT")))
        let majors = try XCTUnwrap(list.sections.first)

        XCTAssertThrowsError(try list.moveInstruments([root.id, UUID()], toSection: majors.id))
        XCTAssertThrowsError(try list.moveInstruments([root.id], toSection: UUID()))
        XCTAssertThrowsError(try list.moveInstruments([majors.id], toSection: nil), "a heading is not an instrument")
        XCTAssertEqual(list, before)
    }
}
