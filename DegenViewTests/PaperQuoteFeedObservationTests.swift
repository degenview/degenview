import AppKit
import SwiftUI
import XCTest

@testable import DegenView

/// The feed must follow a chart on its own. The first version watched the chart from `ContentView`,
/// which does not observe its charts, so it only ran when something unrelated redrew it; these tests
/// mount the feed alone and change the chart, with nothing else to prompt a redraw.
@MainActor
final class PaperQuoteFeedObservationTests: XCTestCase {
    private final class EmptySource: TickerDataSource {
        let type = DataSourceType.coinbase
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private var window: NSWindow?

    override func tearDown() async throws {
        window?.orderOut(nil)
        window = nil
    }

    private func makeStore() throws -> PaperTradingStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return PaperTradingStore(
            database: try AppDatabase(path: directory.appendingPathComponent("p.sqlite").path),
            quoteFlushInterval: 0.02)
    }

    private func mount(_ chart: ChartViewModel, store: PaperTradingStore) {
        let host = NSHostingView(
            rootView: PaperQuoteFeed(viewModel: chart, quote: chart.liveQuote, store: store))
        host.frame = CGRect(x: 0, y: 0, width: 10, height: 10)
        let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        window.setFrameOrigin(NSPoint(x: -20000, y: -20000))
        window.orderBack(nil)
        self.window = window
    }

    private func settle() async throws {
        try await Task.sleep(nanoseconds: 400_000_000)
    }

    private var key: String {
        PaperInstrument.chart(symbol: "BTC-USD", displayName: "BTC-USD", source: .coinbase).key
    }

    func testFeedFollowsTheChartPriceBidAndAskOnItsOwn() async throws {
        let store = try makeStore()
        await store.connect()
        let chart = ChartViewModel(ticker: "BTC-USD", source: .coinbase, api: EmptySource())
        mount(chart, store: store)
        try await settle()
        XCTAssertNil(store.snapshot.quotes[key], "a chart with no price yet feeds nothing")

        chart.currentPrice = 100
        try await settle()
        XCTAssertEqual(store.snapshot.quotes[key]?.last, 100)

        chart.currentPrice = 105
        try await settle()
        XCTAssertEqual(store.snapshot.quotes[key]?.last, 105)

        chart.liveQuote.apply(bid: 104.5, ask: 105.5)
        try await settle()
        XCTAssertEqual(store.snapshot.quotes[key]?.bid, Decimal(string: "104.5"))
        XCTAssertEqual(store.snapshot.quotes[key]?.ask, Decimal(string: "105.5"))
    }

    func testARefreshWithAnUnchangedPriceStillRefreshesTheQuote() async throws {
        let store = try makeStore()
        await store.connect()
        let chart = ChartViewModel(ticker: "BTC-USD", source: .coinbase, api: EmptySource())
        chart.currentPrice = 100
        mount(chart, store: store)
        try await settle()
        let first = try XCTUnwrap(store.snapshot.quotes[key]?.timestamp)

        chart.lastUpdated = Date()
        try await settle()
        let second = try XCTUnwrap(store.snapshot.quotes[key]?.timestamp)
        XCTAssertGreaterThan(second, first, "the engine's quote must not age while the price sits still")
    }

    func testNothingIsFedUntilPaperTradingIsConnected() async throws {
        let store = try makeStore()
        let chart = ChartViewModel(ticker: "BTC-USD", source: .coinbase, api: EmptySource())
        chart.currentPrice = 100
        mount(chart, store: store)
        try await settle()
        XCTAssertNil(store.snapshot.quotes[key])

        await store.connect()
        try await settle()
        XCTAssertEqual(store.snapshot.quotes[key]?.last, 100, "connecting feeds the price already on the chart")
    }
}
