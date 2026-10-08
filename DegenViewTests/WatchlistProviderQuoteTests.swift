import XCTest

@testable import DegenView

private final class WatchlistURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler, let url = request.url else { throw URLError(.badServerResponse) }
            let (status, data) = try handler(request)
            let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: status, httpVersion: nil, headerFields: nil))
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

/// What each provider's existing quote call now also reports, and how a row reads it.
final class WatchlistProviderQuoteTests: XCTestCase {
    private func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [WatchlistURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    override func tearDown() {
        WatchlistURLProtocol.handler = nil
        super.tearDown()
    }

    // MARK: Providers

    func testBinanceReportsDollarTurnoverOnDollarPairsAndUnitsOnOthers() async throws {
        WatchlistURLProtocol.handler = { _ in
            let body = """
                [{"symbol":"BTCUSDT","openPrice":"90000","lastPrice":"99000","volume":"10","quoteVolume":"950000","closeTime":1790000000000},
                 {"symbol":"ETHBTC","openPrice":"0.03","lastPrice":"0.031","volume":"500","quoteVolume":"15","closeTime":1790000000000}]
                """
            return (200, Data(body.utf8))
        }
        let service = BinanceAPIService(session: session())

        let quotes = try await service.fetchQuotes([
            QuoteRequest(symbol: "BTCUSDT", metadata: [:]), QuoteRequest(symbol: "ETHBTC", metadata: [:]),
        ])

        XCTAssertEqual(quotes["BTCUSDT"]?.volume24h, 950_000)
        XCTAssertEqual(quotes["BTCUSDT"]?.volumeKind, .quoteCurrency)
        XCTAssertEqual(quotes["BTCUSDT"]?.timestamp, Date(timeIntervalSince1970: 1_790_000_000))
        XCTAssertEqual(quotes["ETHBTC"]?.volume24h, 500)
        XCTAssertEqual(quotes["ETHBTC"]?.volumeKind, .base)
    }

    func testCoinbaseStatsPriceTheBaseVolumeInDollars() async throws {
        WatchlistURLProtocol.handler = { _ in
            (200, Data(#"{"open":"90","last":"100","volume":"1000"}"#.utf8))
        }
        let service = CoinbaseAPIService(session: session(), requestSpacing: 0)

        let quotes = try await service.fetchQuotes([QuoteRequest(symbol: "BTC-USD", metadata: [:])])

        XCTAssertEqual(quotes["BTC-USD"]?.volume24h, 100_000)
        XCTAssertEqual(quotes["BTC-USD"]?.volumeKind, .quoteCurrency)
        XCTAssertNotNil(quotes["BTC-USD"]?.timestamp)
    }

    func testCoinbaseVolumeStaysInUnitsOnACryptoQuotedProduct() {
        let volume = CoinbaseAPIService.volume(productID: "ETH-BTC", baseVolume: 40, price: 0.03)
        XCTAssertEqual(volume.value, 40)
        XCTAssertEqual(volume.kind, .base)
    }

    func testDexScreenerReportsVolumeAndNeedsTheChain() async throws {
        var paths: [String] = []
        WatchlistURLProtocol.handler = { request in
            paths.append(request.url?.path ?? "")
            let body = """
                {"pairs":[{"pairAddress":"PairAddr","priceUsd":"0.5","priceChange":{"h24":10.0},"volume":{"h24":12345.0}}]}
                """
            return (200, Data(body.utf8))
        }
        let service = DEXScreenerService(session: session())

        let quotes = try await service.fetchQuotes([
            QuoteRequest(symbol: "PairAddr", metadata: ["chain": "solana"]),
            QuoteRequest(symbol: "NoChain", metadata: [:]),
        ])

        XCTAssertEqual(paths.count, 1)
        XCTAssertTrue(paths[0].hasSuffix("/pairs/solana/PairAddr"))
        XCTAssertEqual(quotes["PairAddr"]?.volume24h, 12_345)
        XCTAssertEqual(quotes["PairAddr"]?.previousDayPrice ?? 0, 0.5 / 1.1, accuracy: 1e-9)
        XCTAssertNil(quotes["NoChain"])
    }

    func testAlpacaTimestampsWithNanosecondsParse() throws {
        let parsed = try XCTUnwrap(AlpacaAPIService.parseTimestamp("2026-10-08T14:30:00.123456789Z"))
        let whole = try XCTUnwrap(AlpacaAPIService.parseTimestamp("2026-10-08T14:30:00Z"))
        XCTAssertEqual(parsed.timeIntervalSince(whole), 0.123, accuracy: 0.001)
        XCTAssertNil(AlpacaAPIService.parseTimestamp(nil))
        XCTAssertNil(AlpacaAPIService.parseTimestamp("not a date"))
    }

    func testCoinbaseTickCarriesTheTwentyFourHourFields() throws {
        let frame: [String: Any] = [
            "type": "ticker", "product_id": "BTC-USD", "price": "100.5", "trade_id": NSNumber(value: 7),
            "time": "2026-10-08T14:30:00.123456Z", "last_size": "0.1", "open_24h": "90", "volume_24h": "1234.5",
        ]
        let tick = try XCTUnwrap(CoinbaseTick(json: frame))
        XCTAssertEqual(tick.open24h, 90)
        XCTAssertEqual(tick.volume24h, 1_234.5)
        var bare = frame
        bare["open_24h"] = nil
        XCTAssertNil(try XCTUnwrap(CoinbaseTick(json: bare)).open24h)
    }

    // MARK: Mapping

    private let now = Date(timeIntervalSince1970: 1_790_000_000)

    func testChangeIsAgainstTheProvidersOwnReferenceAndLabelledForIt() {
        let quote = SourceQuote(price: 105, previousDayPrice: 100)
        let crypto = WatchlistQuote(source: .binance, quote: quote, receivedAt: now)
        let stock = WatchlistQuote(source: .alpaca, quote: quote, receivedAt: now)

        XCTAssertEqual(crypto.change, 5)
        XCTAssertEqual(crypto.changePercent ?? 0, 5, accuracy: 1e-9)
        XCTAssertEqual(crypto.changeBasis, .rolling24h)
        XCTAssertEqual(stock.changeBasis, .previousClose)
    }

    func testAMissingOrZeroReferenceLeavesTheChangeUnavailableNotInvented() {
        for previous in [nil, 0.0, -1.0] as [Double?] {
            let quote = WatchlistQuote(
                source: .binance, quote: SourceQuote(price: 10, previousDayPrice: previous), receivedAt: now)
            XCTAssertEqual(quote.last, 10)
            XCTAssertNil(quote.change)
            XCTAssertNil(quote.changePercent)
        }
    }

    func testPredictionMarketsMoveInPercentagePointsOfProbability() {
        let quote = WatchlistQuote(
            source: .polymarket, quote: SourceQuote(price: 0.62, previousDayPrice: 0.50), receivedAt: now)

        XCTAssertEqual(quote.change ?? 0, 0.12, accuracy: 1e-9)
        XCTAssertEqual(quote.changePercent ?? 0, 12, accuracy: 1e-9)
        XCTAssertEqual(quote.changeBasis, .window)
        XCTAssertTrue(WatchlistQuoteFormat.percent(quote, source: .polymarket).hasSuffix("pp"))
    }

    func testAFreshPollIsCurrentWhateverTheProvidersOwnTimestampSays() {
        // A coin that last traded an hour ago still has the right price, and we just confirmed it.
        let quiet = SourceQuote(price: 1, previousDayPrice: 1, timestamp: now.addingTimeInterval(-3_600))
        let quote = WatchlistQuote(source: .coingecko, quote: quiet, receivedAt: now)
        XCTAssertEqual(quote.freshness, .live)
        XCTAssertEqual(quote.timestamp, now.addingTimeInterval(-3_600), "the provider's time is kept for display")
    }

    func testAPriceGoesStaleOnlyWhenWeStopRefreshingIt() {
        for source in [DataSourceType.binance, .coinbase, .coingecko, .dexscreener, .alpaca, .polymarket] {
            let window = WatchlistFreshness.maximumAge(for: source)
            let interval = WatchlistFreshness.refreshInterval(for: source)
            XCTAssertGreaterThan(window, interval * 2, "\(source) must survive a missed round")
            XCTAssertEqual(WatchlistFreshness.evaluate(source: source, receivedAt: now, now: now.addingTimeInterval(window - 1)).isCurrent, true)
            XCTAssertEqual(WatchlistFreshness.evaluate(source: source, receivedAt: now, now: now.addingTimeInterval(window + 1)), .stale)
        }
    }

    func testAStockOutsideUSHoursReadsMarketClosedAndStaysCurrent() {
        // Saturday 2026-10-10 12:00 UTC.
        let saturday = Date(timeIntervalSince1970: 1_791_633_600)
        let quote = WatchlistQuote(
            source: .alpaca, quote: SourceQuote(price: 1, previousDayPrice: 1), receivedAt: saturday)
        XCTAssertEqual(quote.freshness, .marketClosed)
        XCTAssertTrue(quote.freshness.isCurrent, "the close is still the current price, so the row is not dimmed")
        XCTAssertEqual(WatchlistQuote(source: .binance, quote: SourceQuote(price: 1, previousDayPrice: 1), receivedAt: saturday).freshness, .live)
    }

    func testCoinbaseTickMapsOpenAndVolume() {
        let tick = CoinbaseTick(
            productID: "BTC-USD", price: 110, size: 1, time: now, tradeID: 1, open24h: 100, volume24h: 10)
        let quote = WatchlistQuote(coinbaseTick: tick, receivedAt: now)

        XCTAssertEqual(quote.change, 10)
        XCTAssertEqual(quote.changePercent ?? 0, 10, accuracy: 1e-9)
        XCTAssertEqual(quote.volume, 1_100)
        XCTAssertEqual(quote.volumeKind, .quoteCurrency)
        XCTAssertEqual(quote.freshness, .live)
    }

    func testFailureClassification() {
        XCTAssertEqual(WatchlistQuoteFailure.classify(BinanceAPIError.rateLimited), .rateLimited)
        XCTAssertEqual(WatchlistQuoteFailure.classify(CoinGeckoError.rateLimited), .rateLimited)
        XCTAssertEqual(WatchlistQuoteFailure.classify(AlpacaError.credentialsMissing), .credentialsMissing)
        guard case .rejected = WatchlistQuoteFailure.classify(BinanceAPIError.httpError(400)) else {
            return XCTFail("a 400 is the provider refusing the request")
        }
        guard case .failed = WatchlistQuoteFailure.classify(BinanceAPIError.httpError(503)) else {
            return XCTFail("a 503 is the source being down")
        }
        guard case .failed = WatchlistQuoteFailure.classify(URLError(.notConnectedToInternet)) else {
            return XCTFail("no network is not a bad symbol")
        }
    }
}
