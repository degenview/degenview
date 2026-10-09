import XCTest

@testable import DegenView

/// Which series a script's `request.security` calls read, and where the app fetches them from.
final class PineSecurityFeedTests: XCTestCase {
    private let chart = PineSecurityTarget.Chart(
        tickerID: "Binance:BTC", source: .binance, apiSymbol: "BTCUSDT")

    private func bars(_ count: Int, spacing: TimeInterval = 3_600) -> [KlineData] {
        (0..<count).map {
            let close = Double($0 + 1)
            return KlineData(
                openTime: Date(timeIntervalSince1970: 1_700_000_000 + Double($0) * spacing), openPrice: close,
                highPrice: close + 1, lowPrice: close - 1, closePrice: close, volume: 1)
        }
    }

    private func recorded(_ body: String, inputs: [String: PineInputValue] = [:]) throws -> [PineSecurityKey] {
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let recorder = PineSecurityRecorder()
        _ = try? PineRuntimeSession(
            program: program, inputs: inputs, symbol: PineSymbolInfo(ticker: "BTC", tickerID: chart.tickerID),
            securityData: recorder
        ).evaluate(bars: bars(5))
        return recorder.keys
    }

    // MARK: - Recording

    func testTheRecorderLearnsEachSeriesOnceInTheOrderAsked() throws {
        let keys = try recorded(
            """
            a = request.security("BINANCE:ETHUSDT", "60", close)
            b = request.security("BINANCE:ETHUSDT", "60", open)
            c = request.security(syminfo.tickerid, "D", close)
            plot(a + b + c)
            """)
        XCTAssertEqual(
            keys,
            [
                PineSecurityKey(symbol: "BINANCE:ETHUSDT", interval: 3_600),
                PineSecurityKey(symbol: "Binance:BTC", interval: 86_400),
            ])
    }

    func testALowerTimeframeRequestIsRecordedAtItsOwnLength() throws {
        let keys = try recorded(
            """
            [o, c] = request.security_lower_tf(syminfo.tickerid, "12", [open, close])
            plot(array.size(c))
            """)
        XCTAssertEqual(keys, [PineSecurityKey(symbol: chart.tickerID, interval: 720)])
    }

    func testASymbolChosenByAnInputIsRecordedAsChosen() throws {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("T")
                s = input.symbol("BINANCE:SOLUSDT", "Symbol")
                plot(request.security(s, "60", close))
                """)
        let recorder = PineSecurityRecorder()
        _ = try? PineRuntimeSession(program: program, securityData: recorder).evaluate(bars: bars(3))
        XCTAssertEqual(recorder.keys, [PineSecurityKey(symbol: "BINANCE:SOLUSDT", interval: 3_600)])
    }

    func testAScriptWithoutRequestsRecordsNothingAndIsNotProbed() throws {
        XCTAssertEqual(try recorded("plot(close)"), [])
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\nplot(close)")
        XCTAssertFalse(PineSecurityFeed.usesSecurity(program))
    }

    // MARK: - Resolving a series

    func testTheChartsOwnSymbolResolvesToItsSource() {
        let key = PineSecurityKey(symbol: "Binance:BTC", interval: 86_400)
        XCTAssertEqual(
            PineSecurityTarget.resolve(key, chart: chart),
            PineSecurityTarget(source: .binance, symbol: "BTCUSDT", range: .oneDay))
    }

    func testAnExchangePrefixPicksTheSource() {
        func resolve(_ symbol: String) -> PineSecurityTarget? {
            PineSecurityTarget.resolve(PineSecurityKey(symbol: symbol, interval: 3_600), chart: chart)
        }
        XCTAssertEqual(resolve("BINANCE:ethusdt")?.symbol, "ETHUSDT")
        XCTAssertEqual(resolve("COINBASE:BTCUSD")?.source, .coinbase)
        XCTAssertEqual(resolve("COINBASE:BTCUSD")?.symbol, "BTC-USD")
        XCTAssertEqual(resolve("COINBASE:ETH-EUR")?.symbol, "ETH-EUR")
        XCTAssertEqual(resolve("NASDAQ:AAPL"), PineSecurityTarget(source: .alpaca, symbol: "AAPL", range: .oneHour))
        XCTAssertEqual(resolve("ETHUSDT")?.source, .binance, "a bare name stays on the chart's source")
    }

    func testWhatTheAppCannotFetchIsNotResolved() {
        func resolve(_ symbol: String, _ seconds: TimeInterval = 3_600) -> PineSecurityTarget? {
            PineSecurityTarget.resolve(PineSecurityKey(symbol: symbol, interval: seconds), chart: chart)
        }
        XCTAssertNil(resolve("NSE:NIFTY"), "no source for the exchange")
        XCTAssertNil(resolve("BINANCE:BTCUSDT", 14_400), "no 4-hour candles")
        XCTAssertNil(resolve("BINANCE:BTCUSDT", 60), "no 1-minute candles")
        XCTAssertNil(resolve("BINANCE:"), "an empty name")
        let gecko = PineSecurityTarget.Chart(tickerID: "CoinGecko:bitcoin", source: .coingecko, apiSymbol: "bitcoin")
        XCTAssertNil(
            PineSecurityTarget.resolve(PineSecurityKey(symbol: "ETHUSDT", interval: 3_600), chart: gecko),
            "a bare name is only meaningful on a source whose symbols are names")
    }

    func testPineTimeframesMapOntoTheAppsCandleSizes() {
        XCTAssertEqual(PineSecurityTarget.range(forSeconds: 3_600), .oneHour)
        XCTAssertEqual(PineSecurityTarget.range(forSeconds: PineTime.seconds(ofTimeframe: "W") ?? 0), .oneWeek)
        XCTAssertEqual(PineSecurityTarget.range(forSeconds: PineTime.seconds(ofTimeframe: "M") ?? 0), .oneMonth)
        XCTAssertEqual(PineSecurityTarget.range(forSeconds: PineTime.seconds(ofTimeframe: "3M") ?? 0), .threeMonths)
        XCTAssertEqual(PineSecurityTarget.range(forSeconds: PineTime.seconds(ofTimeframe: "12M") ?? 0), .oneYear)
    }
}
