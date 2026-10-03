import XCTest

@testable import DegenView

final class PaperQuoteFeedTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_700_000_000)

    private func dec(_ string: String) -> Decimal {
        Decimal(string: string, locale: Locale(identifier: "en_US_POSIX"))!
    }

    private var stock: PaperInstrument {
        .init(
            key: "test:XYZ", symbol: "XYZ", displayName: "XYZ", source: .alpaca, assetClass: .stock,
            quoteCurrency: .USD, tickSize: 1, minimumQuantity: 1, quantityIncrement: 1, contractMultiplier: 1,
            pointValue: 1)
    }

    // MARK: Coinbase

    func testCoinbaseTickCarriesBestBidAndAsk() throws {
        let frame = """
            {"type":"ticker","product_id":"BTC-USD","price":"84080.23","best_bid":"84080.22",\
            "best_ask":"84080.23","time":"2026-09-30T17:28:25.629380Z","trade_id":7,"last_size":"0.5"}
            """
        let json = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(frame.utf8)) as? [String: Any])
        let tick = try XCTUnwrap(CoinbaseTick(json: json))
        XCTAssertEqual(tick.bestBid, 84080.22)
        XCTAssertEqual(tick.bestAsk, 84080.23)
    }

    func testCoinbaseTickWithoutABookHasNilBidAndAsk() throws {
        let frame = """
            {"type":"ticker","product_id":"BTC-USD","price":"84080.23",\
            "time":"2026-09-30T17:28:25.629380Z","trade_id":7}
            """
        let json = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(frame.utf8)) as? [String: Any])
        let tick = try XCTUnwrap(CoinbaseTick(json: json))
        XCTAssertNil(tick.bestBid)
        XCTAssertNil(tick.bestAsk)
    }

    // MARK: BookTickerCoalescer

    /// Holds scheduled flushes until a test fires them.
    private final class ManualScheduler {
        var blocks: [() -> Void] = []
        var delays: [TimeInterval] = []
        func schedule(_ delay: TimeInterval, _ block: @escaping () -> Void) {
            delays.append(delay)
            blocks.append(block)
        }
        func fire() {
            let pending = blocks
            blocks = []
            pending.forEach { $0() }
        }
    }

    func testCoalescerKeepsTheLatestQuotePerSymbolAndFlushesOnce() {
        let scheduler = ManualScheduler()
        let coalescer = BookTickerCoalescer(delay: 0.25, schedule: scheduler.schedule)
        var delivered: [String: BookTickerCoalescer.Quote] = [:]
        var deliveries = 0
        coalescer.deliver = { symbol, quote in
            delivered[symbol] = quote
            deliveries += 1
        }

        coalescer.submit(symbol: "BTCUSDT", bid: 1, ask: 2)
        coalescer.submit(symbol: "BTCUSDT", bid: 3, ask: 4)
        coalescer.submit(symbol: "ETHUSDT", bid: 5, ask: 6)
        XCTAssertEqual(scheduler.blocks.count, 1, "one flush covers every symbol")
        XCTAssertEqual(scheduler.delays, [0.25])
        XCTAssertEqual(deliveries, 0, "nothing is delivered before the flush")

        scheduler.fire()
        XCTAssertEqual(deliveries, 2)
        XCTAssertEqual(delivered["BTCUSDT"], .init(bid: 3, ask: 4))
        XCTAssertEqual(delivered["ETHUSDT"], .init(bid: 5, ask: 6))

        coalescer.submit(symbol: "BTCUSDT", bid: 7, ask: 8)
        XCTAssertEqual(scheduler.blocks.count, 1, "a later burst schedules a new flush")
        scheduler.fire()
        XCTAssertEqual(delivered["BTCUSDT"], .init(bid: 7, ask: 8))
    }

    func testCancelledCoalescerDeliversNothingFromAnOldFlush() {
        let scheduler = ManualScheduler()
        let coalescer = BookTickerCoalescer(schedule: scheduler.schedule)
        var deliveries = 0
        coalescer.deliver = { _, _ in deliveries += 1 }
        coalescer.submit(symbol: "BTCUSDT", bid: 1, ask: 2)
        coalescer.cancel()
        scheduler.fire()
        XCTAssertEqual(deliveries, 0)

        coalescer.submit(symbol: "BTCUSDT", bid: 3, ask: 4)
        scheduler.fire()
        XCTAssertEqual(deliveries, 1, "a new submission after cancel still works")
    }

    // MARK: Binance socket

    private let bookFrame = """
        {"stream":"btcusdt@bookTicker","data":{"u":1,"s":"BTCUSDT","b":"84080.22","B":"1.2","a":"84080.23","A":"0.5"}}
        """
    private let klineFrame = """
        {"stream":"btcusdt@kline_1m","data":{"e":"kline","k":{"t":1700000000000,"o":"1","h":"2","l":"0.5",\
        "c":"1.5","v":"10","q":"15","x":false}}}
        """

    func testBinanceSubscribesToBookTickerOnlyWhenAsked() {
        var opened: [URL] = []
        let service = BinanceWebSocketService(socketOpenObserver: { opened.append($0) })
        service.connect(symbols: ["BTCUSDT"], interval: "1m") { _, _ in }
        service.connect(symbols: ["BTCUSDT"], interval: "1m", onUpdate: { _, _ in }, onBookTicker: { _, _, _ in })

        XCTAssertEqual(opened.count, 2)
        XCTAssertTrue(opened[0].absoluteString.hasSuffix("streams=btcusdt@kline_1m"))
        XCTAssertTrue(opened[1].absoluteString.hasSuffix("streams=btcusdt@kline_1m/btcusdt@bookTicker"))
    }

    func testBinanceRoutesBookFramesToTheCoalescerAndKlinesToTheUpdateCallback() {
        let scheduler = ManualScheduler()
        let service = BinanceWebSocketService(
            socketOpenObserver: { _ in }, bookCoalescer: BookTickerCoalescer(schedule: scheduler.schedule))
        var klines: [(String, Double)] = []
        var books: [(String, Double, Double)] = []
        service.connect(
            symbols: ["BTCUSDT"], interval: "1m",
            onUpdate: { symbol, kline in klines.append((symbol, kline.closePrice)) },
            onBookTicker: { symbol, bid, ask in books.append((symbol, bid, ask)) })

        service.handleMessage(klineFrame)
        service.handleMessage(bookFrame)
        XCTAssertEqual(klines.count, 1)
        XCTAssertEqual(klines.first?.0, "BTCUSDT")
        XCTAssertEqual(klines.first?.1, 1.5)
        XCTAssertTrue(books.isEmpty, "book quotes wait for the coalescer's flush")

        scheduler.fire()
        XCTAssertEqual(books.count, 1)
        XCTAssertEqual(books.first?.0, "BTCUSDT")
        XCTAssertEqual(books.first?.1, 84080.22)
        XCTAssertEqual(books.first?.2, 84080.23)
    }

    func testBinanceIgnoresBookFramesWhenNotSubscribed() {
        let scheduler = ManualScheduler()
        let service = BinanceWebSocketService(
            socketOpenObserver: { _ in }, bookCoalescer: BookTickerCoalescer(schedule: scheduler.schedule))
        service.connect(symbols: ["BTCUSDT"], interval: "1m") { _, _ in }
        service.handleMessage(bookFrame)
        XCTAssertTrue(scheduler.blocks.isEmpty)
    }

    // MARK: PaperQuoteSample

    func testSampleKeepsAFreshBookAndDropsAStaleOne() {
        let fresh = PaperQuoteSample.make(
            last: 100, bid: 99, ask: 101, bookUpdatedAt: now.addingTimeInterval(-5), refreshedAt: now,
            isConnected: true, now: now)
        XCTAssertEqual(fresh.bid, 99)
        XCTAssertEqual(fresh.ask, 101)

        let stale = PaperQuoteSample.make(
            last: 100, bid: 99, ask: 101, bookUpdatedAt: now.addingTimeInterval(-20), refreshedAt: now,
            isConnected: true, now: now)
        XCTAssertNil(stale.bid)
        XCTAssertNil(stale.ask)
        XCTAssertEqual(stale.last, 100)

        let never = PaperQuoteSample.make(
            last: 100, bid: nil, ask: nil, bookUpdatedAt: nil, refreshedAt: now, isConnected: true, now: now)
        XCTAssertNil(never.bid)
    }

    func testSampleDropsACrossedOrNonPositiveBook() {
        let crossed = PaperQuoteSample.make(
            last: 100, bid: 102, ask: 101, bookUpdatedAt: now, refreshedAt: now, isConnected: true, now: now)
        XCTAssertNil(crossed.bid)
        XCTAssertNil(crossed.ask)
        let zero = PaperQuoteSample.make(
            last: 100, bid: 0, ask: 0, bookUpdatedAt: now, refreshedAt: now, isConnected: true, now: now)
        XCTAssertNil(zero.bid)
    }

    func testSampleChangesWhenOnlyTheRefreshTimeDoes() {
        let first = PaperQuoteSample.make(
            last: 100, bid: nil, ask: nil, bookUpdatedAt: nil, refreshedAt: now, isConnected: true, now: now)
        let second = PaperQuoteSample.make(
            last: 100, bid: nil, ask: nil, bookUpdatedAt: nil, refreshedAt: now.addingTimeInterval(5),
            isConnected: true, now: now)
        XCTAssertNotEqual(first, second, "an unchanged price must still count as news")
    }

    func testDecimalConversionHasNoBinaryNoise() {
        XCTAssertEqual(PaperQuoteSample.decimal(84080.22), dec("84080.22"))
        XCTAssertEqual(PaperQuoteSample.decimal(0.00001234), dec("0.00001234"))
        XCTAssertEqual(PaperQuoteSample.decimal(3), 3)
    }

    // MARK: ChartLiveQuote

    @MainActor
    func testLiveQuoteIgnoresIncompleteOrInsaneBooks() {
        let quote = ChartLiveQuote()
        quote.apply(bid: 99, ask: 101, at: now)
        XCTAssertEqual(quote.bid, 99)
        XCTAssertEqual(quote.ask, 101)
        XCTAssertEqual(quote.updatedAt, now)

        quote.apply(bid: nil, ask: nil, at: now.addingTimeInterval(1))
        quote.apply(bid: 105, ask: 101, at: now.addingTimeInterval(1))
        quote.apply(bid: 0, ask: 1, at: now.addingTimeInterval(1))
        XCTAssertEqual(quote.bid, 99)
        XCTAssertEqual(quote.updatedAt, now, "a frame without a usable book never counts as an update")

        quote.clear()
        XCTAssertNil(quote.bid)
        XCTAssertNil(quote.updatedAt)
    }

    // MARK: Engine persistence

    private final class SaveCounter: @unchecked Sendable {
        private let lock = NSLock()
        private var value = 0
        func increment() { lock.withLock { value += 1 } }
        var count: Int { lock.withLock { value } }
    }

    func testAQuoteThatCannotMoveAnOrderIsNotSaved() async throws {
        let counter = SaveCounter()
        let engine = PaperTradingEngine(now: { self.now }, persist: { _ in counter.increment() })
        let account = try await engine.createAccount(name: "Paper", initialBalance: 10_000)
        let afterCreate = counter.count

        try await engine.process(.init(instrumentKey: stock.key, bid: 99, ask: 100, last: 100, timestamp: now))
        XCTAssertEqual(counter.count, afterCreate, "a quote with no working orders writes nothing")

        _ = try await engine.submit(
            .init(
                accountID: account, instrument: stock, side: .buy, type: .limit, quantity: 1, limitPrice: 50))
        let afterSubmit = counter.count
        try await engine.process(.init(instrumentKey: stock.key, bid: 99, ask: 100, last: 100, timestamp: now))
        XCTAssertGreaterThan(counter.count, afterSubmit, "a quote that is checked against a working order is saved")
    }

    // MARK: Store

    @MainActor
    private func makeStore(directory: URL, flushInterval: TimeInterval = 0.02) throws -> PaperTradingStore {
        PaperTradingStore(
            database: try AppDatabase(path: directory.appendingPathComponent("p.sqlite").path),
            quoteFlushInterval: flushInterval)
    }

    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    @MainActor
    func testStreamedQuotesCoalesceToTheLatest() async throws {
        let store = try makeStore(directory: try temporaryDirectory())
        await store.connect()
        for price in ["100", "101", "102"] {
            store.stream(instrument: stock, bid: dec(price) - 1, ask: dec(price) + 1, last: dec(price))
        }
        XCTAssertNil(store.snapshot.quotes[stock.key], "nothing lands before the flush interval")

        try await Task.sleep(nanoseconds: 300_000_000)
        let quote = try XCTUnwrap(store.snapshot.quotes[stock.key])
        XCTAssertEqual(quote.last, 102)
        XCTAssertEqual(quote.bid, 101)
        XCTAssertEqual(quote.ask, 103)
    }

    @MainActor
    func testAStoreDoesNotRestoreQuotesFromAnEarlierRun() async throws {
        let directory = try temporaryDirectory()
        let first = try makeStore(directory: directory)
        await first.connect()
        await first.process(instrument: stock, bid: 99, ask: 101, last: 100, timestamp: Date())
        let accountID = try XCTUnwrap(first.selectedAccount?.id)
        _ = await first.submit(
            .init(accountID: accountID, instrument: stock, side: .buy, type: .limit, quantity: 1, limitPrice: 50))
        XCTAssertNotNil(first.snapshot.quotes[stock.key])

        let second = try makeStore(directory: directory)
        XCTAssertTrue(second.snapshot.quotes.isEmpty, "last session's price must not reach the ticket")
        XCTAssertEqual(second.snapshot.accounts.count, 1, "the rest of the account is restored")
    }
}
