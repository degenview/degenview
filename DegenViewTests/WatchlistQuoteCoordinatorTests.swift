import GRDB
import XCTest

@testable import DegenView

/// Answers batches from a script and records every call.
private final class RecordingQuoteProvider: WatchlistQuoteProvider, @unchecked Sendable {
    struct Call: Equatable {
        let source: DataSourceType
        let symbols: [String]
    }

    typealias Handler = @Sendable (DataSourceType, [QuoteRequest]) throws -> [String: SourceQuote]

    private let lock = NSLock()
    private var recorded: [Call] = []
    private var handlers: [DataSourceType: Handler] = [:]
    private var chainLookups: [String] = []
    var chain: String?

    var calls: [Call] { lock.withLock { recorded } }
    var lookups: [String] { lock.withLock { chainLookups } }

    func calls(to source: DataSourceType) -> [Call] { calls.filter { $0.source == source } }

    func answer(_ source: DataSourceType, _ handler: @escaping Handler) {
        lock.withLock { handlers[source] = handler }
    }

    /// Prices every symbol asked for at 100, against a previous 90.
    func answerEverything(_ source: DataSourceType) {
        answer(source) { _, requests in
            Dictionary(
                uniqueKeysWithValues: requests.map { ($0.symbol, SourceQuote(price: 100, previousDayPrice: 90)) })
        }
    }

    func quotes(source: DataSourceType, requests: [QuoteRequest]) async throws -> [String: SourceQuote] {
        let handler = lock.withLock { () -> Handler? in
            recorded.append(Call(source: source, symbols: requests.map(\.symbol)))
            return handlers[source]
        }
        return try handler?(source, requests) ?? [:]
    }

    func resolveChain(address: String) async -> String? {
        lock.withLock { chainLookups.append(address) }
        return chain
    }

}

private final class FakeCoinbaseSocket: CoinbaseTickSource {
    var connects: [[String]] = []
    var disconnects = 0
    var onTick: ((CoinbaseTick) -> Void)?

    func connect(products: [String], onTick: @escaping (CoinbaseTick) -> Void) {
        connects.append(products)
        self.onTick = onTick
    }

    func disconnect() {
        disconnects += 1
        onTick = nil
    }
}

@MainActor
final class WatchlistQuoteCoordinatorTests: XCTestCase {
    private var provider: RecordingQuoteProvider!
    private var socket: FakeCoinbaseSocket!
    private var book: WatchlistQuoteBook!
    private var coordinator: WatchlistQuoteCoordinator!
    private var now = Date(timeIntervalSince1970: 1_790_000_000)

    private let btc = InstrumentID(source: .binance, symbol: "BTCUSDT")
    private let eth = InstrumentID(source: .binance, symbol: "ETHUSDT")
    private let sol = InstrumentID(source: .coingecko, symbol: "solana")

    override func setUp() {
        provider = RecordingQuoteProvider()
        socket = FakeCoinbaseSocket()
        book = WatchlistQuoteBook(revisionInterval: .milliseconds(1))
        coordinator = makeCoordinator()
    }

    private func makeCoordinator() -> WatchlistQuoteCoordinator {
        WatchlistQuoteCoordinator(
            provider: provider, book: book, coinbase: socket, clock: { [unowned self] in now }, runsLoop: false)
    }

    // MARK: Sharing

    func testAMarketInSeveralListsAndWindowsIsRequestedOnce() async {
        provider.answerEverything(.binance)
        // Three sidebars (two windows, one showing a second list) all hold BTC.
        for _ in 0..<3 { coordinator.update(consumer: UUID(), instruments: [btc, eth], isActive: true) }
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)

        await coordinator.pollOnce()

        XCTAssertEqual(provider.calls.count, 1)
        XCTAssertEqual(provider.calls[0].symbols.sorted(), ["BTCUSDT", "ETHUSDT"])
        XCTAssertEqual(coordinator.subscriberCount(for: btc), 4)
        XCTAssertEqual(book.quote(for: btc)?.last, 100)
    }

    func testReleasingOneConsumerKeepsTheMarketForTheOthers() async {
        provider.answerEverything(.binance)
        let first = UUID()
        let second = UUID()
        coordinator.update(consumer: first, instruments: [btc, eth], isActive: true)
        coordinator.update(consumer: second, instruments: [btc], isActive: true)

        coordinator.release(consumer: first)
        await coordinator.pollOnce()

        XCTAssertEqual(provider.calls.map(\.symbols), [["BTCUSDT"]])
        XCTAssertEqual(coordinator.subscribedInstruments, [btc])
    }

    func testChartsAndWatchlistsCanHoldTheSameMarketIndependently() async {
        // A chart keeps its own live feed; the coordinator neither depends on it nor disturbs it,
        // so a market on screen as a card and in a list is simply requested for the list.
        provider.answerEverything(.binance)
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)

        await coordinator.pollOnce()

        XCTAssertEqual(provider.calls.count, 1)
        XCTAssertEqual(book.quote(for: btc)?.freshness, .live)
    }

    // MARK: Lifecycle

    func testNothingPollsWhileEverySidebarIsHiddenAndTheLastPricesStay() async {
        provider.answerEverything(.binance)
        let id = UUID()
        coordinator.update(consumer: id, instruments: [btc], isActive: true)
        await coordinator.pollOnce()
        XCTAssertEqual(provider.calls.count, 1)

        coordinator.update(consumer: id, instruments: [btc], isActive: false)
        await coordinator.pollOnce()

        XCTAssertEqual(provider.calls.count, 1, "hidden sidebars cost no requests")
        XCTAssertEqual(book.quote(for: btc)?.last, 100, "the cache survives for when it comes back")
        XCTAssertTrue(coordinator.subscribedInstruments.isEmpty)
    }

    func testResumingDowngradesPricesThatAgedWhileHidden() async {
        provider.answerEverything(.binance)
        let id = UUID()
        coordinator.update(consumer: id, instruments: [btc], isActive: true)
        await coordinator.pollOnce()
        coordinator.update(consumer: id, instruments: [btc], isActive: false)

        now = now.addingTimeInterval(3_600)
        coordinator.update(consumer: id, instruments: [btc], isActive: true)

        XCTAssertEqual(book.quote(for: btc)?.freshness, .stale)
        XCTAssertEqual(book.quote(for: btc)?.last, 100, "old price kept, not blanked")
    }

    func testTheLoopOnlyRunsWhileSomeoneIsWatching() {
        let live = WatchlistQuoteCoordinator(
            provider: provider, book: book, coinbase: socket, clock: { [unowned self] in now }, runsLoop: true)
        let id = UUID()
        live.update(consumer: id, instruments: [btc], isActive: true)
        XCTAssertTrue(live.isRunning)
        live.update(consumer: id, instruments: [btc], isActive: false)
        XCTAssertFalse(live.isRunning)
        live.update(consumer: id, instruments: [btc], isActive: true)
        live.release(consumer: id)
        XCTAssertFalse(live.isRunning)
    }

    // MARK: Failure isolation

    func testOneSourceFailingLeavesTheOthersAndKeepsItsLastPrices() async {
        provider.answerEverything(.binance)
        provider.answerEverything(.coingecko)
        coordinator.update(consumer: UUID(), instruments: [btc, sol], isActive: true)
        await coordinator.pollOnce()

        provider.answer(.binance) { _, _ in throw WatchlistQuoteFailure.failed("offline") }
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()

        XCTAssertEqual(book.quote(for: btc)?.last, 100)
        XCTAssertEqual(book.quote(for: btc)?.freshness, .stale)
        XCTAssertEqual(book.quote(for: sol)?.freshness, .live)
    }

    func testAFailingSourceBacksOffInsteadOfBeingHammered() async {
        provider.answer(.binance) { _, _ in throw WatchlistQuoteFailure.rateLimited }
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)

        await coordinator.pollOnce()
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()
        XCTAssertEqual(provider.calls(to: .binance).count, 1, "still inside the backoff window")

        now = now.addingTimeInterval(10)
        await coordinator.pollOnce()
        XCTAssertEqual(provider.calls(to: .binance).count, 2)
    }

    func testAFailedQuoteIsNeverShownAsZero() async {
        provider.answer(.binance) { _, _ in throw WatchlistQuoteFailure.failed("offline") }
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)

        await coordinator.pollOnce()

        XCTAssertNil(book.quote(for: btc)?.last)
        XCTAssertEqual(book.quote(for: btc)?.freshness, .unavailable)
    }

    func testOneRejectedSymbolIsFoundWithoutBlankingTheRest() async {
        let bad = InstrumentID(source: .binance, symbol: "DELISTEDUSDT")
        provider.answer(.binance) { _, requests in
            if requests.contains(where: { $0.symbol == "DELISTEDUSDT" }) { throw WatchlistQuoteFailure.rejected("Invalid symbol") }
            return Dictionary(
                uniqueKeysWithValues: requests.map { ($0.symbol, SourceQuote(price: 100, previousDayPrice: 90)) })
        }
        coordinator.update(consumer: UUID(), instruments: [btc, eth, bad], isActive: true)

        await coordinator.pollOnce()

        XCTAssertEqual(book.quote(for: btc)?.last, 100)
        XCTAssertEqual(book.quote(for: eth)?.last, 100)
        XCTAssertEqual(book.quote(for: bad)?.freshness, .unavailable)
        XCTAssertNil(book.quote(for: bad)?.last)

        let callsAfterFirstRound = provider.calls(to: .binance).count
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()
        XCTAssertFalse(
            provider.calls.dropFirst(callsAfterFirstRound).contains { $0.symbols.contains("DELISTEDUSDT") },
            "a rejected market sits out the next rounds")
    }

    func testWhenEverythingIsRejectedNothingIsBlamedOnTheSymbols() async {
        provider.answer(.binance) { _, _ in throw WatchlistQuoteFailure.rejected("Bad request") }
        coordinator.update(consumer: UUID(), instruments: [btc, eth], isActive: true)

        await coordinator.pollOnce()
        let first = provider.calls(to: .binance).count
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()

        XCTAssertGreaterThan(provider.calls(to: .binance).count, first, "still retried; not parked as bad symbols")
    }

    func testMissingAlpacaKeysSayWhy() async {
        provider.answer(.alpaca) { _, _ in throw WatchlistQuoteFailure.credentialsMissing }
        let apple = InstrumentID(source: .alpaca, symbol: "AAPL")
        coordinator.update(consumer: UUID(), instruments: [apple], isActive: true)

        await coordinator.pollOnce()

        XCTAssertEqual(book.quote(for: apple)?.freshness, .unsupported)
        XCTAssertTrue(book.quote(for: apple)?.note?.contains("Alpaca") ?? false)
    }

    func testIndexChartsAreUnsupportedAndNeverRequested() async {
        let index = InstrumentID(source: .coinMarketCap, symbol: "fear-greed")
        coordinator.update(consumer: UUID(), instruments: [index], isActive: true)

        await coordinator.pollOnce()

        XCTAssertTrue(provider.calls.isEmpty)
        XCTAssertEqual(book.quote(for: index)?.freshness, .unsupported)
    }

    // MARK: Freshness

    func testAPriceStaysLiveUntilItsWindowPassesThenGoesStale() async {
        provider.answerEverything(.binance)
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)
        await coordinator.pollOnce()
        XCTAssertEqual(book.quote(for: btc)?.freshness, .live)

        coordinator.ageQuotes(now: now.addingTimeInterval(30))
        XCTAssertEqual(book.quote(for: btc)?.freshness, .live)
        coordinator.ageQuotes(now: now.addingTimeInterval(600))
        XCTAssertEqual(book.quote(for: btc)?.freshness, .stale)
    }

    func testAnOldProviderTimestampDoesNotMakeAFreshPriceStale() async {
        provider.answer(.coingecko) { _, requests in
            Dictionary(
                uniqueKeysWithValues: requests.map {
                    ($0.symbol, SourceQuote(price: 5, previousDayPrice: 4, timestamp: Date(timeIntervalSince1970: 1)))
                })
        }
        coordinator.update(consumer: UUID(), instruments: [sol], isActive: true)

        await coordinator.pollOnce()

        XCTAssertEqual(book.quote(for: sol)?.freshness, .live)
    }

    func testAFailedRoundRecoversWithinSecondsNotMinutes() async {
        provider.answerEverything(.binance)
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)
        await coordinator.pollOnce()

        provider.answer(.binance) { _, _ in throw WatchlistQuoteFailure.failed("offline") }
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()
        provider.answerEverything(.binance)
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()

        XCTAssertEqual(book.quote(for: btc)?.freshness, .live, "back within one backoff step of 5 seconds")
    }

    func testAFailureWithNoPriceYetKeepsItsReasonForTheRow() async {
        provider.answer(.coingecko) { _, _ in throw WatchlistQuoteFailure.rateLimited }
        coordinator.update(consumer: UUID(), instruments: [sol], isActive: true)

        await coordinator.pollOnce()

        XCTAssertNil(book.quote(for: sol)?.last)
        XCTAssertEqual(book.quote(for: sol)?.note, "Rate limited by the provider")
    }

    // MARK: Prediction markets

    func testPredictionMarketsAreReadLessOftenThanTheRest() async {
        provider.answerEverything(.binance)
        provider.answerEverything(.polymarket)
        let market = InstrumentID(source: .polymarket, symbol: "12345")
        coordinator.update(consumer: UUID(), instruments: [btc, market], isActive: true)

        await coordinator.pollOnce()
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()
        now = now.addingTimeInterval(30)
        await coordinator.pollOnce()

        XCTAssertEqual(provider.calls(to: .binance).count, 3)
        XCTAssertEqual(provider.calls(to: .polymarket).count, 2)
    }

    // MARK: DEX networks

    func testADexPairSavedWithoutANetworkIsLookedUpOnceAndThenPriced() async {
        provider.chain = "solana"
        provider.answerEverything(.dexscreener)
        let pair = InstrumentID(source: .dexscreener, symbol: "PairAddr")
        var resolved: [(InstrumentID, String)] = []
        coordinator.onChainResolved = { resolved.append(($0, $1)) }
        coordinator.update(consumer: UUID(), instruments: [pair], isActive: true)

        await coordinator.pollOnce()
        now = now.addingTimeInterval(5)
        await coordinator.pollOnce()

        XCTAssertEqual(provider.lookups, ["PairAddr"])
        XCTAssertEqual(resolved.count, 1)
        XCTAssertEqual(resolved.first?.1, "solana")
        XCTAssertEqual(book.quote(for: pair)?.last, 100)
    }

    func testAnUnresolvablePairSaysSoAndIsNotLookedUpAgain() async {
        provider.chain = nil
        let pair = InstrumentID(source: .dexscreener, symbol: "Gone")
        coordinator.update(consumer: UUID(), instruments: [pair], isActive: true)

        await coordinator.pollOnce()
        await coordinator.pollOnce()

        XCTAssertEqual(provider.lookups.count, 1)
        XCTAssertTrue(provider.calls.isEmpty)
        XCTAssertNotNil(book.quote(for: pair)?.note)
    }

    // MARK: Coinbase

    private let coinbaseBTC = InstrumentID(source: .coinbase, symbol: "BTC-USD")
    private let coinbaseETH = InstrumentID(source: .coinbase, symbol: "ETH-USD")

    func testCoinbaseUsesOneSharedSocketAndReconnectsOnlyWhenTheProductsChange() {
        let id = UUID()
        coordinator.update(consumer: id, instruments: [coinbaseBTC, coinbaseETH], isActive: true)
        coordinator.update(consumer: UUID(), instruments: [coinbaseBTC], isActive: true)
        coordinator.update(consumer: id, instruments: [coinbaseBTC, coinbaseETH], isActive: true)

        XCTAssertEqual(socket.connects, [["BTC-USD", "ETH-USD"]])

        coordinator.update(consumer: id, instruments: [coinbaseBTC], isActive: true)
        XCTAssertEqual(
            socket.connects, [["BTC-USD", "ETH-USD"], ["BTC-USD"]],
            "ETH is wanted by nobody now, BTC still by the second consumer: one reconnect")
    }

    func testCoinbaseSocketClosesWhenNobodyIsWatching() {
        let id = UUID()
        coordinator.update(consumer: id, instruments: [coinbaseBTC], isActive: true)
        coordinator.update(consumer: id, instruments: [coinbaseBTC], isActive: false)

        XCTAssertGreaterThan(socket.disconnects, 0)
        XCTAssertNil(socket.onTick)
    }

    func testCoinbaseTicksAreCoalescedIntoOneUpdatePerProduct() async throws {
        coordinator.update(consumer: UUID(), instruments: [coinbaseBTC], isActive: true)
        var changes = 0
        let cancellable = book.cell(for: coinbaseBTC).$quote.dropFirst().sink { _ in changes += 1 }

        for price in 1...20 {
            socket.onTick?(
                CoinbaseTick(
                    productID: "BTC-USD", price: Double(100 + price), size: 1, time: now, tradeID: Int64(price),
                    open24h: 100, volume24h: 5))
        }
        coordinator.flushCoinbase()

        XCTAssertEqual(changes, 1)
        XCTAssertEqual(book.quote(for: coinbaseBTC)?.last, 120)
        XCTAssertEqual(book.quote(for: coinbaseBTC)?.changePercent ?? 0, 20, accuracy: 1e-9)
        cancellable.cancel()
    }

    func testCoinbaseProductsTheSocketHasNotAnsweredAreFilledFromREST() async {
        provider.answerEverything(.coinbase)
        coordinator.update(consumer: UUID(), instruments: [coinbaseBTC, coinbaseETH], isActive: true)
        await coordinator.pollOnce()
        XCTAssertTrue(provider.calls(to: .coinbase).isEmpty, "give the socket a moment first")

        socket.onTick?(CoinbaseTick(productID: "BTC-USD", price: 1, size: 1, time: now, tradeID: 1))
        coordinator.flushCoinbase()
        now = now.addingTimeInterval(10)
        await coordinator.pollOnce()

        XCTAssertEqual(provider.calls(to: .coinbase).map(\.symbols), [["ETH-USD"]])
    }

    func testACoinbaseProductTheSocketHasGoneQuietOnIsRefreshedFromREST() async {
        provider.answerEverything(.coinbase)
        coordinator.update(consumer: UUID(), instruments: [coinbaseBTC], isActive: true)
        socket.onTick?(CoinbaseTick(productID: "BTC-USD", price: 1, size: 1, time: now, tradeID: 1))
        coordinator.flushCoinbase()

        now = now.addingTimeInterval(10)
        await coordinator.pollOnce()
        XCTAssertTrue(provider.calls(to: .coinbase).isEmpty)

        now = now.addingTimeInterval(25)
        await coordinator.pollOnce()
        XCTAssertEqual(provider.calls(to: .coinbase).map(\.symbols), [["BTC-USD"]])
    }

    // MARK: No persistence

    func testQuoteTrafficNeverWritesToTheDatabase() async throws {
        let database = try AppDatabase.makeInMemory()
        let store = WatchlistStore(database: database)
        let list = try store.createWatchlist(name: "Crypto")
        try store.addInstrument(
            WatchlistInstrument(instrument: btc, name: "BTC", label: "BTC"), to: list.id)
        func snapshot() throws -> [String] {
            try database.reader.read { db in
                try String.fetchAll(db, sql: "SELECT id || position || payload FROM watchlist ORDER BY position")
            }
        }
        let before = try snapshot()

        provider.answerEverything(.binance)
        coordinator.update(consumer: UUID(), instruments: [btc], isActive: true)
        for _ in 0..<5 {
            now = now.addingTimeInterval(5)
            await coordinator.pollOnce()
        }

        XCTAssertEqual(try snapshot(), before)
        XCTAssertEqual(store.lists.count, 2)
    }
}
