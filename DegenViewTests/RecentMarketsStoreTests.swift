import XCTest

@testable import DegenView

@MainActor
final class RecentMarketsStoreTests: XCTestCase {
    private func result(
        _ symbol: String, _ full: String? = nil, _ source: DataSourceType = .binance,
        metadata: [String: String] = [:]
    ) -> TickerSearchResult {
        TickerSearchResult(
            symbol: symbol, fullSymbol: full ?? symbol, source: source, price: 123, metadata: metadata)
    }

    private func makeStore(_ database: AppDatabase? = nil) throws -> (RecentMarketsStore, AppDatabase) {
        let database = try database ?? .makeInMemory()
        return (RecentMarketsStore(database: database), database)
    }

    func testNewestFirstWithoutDuplicates() throws {
        let (store, _) = try makeStore()

        store.record(result("BTC/USDT", "BTCUSDT"))
        store.record(result("ETH/USDT", "ETHUSDT"))
        store.record(result("BTC/USDT", "BTCUSDT"))

        XCTAssertEqual(store.items.map(\.symbol), ["BTC/USDT", "ETH/USDT"])
    }

    func testTheSameSymbolOnAnotherSourceIsAnotherMarket() throws {
        let (store, _) = try makeStore()

        store.record(result("BTC/USDT", "BTCUSDT", .binance))
        store.record(result("BTC/USD", "BTC-USD", .coinbase))
        store.record(result("BTC/USDT", "BTCUSDT", .coinbase))

        XCTAssertEqual(store.items.count, 3)
    }

    func testKeepsOnlyTheLatestTen() throws {
        let (store, _) = try makeStore()

        for index in 0..<(RecentMarketsStore.limit + 3) { store.record(result("T\(index)")) }

        XCTAssertEqual(store.items.count, RecentMarketsStore.limit)
        XCTAssertEqual(store.items.first?.symbol, "T12")
        XCTAssertEqual(store.items.last?.symbol, "T3")
    }

    func testPredictionMarketsAreNotKept() throws {
        let (store, _) = try makeStore()

        store.record(result("Will it rain?", "token", .polymarket))
        store.record(result("Fed cut", "SERIES/MKT", .kalshi))

        XCTAssertTrue(store.items.isEmpty)
    }

    func testPersistsAcrossInstances() throws {
        let (store, database) = try makeStore()
        store.record(result("BTC/USDT", "BTCUSDT"))
        store.record(result("AAPL", "AAPL", .alpaca))

        let (reopened, _) = try makeStore(database)
        XCTAssertEqual(reopened.items.map(\.symbol), ["AAPL", "BTC/USDT"])

        reopened.remove(try XCTUnwrap(reopened.items.first))
        XCTAssertEqual(try makeStore(database).0.items.map(\.symbol), ["BTC/USDT"])

        reopened.clear()
        XCTAssertTrue(try makeStore(database).0.items.isEmpty)
    }

    func testRoundTripKeepsWhatSelectingNeedsAndDropsThePrice() throws {
        let dex = result("WBTC/SOL", "pairAddress", .dexscreener, metadata: ["chain": "solana", "dex": "raydium"])

        let restored = RecentMarket(dex).result

        XCTAssertEqual(restored, dex)
        XCTAssertEqual(restored.symbol, "WBTC/SOL")
        XCTAssertEqual(restored.chain, "solana")
        XCTAssertEqual(restored.dex, "raydium")
        XCTAssertNil(restored.price)
    }
}
