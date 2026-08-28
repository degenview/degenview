import XCTest

@testable import DegenView

final class KlineDataTests: XCTestCase {
    func testBinanceParserAcceptsMixedNumericTypesAndQuoteVolume() {
        let raw: [Any] = [Int64(1_700_000_000_000), "10", 13.0, NSNumber(value: 8), "12", "4", 0, "48"]
        let candle = KlineData(raw: raw)

        XCTAssertEqual(candle?.openPrice, 10)
        XCTAssertEqual(candle?.highPrice, 13)
        XCTAssertEqual(candle?.lowPrice, 8)
        XCTAssertEqual(candle?.closePrice, 12)
        XCTAssertEqual(candle?.volume, 4)
        XCTAssertEqual(candle?.quoteVolume, 48)
        XCTAssertNil(KlineData(raw: [1, "invalid", 3, 4, 5, 6]))
    }

    func testPriceSamplesProduceAlignedContinuousCandles() {
        let base = Date(timeIntervalSince1970: 3_600)
        let samples = [
            (base.addingTimeInterval(10), 10.0),
            (base.addingTimeInterval(20), 12.0),
            (base.addingTimeInterval(3_610), 9.0),
        ]
        let candles = KlineData.candles(from: samples, interval: 3_600)

        XCTAssertEqual(candles.count, 2)
        XCTAssertEqual(candles[0].openTime, base)
        XCTAssertEqual(candles[0].highPrice, 12)
        XCTAssertEqual(candles[1].openPrice, candles[0].closePrice)
        XCTAssertEqual(candles[1].lowPrice, 9)
    }

    func testAggregationPreservesOHLCAndVolumes() {
        let values = [
            KlineData(
                openTime: .distantPast, openPrice: 10, highPrice: 15, lowPrice: 8, closePrice: 12, volume: 2,
                quoteVolume: 20),
            KlineData(
                openTime: .now, openPrice: 12, highPrice: 14, lowPrice: 7, closePrice: 9, volume: 3, quoteVolume: 30),
        ]
        let merged = values.aggregated(into: 1)

        XCTAssertEqual(merged.count, 1)
        XCTAssertEqual(merged[0].openPrice, 10)
        XCTAssertEqual(merged[0].highPrice, 15)
        XCTAssertEqual(merged[0].lowPrice, 7)
        XCTAssertEqual(merged[0].closePrice, 9)
        XCTAssertEqual(merged[0].volume, 5)
        XCTAssertEqual(merged[0].quoteVolume, 50)
    }

    func testDownsamplingKeepsBothEndpoints() {
        let values = (0..<10).map { KlineData(time: Date(timeIntervalSince1970: Double($0)), price: Double($0)) }
        let result = values.downsampled(to: 4)
        XCTAssertEqual(result.count, 4)
        XCTAssertEqual(result.first?.closePrice, 0)
        XCTAssertEqual(result.last?.closePrice, 9)
    }
}

final class TickerSearchRelevanceTests: XCTestCase {
    func testExactCryptoBasePrefersUSDTThenOtherQuotes() {
        let results = [
            result("WBTC/BTC", "WBTCBTC"),
            result("BTC/EUR", "BTCEUR"),
            result("BTC/USDC", "BTCUSDC"),
            result("BTC/USDT", "BTCUSDT"),
        ].ranked(for: "BTC")

        XCTAssertEqual(results.map(\.symbol), ["BTC/USDT", "BTC/EUR", "BTC/USDC", "WBTC/BTC"])
    }

    func testExactStockTickerPrecedesSymbolAndCompanyNameMatches() {
        let results = [
            result("PAAPL — Sample Holdings", "PAAPL", source: .alpaca),
            result("XYZ — AAPL Technologies", "XYZ", source: .alpaca),
            result("AAPL — Apple Inc.", "AAPL", source: .alpaca),
        ].ranked(for: "AAPL")

        XCTAssertEqual(results.first?.fullSymbol, "AAPL")
        XCTAssertEqual(results.map(\.fullSymbol), ["AAPL", "PAAPL", "XYZ"])
    }

    func testExactDisplayedAndFullSymbolQueriesRankFirst() {
        let results = [
            result("BTC/USDC", "BTCUSDC"),
            result("BTC/USDT", "BTCUSDT"),
        ]

        XCTAssertEqual(results.ranked(for: "BTC/USDT").first?.fullSymbol, "BTCUSDT")
        XCTAssertEqual(results.ranked(for: "BTCUSDC").first?.fullSymbol, "BTCUSDC")
    }

    func testEqualRanksPreserveProviderOrder() {
        let results = [
            result("BTC/EUR", "BTCEUR"),
            result("BTC/USDC", "BTCUSDC"),
            result("BTC/FDUSD", "BTCFDUSD"),
        ]

        XCTAssertEqual(results.ranked(for: "BTC").map(\.fullSymbol), results.map(\.fullSymbol))
    }

    private func result(
        _ symbol: String, _ fullSymbol: String, source: DataSourceType = .binance
    ) -> TickerSearchResult {
        TickerSearchResult(symbol: symbol, fullSymbol: fullSymbol, source: source, price: nil)
    }
}
