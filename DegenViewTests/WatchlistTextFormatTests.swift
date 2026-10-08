import XCTest

@testable import DegenView

final class WatchlistTextFormatTests: XCTestCase {
    private func parsedInstruments(_ text: String) -> [InstrumentID] {
        WatchlistTextFormat.parse(text).rows.compactMap {
            if case .instrument(let item) = $0 { return item.instrument }
            return nil
        }
    }

    func testExportListsSectionsAndQualifiedSymbols() throws {
        var list = Watchlist(name: "Crypto")
        let majors = try list.addSection(title: "Majors")
        try list.add(
            WatchlistInstrument(instrument: InstrumentID(source: .binance, symbol: "BTCUSDT"), name: "BTC", label: "BTC"),
            toSection: majors.id)
        try list.add(
            WatchlistInstrument(
                instrument: InstrumentID(source: .dexscreener, symbol: "PAIR", chain: "solana"), name: "X", label: "X"))

        XCTAssertEqual(WatchlistTextFormat.export(list), "###Majors\nBINANCE:BTCUSDT\nDEXSCREENER:solana/PAIR")
    }

    func testRoundTripKeepsOrderSectionsChainAndPredictionTitles() throws {
        var list = Watchlist(name: "Mixed")
        try list.add(
            WatchlistInstrument(instrument: InstrumentID(source: .coinbase, symbol: "ETH-USD"), name: "ETH", label: "ETH"))
        try list.addSection(title: "Bets")
        try list.add(
            WatchlistInstrument(
                instrument: InstrumentID(source: .polymarket, symbol: "123456"), name: "Will it rain, today?",
                label: "Yes", displayName: "Will it rain, today?"))
        try list.add(
            WatchlistInstrument(
                instrument: InstrumentID(source: .dexscreener, symbol: "PAIR", chain: "base"), name: "P", label: "P"))

        let parsed = WatchlistTextFormat.parse(WatchlistTextFormat.export(list))

        XCTAssertTrue(parsed.skipped.isEmpty)
        XCTAssertEqual(parsed.rows.count, 4)
        guard case .instrument(let market) = parsed.rows[2], case .instrument(let dex) = parsed.rows[3] else {
            return XCTFail("unexpected rows \(parsed.rows)")
        }
        XCTAssertEqual(market.displayName, "Will it rain, today?")
        XCTAssertEqual(dex.instrument.chain, "base")
        XCTAssertEqual(parsedInstruments(WatchlistTextFormat.export(list)), list.instruments.map(\.instrument))
    }

    func testTradingViewStyleCommaSeparatedLine() {
        let parsed = WatchlistTextFormat.parse("###Core,BINANCE:BTCUSDT,NASDAQ:AAPL,COINBASE:ETH-USD")

        XCTAssertEqual(parsed.rows.count, 4)
        XCTAssertEqual(
            parsedInstruments("BINANCE:BTCUSDT,NASDAQ:AAPL,COINBASE:ETH-USD"),
            [
                InstrumentID(source: .binance, symbol: "BTCUSDT"), InstrumentID(source: .alpaca, symbol: "AAPL"),
                InstrumentID(source: .coinbase, symbol: "ETH-USD"),
            ])
    }

    func testUnsupportedAndAmbiguousRowsAreReportedNotGuessed() {
        let text = "BINANCE:BTCUSDT.P\nKRAKEN:XBTUSD\nCOINBASE:BTCUSD\nBTCUSDT\nPOLYMARKET:123\nCMC:fear\nBINANCE:ETHUSDT"
        let parsed = WatchlistTextFormat.parse(text)

        XCTAssertEqual(parsedInstruments(text), [InstrumentID(source: .binance, symbol: "ETHUSDT")])
        XCTAssertEqual(parsed.skipped.map(\.line), [1, 2, 3, 4, 5, 6])
        XCTAssertTrue(parsed.skipped[1].reason.contains("KRAKEN"))
    }

    func testBlankLinesAndSpacingAreIgnored() {
        XCTAssertEqual(
            parsedInstruments("\n  binance:btcusdt  \n\n"), [InstrumentID(source: .binance, symbol: "BTCUSDT")])
    }
}
