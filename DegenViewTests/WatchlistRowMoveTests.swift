import XCTest

@testable import DegenView

/// `resolveMove` turns the list's native row move into a stored move. Rows here are what the sidebar shows:
/// ROOT, #Majors, A, B, #Watching, C, #Empty, <placeholder>.
final class WatchlistRowMoveTests: XCTestCase {
    private var rows: [WatchlistLayoutEngine.Row] = []
    private var ids: [String: UUID] = [:]

    override func setUpWithError() throws {
        var list = Watchlist(name: "Crypto")
        func item(_ symbol: String) -> WatchlistInstrument {
            WatchlistInstrument(instrument: InstrumentID(source: .binance, symbol: symbol), name: symbol, label: symbol)
        }
        try list.add(item("ROOT"))
        let majors = try list.addSection(title: "Majors")
        try list.add(item("A"), toSection: majors.id)
        try list.add(item("B"), toSection: majors.id)
        let watching = try list.addSection(title: "Watching")
        try list.add(item("C"), toSection: watching.id)
        let empty = try list.addSection(title: "Empty")
        rows = WatchlistLayoutEngine.rows(in: list)
        for row in rows {
            switch row {
            case .instrument(let item, _): ids[item.instrument.symbol] = item.id
            case .section(let section, _): ids["#" + section.title] = section.id
            case .emptySection: break
            }
        }
        _ = empty
    }

    private func row(_ name: String) -> Int {
        rows.firstIndex { $0.entryID == ids[name] } ?? -1
    }

    private func move(_ name: String, to destination: Int) -> WatchlistLayoutEngine.Move? {
        WatchlistLayoutEngine.resolveMove(rows: rows, sources: IndexSet(integer: row(name)), destination: destination)
    }

    func testRowsAreAsExpected() {
        XCTAssertEqual(rows.count, 8)
        if case .emptySection = rows[7] {} else { XCTFail("last row should be the placeholder") }
    }

    func testDropBetweenTwoSymbolsOfASectionGoesBeforeTheLowerOne() {
        let result = move("ROOT", to: row("B"))
        XCTAssertEqual(result, .init(ids: [ids["ROOT"]!], before: ids["B"]))
    }

    func testDropUnderAHeadingIsTheTopOfThatSection() {
        let result = move("C", to: row("#Majors") + 1)
        XCTAssertEqual(result, .init(ids: [ids["C"]!], before: ids["A"]))
    }

    func testDropJustAboveTheNextHeadingIsTheEndOfTheSectionAbove() {
        // Between B (last of Majors) and #Watching.
        let result = move("ROOT", to: row("#Watching"))
        XCTAssertEqual(result, .init(ids: [ids["ROOT"]!], before: ids["#Watching"]))
    }

    func testDropOnAnEmptySectionsPlaceholderResolvesToTheNextRealEntryOrTheEnd() {
        let result = move("A", to: 7)
        XCTAssertEqual(result, .init(ids: [ids["A"]!], before: nil), "nothing real follows the placeholder: the end")
    }

    func testTopAndBottomOfTheList() {
        XCTAssertEqual(move("C", to: 0), .init(ids: [ids["C"]!], before: ids["ROOT"]))
        XCTAssertEqual(move("A", to: rows.count), .init(ids: [ids["A"]!], before: nil))
    }

    func testDraggingAHeadingResolvesLikeAnyRow() {
        XCTAssertEqual(move("#Majors", to: rows.count), .init(ids: [ids["#Majors"]!], before: nil))
        XCTAssertEqual(move("#Watching", to: 0), .init(ids: [ids["#Watching"]!], before: ids["ROOT"]))
    }

    func testSeveralRowsMoveTogetherInDisplayOrder() {
        let sources = IndexSet([row("B"), row("A")])
        let result = WatchlistLayoutEngine.resolveMove(rows: rows, sources: sources, destination: row("C"))
        XCTAssertEqual(result, .init(ids: [ids["A"]!, ids["B"]!], before: ids["C"]))
    }

    func testDroppingOnOrBesideItselfChangesNothing() {
        XCTAssertNil(move("A", to: row("A")))
        XCTAssertNil(move("A", to: row("A") + 1), "just below itself is where it already is")
        XCTAssertNil(move("C", to: row("#Empty")), "C already sits just before the empty section's heading")
    }

    func testAPlaceholderCannotBeDragged() {
        XCTAssertNil(WatchlistLayoutEngine.resolveMove(rows: rows, sources: IndexSet(integer: 7), destination: 0))
        XCTAssertNil(WatchlistLayoutEngine.resolveMove(rows: rows, sources: IndexSet(integer: 99), destination: 0))
    }

    func testTheDestinationNeverNamesAMovedRow() {
        let sources = IndexSet([row("A"), row("B")])
        // Destination points at B itself: the target skips ahead to the next unmoved entry.
        let result = WatchlistLayoutEngine.resolveMove(rows: rows, sources: sources, destination: row("B"))
        XCTAssertNil(result, "A and B dropped onto B stay where they are")
    }

    // MARK: Section-only moves (a sort or filter is on)

    private func sectionMove(_ names: [String], to destination: Int) -> WatchlistLayoutEngine.SectionMove? {
        WatchlistLayoutEngine.resolveSectionMove(
            rows: rows, sources: IndexSet(names.map(row)), destination: destination)
    }

    func testSectionMoveBelowASymbolOfAnotherSectionTargetsThatSection() {
        // Drop ROOT under C: C's section is Watching.
        XCTAssertEqual(sectionMove(["ROOT"], to: row("C") + 1), .init(ids: [ids["ROOT"]!], section: ids["#Watching"]))
    }

    func testSectionMoveUnderAHeadingTargetsThatSection() {
        XCTAssertEqual(sectionMove(["C"], to: row("#Majors") + 1), .init(ids: [ids["C"]!], section: ids["#Majors"]))
    }

    func testSectionMoveJustAboveTheNextHeadingIsTheEndOfTheSectionAbove() {
        // Between B (Majors) and #Watching: Majors.
        XCTAssertEqual(sectionMove(["C"], to: row("#Watching")), .init(ids: [ids["C"]!], section: ids["#Majors"]))
    }

    func testSectionMoveAboveEverythingIsTheRoot() {
        XCTAssertEqual(sectionMove(["A"], to: 0), .init(ids: [ids["A"]!], section: nil))
    }

    func testSectionMoveOntoAnEmptySectionsPlaceholderTargetsThatSection() {
        XCTAssertEqual(sectionMove(["A"], to: 8), .init(ids: [ids["A"]!], section: ids["#Empty"]))
        XCTAssertEqual(sectionMove(["A"], to: 7), .init(ids: [ids["A"]!], section: ids["#Empty"]))
    }

    func testSectionMoveWithinTheSameSectionIsNothing() {
        XCTAssertNil(sectionMove(["A"], to: row("B") + 1), "A dropped below B stays in Majors")
        XCTAssertNil(sectionMove(["A"], to: row("A")), "A dropped above itself stays in Majors")
        XCTAssertNil(sectionMove(["ROOT"], to: 0), "ROOT dropped at the top stays at the root")
    }

    func testSectionMoveOnlyMovesTheSymbolsNotAlreadyThere() {
        // A and C dropped under B: A is already in Majors, C is not.
        XCTAssertEqual(
            sectionMove(["A", "C"], to: row("B") + 1), .init(ids: [ids["C"]!], section: ids["#Majors"]))
    }

    func testSectionMoveIgnoresHeadingsAndPlaceholders() {
        XCTAssertNil(sectionMove(["#Majors"], to: rows.count))
        XCTAssertNil(WatchlistLayoutEngine.resolveSectionMove(rows: rows, sources: IndexSet(integer: 7), destination: 0))
        XCTAssertNil(WatchlistLayoutEngine.resolveSectionMove(rows: rows, sources: IndexSet(integer: 99), destination: 0))
    }
}
