import Combine
import XCTest

@testable import DegenView

@MainActor
final class WatchlistQuoteBookTests: XCTestCase {
    private var cancellables = Set<AnyCancellable>()
    private let btc = InstrumentID(source: .binance, symbol: "BTCUSDT")
    private let eth = InstrumentID(source: .binance, symbol: "ETHUSDT")

    func testAQuoteOnlyNotifiesItsOwnCell() {
        let book = WatchlistQuoteBook()
        var btcChanges = 0
        var ethChanges = 0
        book.cell(for: btc).$quote.dropFirst().sink { _ in btcChanges += 1 }.store(in: &cancellables)
        book.cell(for: eth).$quote.dropFirst().sink { _ in ethChanges += 1 }.store(in: &cancellables)

        book.apply([btc: WatchlistQuote(last: 1, freshness: .live)])

        XCTAssertEqual(btcChanges, 1)
        XCTAssertEqual(ethChanges, 0)
    }

    func testAnIdenticalQuoteDoesNotPublishAgain() {
        let book = WatchlistQuoteBook()
        let quote = WatchlistQuote(last: 1, freshness: .live)
        var changes = 0
        book.cell(for: btc).$quote.dropFirst().sink { _ in changes += 1 }.store(in: &cancellables)

        book.apply([btc: quote])
        book.apply([btc: quote])

        XCTAssertEqual(changes, 1)
    }

    func testRevisionIsCoalescedAcrossABurstOfUpdates() async throws {
        let book = WatchlistQuoteBook(revisionInterval: .milliseconds(20))
        var revisions: [Int] = []
        book.$revision.dropFirst().sink { revisions.append($0) }.store(in: &cancellables)

        for price in 1...50 { book.apply([btc: WatchlistQuote(last: Double(price), freshness: .live)]) }
        try await Task.sleep(for: .milliseconds(120))

        XCTAssertEqual(revisions, [1])
        XCTAssertEqual(book.quote(for: btc)?.last, 50)
    }

    func testMarkingASourceStaleKeepsTheLastKnownPrice() {
        let book = WatchlistQuoteBook()
        book.apply([btc: WatchlistQuote(last: 100, freshness: .live)])

        book.markAll(from: [.binance], as: .stale, among: [btc, eth])

        XCTAssertEqual(book.quote(for: btc)?.last, 100)
        XCTAssertEqual(book.quote(for: btc)?.freshness, .stale)
        XCTAssertNil(book.quote(for: eth)?.last)
    }
}
