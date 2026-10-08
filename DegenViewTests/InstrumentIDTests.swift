import XCTest

@testable import DegenView

final class InstrumentIDTests: XCTestCase {
    func testTheSameTickerOnTwoProvidersIsTwoInstruments() {
        let binance = InstrumentID(source: .binance, symbol: "BTCUSDT")
        let coinbase = InstrumentID(source: .coinbase, symbol: "BTC-USD")
        let coinbaseSameText = InstrumentID(source: .coinbase, symbol: "BTCUSDT")

        XCTAssertNotEqual(binance, coinbase)
        XCTAssertNotEqual(binance, coinbaseSameText)
        XCTAssertNotEqual(binance.key, coinbaseSameText.key)
        XCTAssertEqual(Set([binance, coinbase, coinbaseSameText]).count, 3)
    }

    func testExchangeSymbolsIgnoreCaseButOpaqueIdsDoNot() {
        XCTAssertEqual(
            InstrumentID(source: .binance, symbol: "btcusdt"), InstrumentID(source: .binance, symbol: "BTCUSDT"))
        XCTAssertEqual(
            InstrumentID(source: .coingecko, symbol: "Bitcoin"), InstrumentID(source: .coingecko, symbol: "bitcoin"))
        XCTAssertNotEqual(
            InstrumentID(source: .dexscreener, symbol: "AbC"), InstrumentID(source: .dexscreener, symbol: "abc"))
    }

    func testDexChainIsMetadataNotIdentity() {
        let withChain = InstrumentID(source: .dexscreener, symbol: "0xPAIR", chain: "base")
        let without = InstrumentID(source: .dexscreener, symbol: "0xPAIR")

        XCTAssertEqual(withChain, without)
        XCTAssertEqual(withChain.chain, "base")
        XCTAssertNil(without.chain)
    }

    func testPredictionMarketIdentityIsTheMarketIdNotTheTitle() {
        let token = String(repeating: "7", count: 77)
        let search = TickerSearchResult(
            symbol: "Yes", fullSymbol: token, source: .polymarket, price: nil,
            metadata: ["eventTitle": "Fed cuts in 2026?"])
        let other = TickerSearchResult(
            symbol: "Yes", fullSymbol: token + "8", source: .polymarket, price: nil,
            metadata: ["eventTitle": "Fed cuts in 2026?"])

        XCTAssertEqual(InstrumentID(searchResult: search).symbol, token)
        XCTAssertNotEqual(InstrumentID(searchResult: search), InstrumentID(searchResult: other))
        XCTAssertEqual(
            InstrumentID(source: .kalshi, symbol: "KXFED/KXFED-26DEC").symbol, "KXFED/KXFED-26DEC")
    }

    func testSearchResultKeepsTheDexChain() {
        let result = TickerSearchResult(
            symbol: "BONK/SOL", fullSymbol: "PAIRADDR", source: .dexscreener, price: nil,
            metadata: ["chain": "solana"])
        XCTAssertEqual(InstrumentID(searchResult: result).chain, "solana")
    }

    func testOnlyMarketChartsHaveAnInstrument() {
        XCTAssertEqual(
            InstrumentID(config: TickerConfig(symbol: "BTCUSDT", source: .binance)),
            InstrumentID(source: .binance, symbol: "BTCUSDT"))
        XCTAssertNil(
            InstrumentID(
                config: TickerConfig(
                    symbol: "cmc", source: .coinMarketCap,
                    coinMarketCapChart: CoinMarketCapChartConfig(type: .fearAndGreedLatest))))
        XCTAssertNil(
            InstrumentID(
                config: TickerConfig(
                    symbol: "pf", source: .binance, portfolioChart: PortfolioChartConfig(kind: .value))))
    }

    func testQualifiedSymbolAndSlugRoundTrip() {
        XCTAssertEqual(InstrumentID(source: .binance, symbol: "BTCUSDT").qualifiedSymbol, "BINANCE:BTCUSDT")
        for source in DataSourceType.allCases where source.isWatchlistInstrumentSource {
            XCTAssertEqual(DataSourceType(watchlistSlug: source.watchlistSlug), source)
        }
        XCTAssertNil(DataSourceType(watchlistSlug: "CMC"))
        XCTAssertEqual(DataSourceType(watchlistSlug: "binance"), .binance)
    }

    func testCodableRoundTripKeepsChain() throws {
        let id = InstrumentID(source: .dexscreener, symbol: "PAIR", chain: "solana")
        let decoded = try JSONDecoder().decode(InstrumentID.self, from: JSONEncoder().encode(id))
        XCTAssertEqual(decoded.chain, "solana")
        XCTAssertEqual(decoded, id)
    }
}
