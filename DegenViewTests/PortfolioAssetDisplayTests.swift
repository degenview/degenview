import XCTest

@testable import DegenView

private final class CoinNamesURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> Data)?

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler, let url = request.url else { throw URLError(.badServerResponse) }
            let data = try handler(request)
            let response = try XCTUnwrap(
                HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

final class PortfolioAssetDisplayTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDown() {
        CoinNamesURLProtocol.handler = nil
        try? FileManager.default.removeItem(at: directory)
        super.tearDown()
    }

    private func asset(
        _ symbol: String, name: String? = nil, source: DataSourceType = .binance, key: String? = nil
    ) -> PortfolioAsset {
        PortfolioAsset(key: key ?? "\(source.rawValue):\(symbol)", symbol: symbol, name: name ?? symbol, source: source)
    }

    private func makeResolver() -> IconResolver {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CoinNamesURLProtocol.self]
        return IconResolver(
            session: URLSession(configuration: configuration), baseURL: "https://coingecko.test/api/v3",
            directory: directory, rateLimiter: CGRateLimiter(gap: 0))
    }

    private func coin(_ id: String, _ symbol: String, _ name: String) -> [String: String] {
        ["id": id, "symbol": symbol, "name": name, "image": "https://img.test/\(id).png"]
    }

    // MARK: - Ticker and embedded name

    func testDisplayTickerDropsTheTradingPair() {
        XCTAssertEqual(asset("BTC/USDT").displayTicker, "BTC")
        XCTAssertEqual(asset("ETH/USD", source: .coinbase).displayTicker, "ETH")
        XCTAssertEqual(asset("BONK/SOL", source: .dexscreener).displayTicker, "BONK")
    }

    func testDisplayTickerLeavesBareSymbolsAlone() {
        XCTAssertEqual(asset("BTC").displayTicker, "BTC")
        XCTAssertEqual(asset("btc").displayTicker, "BTC")
    }

    func testDisplayTickerDropsAnEquityCompanyName() {
        let apple = asset("AAPL — Apple Inc.", source: .alpaca)
        XCTAssertEqual(apple.displayTicker, "AAPL")
        XCTAssertEqual(apple.embeddedName, "Apple Inc.")
    }

    func testEmbeddedNameIsNilWithoutOne() {
        XCTAssertNil(asset("BTC/USDT").embeddedName)
        XCTAssertNil(asset("BTC").embeddedName)
    }

    // MARK: - Quantity

    func testQuantityFormatsGroupedWithAtMostEightDecimals() {
        // The user's locale decides the separators; the digits are what is under test.
        let point = Locale.current.decimalSeparator ?? "."
        let group = Locale.current.groupingSeparator ?? ","
        XCTAssertEqual(Decimal(string: "0.123456789012")!.portfolioQuantity, "0\(point)12345679")
        XCTAssertEqual(Decimal(string: "1234567.5")!.portfolioQuantity, "1\(group)234\(group)567\(point)5")
        XCTAssertEqual(Decimal(0).portfolioQuantity, "0")
        XCTAssertEqual(Decimal(string: "2.50000000")!.portfolioQuantity, "2\(point)5")
    }

    // MARK: - Subtitle

    func testSubtitleFallsBackToTheStoredLabel() {
        XCTAssertEqual(PortfolioAssetInfoViewModel.subtitle(for: asset("BTC/USDT"), resolvedName: nil), "BTC/USDT")
    }

    func testSubtitlePrefersALookedUpCoinName() {
        XCTAssertEqual(
            PortfolioAssetInfoViewModel.subtitle(for: asset("BTC/USDT"), resolvedName: "Bitcoin"), "Bitcoin")
    }

    func testSubtitlePrefersAStoredNameOverALookup() {
        let imported = asset("BTC", name: "Bitcoin (cold wallet)")
        XCTAssertEqual(
            PortfolioAssetInfoViewModel.subtitle(for: imported, resolvedName: "Bitcoin"), "Bitcoin (cold wallet)")
    }

    func testSubtitleIgnoresAStoredNameThatIsJustTheTicker() {
        let imported = asset("BTC", name: "BTC")
        XCTAssertEqual(PortfolioAssetInfoViewModel.subtitle(for: imported, resolvedName: "Bitcoin"), "Bitcoin")
    }

    func testSubtitleUsesTheNameAnEquitySymbolCarries() {
        let apple = asset("AAPL — Apple Inc.", source: .alpaca)
        XCTAssertEqual(PortfolioAssetInfoViewModel.subtitle(for: apple, resolvedName: nil), "Apple Inc.")
    }

    // MARK: - Coin names

    func testCoinNameBySymbolComesFromTheSnapshotAndHighestCapWins() async throws {
        CoinNamesURLProtocol.handler = { _ in
            try JSONSerialization.data(withJSONObject: [
                self.coin("bitcoin", "btc", "Bitcoin"), self.coin("batcat", "btc", "Batcat"),
            ])
        }

        let name = await makeResolver().coinName(forSymbol: "BTC")

        XCTAssertEqual(name, "Bitcoin")
    }

    func testCoinNameByIDComesFromTheBatchedLookup() async throws {
        var queried: [String] = []
        CoinNamesURLProtocol.handler = { request in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            queried = (items.first { $0.name == "ids" }?.value ?? "").split(separator: ",").map(String.init)
            return try JSONSerialization.data(withJSONObject: [self.coin("bonk", "bonk", "Bonk")])
        }

        let name = await makeResolver().coinName(forCoinID: "Bonk")

        XCTAssertEqual(name, "Bonk")
        XCTAssertEqual(queried, ["bonk"])
    }

    func testNamesPersistAcrossRelaunchWithoutRefetching() async throws {
        var calls = 0
        CoinNamesURLProtocol.handler = { _ in
            calls += 1
            return try JSONSerialization.data(withJSONObject: [self.coin("ethereum", "eth", "Ethereum")])
        }
        let first = makeResolver()
        _ = await first.coinName(forSymbol: "ETH")
        let relaunched = makeResolver()
        let name = await relaunched.coinName(forSymbol: "ETH")

        XCTAssertEqual(name, "Ethereum")
        XCTAssertEqual(calls, 1, "Names persist with the snapshot; a relaunch doesn't refetch")
    }
}
