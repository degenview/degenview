import XCTest

@testable import DegenView

final class MarketSymbolTests: XCTestCase {
    private func display(_ ticker: String, _ source: DataSourceType) -> String? {
        MarketSymbol(ticker: ticker, source: source)?.display
    }

    func testBinancePairsSplitOnTheirQuoteAsset() {
        XCTAssertEqual(display("BTCUSDT", .binance), "BTC/USDT")
        XCTAssertEqual(display("ethbtc", .binance), "ETH/BTC")
        XCTAssertEqual(display("BNBEUR", .binance), "BNB/EUR")
        XCTAssertEqual(display("SOLUSDC", .binance), "SOL/USDC")
    }

    func testBinanceMatchesTheLongestQuote() {
        XCTAssertEqual(display("BTCFDUSD", .binance), "BTC/FDUSD", "not BTCFD/USD")
        XCTAssertEqual(display("USDCUSDT", .binance), "USDC/USDT", "a stablecoin can be the base")
        XCTAssertEqual(display("BTCUSDT", .binance), "BTC/USDT", "not BTCUSDT split on USD")
    }

    func testBinanceIdWithoutAQuoteIsLeftAlone() {
        XCTAssertNil(MarketSymbol(ticker: "FOO", source: .binance))
        XCTAssertNil(MarketSymbol(ticker: "USDT", source: .binance), "A quote alone has no base")
    }

    func testCoinbaseProductIDsSplitOnTheHyphen() {
        XCTAssertEqual(display("BTC-USD", .coinbase), "BTC/USD")
        XCTAssertEqual(display("btc-usd", .coinbase), "BTC/USD")
        XCTAssertEqual(display("BTC/USD", .coinbase), "BTC/USD")
        XCTAssertEqual(display("ETH-BTC", .coinbase), "ETH/BTC")
        XCTAssertNil(MarketSymbol(ticker: "BTC", source: .coinbase))
        XCTAssertNil(MarketSymbol(ticker: "A-B-C", source: .coinbase))
    }

    func testOtherSourcesAreNotSplit() {
        XCTAssertNil(MarketSymbol(ticker: "bitcoin", source: .coingecko))
        XCTAssertNil(MarketSymbol(ticker: "BTCUSDT", source: .dexscreener))
        XCTAssertNil(MarketSymbol(ticker: "AAPL", source: .alpaca))
    }

    // MARK: - Chart labels

    private final class StubSource: TickerDataSource {
        let type: DataSourceType
        init(_ type: DataSourceType) { self.type = type }
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    @MainActor
    private func chart(_ ticker: String, _ source: DataSourceType, displayName: String? = nil) -> ChartViewModel {
        ChartViewModel(ticker: ticker, source: source, displayName: displayName, api: StubSource(source))
    }

    @MainActor
    func testExchangeChartsTitleAsBaseSlashQuote() {
        XCTAssertEqual(chart("BTCUSDT", .binance).title, "BTC/USDT")
        XCTAssertEqual(chart("BTC-USD", .coinbase).title, "BTC/USD")
        XCTAssertEqual(chart("eth-eur", .coinbase).title, "ETH/EUR")
    }

    @MainActor
    func testNonUSDTBinancePairsKeepTheirOwnQuote() {
        XCTAssertEqual(chart("ETHBTC", .binance).title, "ETH/BTC")
        XCTAssertEqual(chart("FLOKIEUR", .binance).title, "FLOKI/EUR")
    }

    @MainActor
    func testBareLegacyBinanceTickerTitlesAsThePairItFetches() {
        let legacy = chart("BTC", .binance)

        XCTAssertEqual(legacy.apiSymbol, "BTCUSDT")
        XCTAssertEqual(legacy.title, "BTC/USDT")
    }

    @MainActor
    func testIdentityStaysTheExchangeID() {
        let binance = chart("BTCUSDT", .binance)
        let coinbase = chart("BTC-USD", .coinbase)

        XCTAssertEqual(binance.ticker, "BTCUSDT")
        XCTAssertEqual(binance.iconKey, "Binance:BTCUSDT")
        XCTAssertEqual(coinbase.ticker, "BTC-USD")
        XCTAssertEqual(coinbase.apiSymbol, "BTC-USD")
        XCTAssertEqual(coinbase.iconKey, "Coinbase:BTC-USD")
    }

    @MainActor
    func testExplicitDisplayNameStillWinsAndOtherSourcesAreUnchanged() {
        XCTAssertEqual(chart("BTCUSDT", .binance, displayName: "My BTC").title, "My BTC")
        XCTAssertEqual(chart("bitcoin", .coingecko).title, "BITCOIN")
        XCTAssertEqual(chart("AAPL", .alpaca).title, "AAPL")
    }

    @MainActor
    func testBaseSymbolUsesTheSameSplit() {
        XCTAssertEqual(chart("BTCFDUSD", .binance).baseSymbol, "BTC")
        XCTAssertEqual(chart("ETHBTC", .binance).baseSymbol, "ETH")
        XCTAssertEqual(chart("BTC-USD", .coinbase).baseSymbol, "BTC")
        XCTAssertEqual(chart("FOO", .binance).baseSymbol, "FOO")
    }
}
