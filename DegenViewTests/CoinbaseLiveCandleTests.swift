import XCTest

@testable import DegenView

@MainActor
final class CoinbaseLiveCandleTests: XCTestCase {
    /// Tuesday 2026-09-22 12:00 UTC.
    private let hour = Date(timeIntervalSince1970: 1_790_078_400)

    private func makeChart(_ candle: KlineData) -> ChartViewModel {
        let chart = ChartViewModel(ticker: "BTC-USD", source: .coinbase, api: EmptySource())
        chart.klineData = [candle]
        return chart
    }

    private func candle(at time: Date) -> KlineData {
        KlineData(
            openTime: time, openPrice: 100, highPrice: 110, lowPrice: 95, closePrice: 105, volume: 10, quoteVolume: 1_000)
    }

    private func tick(_ price: Double, size: Double = 1, at time: Date, trade: Int64 = 1) -> CoinbaseTick {
        CoinbaseTick(productID: "BTC-USD", price: price, size: size, time: time, tradeID: trade)
    }

    private let hourly = CoinbaseGranularity(interval: "1h")!

    func testTradeInsideTheCandleMovesCloseHighLowAndVolume() {
        let chart = makeChart(candle(at: hour))

        chart.applyTick(tick(120, size: 2, at: hour.addingTimeInterval(600)), plan: hourly)
        chart.applyTick(tick(90, size: 1, at: hour.addingTimeInterval(1_200)), plan: hourly)

        XCTAssertEqual(chart.klineData.count, 1)
        let live = chart.klineData[0]
        XCTAssertEqual(live.openPrice, 100, "The open comes from REST and never moves")
        XCTAssertEqual(live.highPrice, 120)
        XCTAssertEqual(live.lowPrice, 90)
        XCTAssertEqual(live.closePrice, 90)
        XCTAssertEqual(live.volume, 13)
        XCTAssertEqual(live.quoteVolume, 1_000 + 120 * 2 + 90, accuracy: 1e-9)
        XCTAssertEqual(chart.currentPrice, 90)
        XCTAssertFalse(live.isClosed)
    }

    func testTradeInTheNextHourClosesTheCandleAndOpensANewOne() {
        let chart = makeChart(candle(at: hour))

        chart.applyTick(tick(108, size: 0.5, at: hour.addingTimeInterval(3_605)), plan: hourly)

        XCTAssertEqual(chart.klineData.count, 2)
        XCTAssertTrue(chart.klineData[0].isClosed)
        let opened = chart.klineData[1]
        XCTAssertEqual(opened.openTime, hour.addingTimeInterval(3_600))
        XCTAssertEqual(opened.openPrice, 108)
        XCTAssertEqual(opened.highPrice, 108)
        XCTAssertEqual(opened.lowPrice, 108)
        XCTAssertEqual(opened.closePrice, 108)
        XCTAssertEqual(opened.volume, 0.5)
        XCTAssertFalse(opened.isClosed)
    }

    func testTradeFromBeforeTheLastCandleIsIgnored() {
        let chart = makeChart(candle(at: hour))

        chart.applyTick(tick(1, at: hour.addingTimeInterval(-60)), plan: hourly)

        XCTAssertEqual(chart.klineData[0].closePrice, 105)
        XCTAssertEqual(chart.klineData[0].lowPrice, 95)
    }

    func testWeeklyCandleTakesTradesUntilNextMonday() throws {
        let weekly = try XCTUnwrap(CoinbaseGranularity(interval: "1w"))
        let monday = Date(timeIntervalSince1970: 1_789_948_800)  // 2026-09-21 00:00 UTC
        let chart = makeChart(candle(at: monday))

        chart.applyTick(tick(101, at: monday.addingTimeInterval(6 * 86_400 + 3_600)), plan: weekly)
        XCTAssertEqual(chart.klineData.count, 1, "Sunday still belongs to the week")

        chart.applyTick(tick(102, at: monday.addingTimeInterval(7 * 86_400 + 60)), plan: weekly)
        XCTAssertEqual(chart.klineData.count, 2)
        XCTAssertEqual(chart.klineData[1].openTime, monday.addingTimeInterval(7 * 86_400))
    }

    func testTradesBeforeAnyCandlesAreDropped() {
        let chart = ChartViewModel(ticker: "BTC-USD", source: .coinbase, api: EmptySource())

        chart.applyTick(tick(1, at: hour), plan: hourly)

        XCTAssertTrue(chart.klineData.isEmpty)
        XCTAssertNil(chart.currentPrice)
    }

    // MARK: - Symbol handling

    func testCoinbaseChartUsesTheProductIDAndItsBaseAsset() {
        let chart = ChartViewModel(ticker: "eth-eur", source: .coinbase, api: EmptySource())

        XCTAssertEqual(chart.apiSymbol, "ETH-EUR")
        XCTAssertEqual(chart.baseSymbol, "ETH")
        XCTAssertEqual(chart.iconKey, "Coinbase:eth-eur")
    }

    // MARK: - Source registration and search order

    func testCoinbaseSitsSecondInTheSearchFanOut() {
        XCTAssertEqual(
            DataSourceFactory.shared.allSources.map(\.type),
            [.binance, .coinbase, .coingecko, .dexscreener])
        XCTAssertEqual(DataSourceType.cryptoSources, [.binance, .coinbase, .coingecko, .dexscreener])
        XCTAssertTrue(DataSourceFactory.shared.service(for: .coinbase) is CoinbaseAPIService)
    }

    func testSourcesListInPriorityOrderNotAlphabetically() {
        let viewModel = TickerSearchViewModel(sources: {
            [.binance, .coinbase, .coingecko, .dexscreener].map(StubSource.init)
        })

        XCTAssertEqual(viewModel.orderedSources, [.binance, .coinbase, .coingecko, .dexscreener])
    }

    func testSourcesWithResultsFloatAboveEmptyOnesKeepingPriorityOrder() {
        let viewModel = TickerSearchViewModel(sources: {
            [.binance, .coinbase, .coingecko, .dexscreener].map(StubSource.init)
        })
        viewModel.searchResults = [
            .coingecko: [TickerSearchResult(symbol: "BTC", fullSymbol: "bitcoin", source: .coingecko, price: nil)],
            .coinbase: [TickerSearchResult(symbol: "BTC/USD", fullSymbol: "BTC-USD", source: .coinbase, price: nil)],
        ]

        XCTAssertEqual(viewModel.orderedSources, [.coinbase, .coingecko, .binance, .dexscreener])
        XCTAssertEqual(viewModel.firstAvailableResult?.source, .coinbase)
    }

    func testBinanceHitIsTheEnterKeyPickWhenBothExchangesMatch() {
        let viewModel = TickerSearchViewModel(sources: {
            [.binance, .coinbase, .coingecko, .dexscreener].map(StubSource.init)
        })
        viewModel.searchResults = [
            .coinbase: [TickerSearchResult(symbol: "BTC/USD", fullSymbol: "BTC-USD", source: .coinbase, price: nil)],
            .binance: [TickerSearchResult(symbol: "BTC/USDT", fullSymbol: "BTCUSDT", source: .binance, price: nil)],
        ]

        XCTAssertEqual(viewModel.firstAvailableResult?.source, .binance)
    }

    func testUSDRanksWithUSDTAheadOfOtherQuotesForABareTicker() {
        let results = [
            TickerSearchResult(symbol: "BTC/EUR", fullSymbol: "BTC-EUR", source: .coinbase, price: nil),
            TickerSearchResult(symbol: "BTC/USDC", fullSymbol: "BTC-USDC", source: .coinbase, price: nil),
            TickerSearchResult(symbol: "BTC/USD", fullSymbol: "BTC-USD", source: .coinbase, price: nil),
        ].ranked(for: "btc")

        XCTAssertEqual(results.first?.fullSymbol, "BTC-USD")
    }

    func testCoinbaseSourceRoundTripsThroughPersistedConfig() throws {
        let config = TickerConfig(symbol: "BTC-USD", source: .coinbase)
        let decoded = try JSONDecoder().decode(TickerConfig.self, from: JSONEncoder().encode(config))

        XCTAssertEqual(decoded.source, .coinbase)
        XCTAssertEqual(DataSourceType(rawValue: "Coinbase"), .coinbase, "The raw value is persisted — it must not change")
        XCTAssertTrue(DataSourceType.coinbase.providesVolume)
    }

    private final class EmptySource: TickerDataSource {
        let type = DataSourceType.coinbase
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private final class StubSource: TickerDataSource {
        let type: DataSourceType
        init(_ type: DataSourceType) { self.type = type }
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }
}
