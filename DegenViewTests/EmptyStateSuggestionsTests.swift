import XCTest

@testable import DegenView

final class EmptyStateSuggestionsTests: XCTestCase {
    private func favorite(_ symbol: String, _ source: DataSourceType = .binance) -> FavoriteItem {
        FavoriteItem(name: symbol, ticker: symbol, config: TickerConfig(symbol: symbol, source: source))
    }

    private func recent(_ symbol: String, _ source: DataSourceType = .binance) -> RecentMarket {
        RecentMarket(
            TickerSearchResult(symbol: symbol, fullSymbol: symbol, source: source, price: nil, metadata: [:]))
    }

    private func savedView(_ name: String, daysAgo: Double = 0, configs: [TickerConfig] = []) -> SavedView {
        SavedView(
            name: name, tickers: configs.map(\.symbol), timeRange: .oneDay,
            createdAt: Date(timeIntervalSince1970: 1_000_000 - daysAgo * 86_400),
            tickerConfigs: configs, candleCount: 100)
    }

    func testNothingToOfferIsEmpty() {
        let suggestions = EmptyStateSuggestions(favorites: [], recents: [], savedViews: [])

        XCTAssertFalse(suggestions.hasAny)
    }

    func testAnySectionCountsAsContent() {
        XCTAssertTrue(EmptyStateSuggestions(favorites: [favorite("BTCUSDT")], recents: [], savedViews: []).hasAny)
        XCTAssertTrue(EmptyStateSuggestions(favorites: [], recents: [recent("BTCUSDT")], savedViews: []).hasAny)
        XCTAssertTrue(EmptyStateSuggestions(favorites: [], recents: [], savedViews: [savedView("A")]).hasAny)
    }

    func testRecentsLeaveOutMarketsThatAreFavorites() {
        let suggestions = EmptyStateSuggestions(
            favorites: [favorite("BTCUSDT")],
            recents: [recent("btcusdt"), recent("ETHUSDT")],
            savedViews: [])

        XCTAssertEqual(suggestions.recents.map(\.symbol), ["ETHUSDT"])
    }

    func testTheSameSymbolOnAnotherSourceIsNotADuplicate() {
        let suggestions = EmptyStateSuggestions(
            favorites: [favorite("BTC-USD", .coinbase)],
            recents: [recent("BTC-USD", .coingecko)],
            savedViews: [])

        XCTAssertEqual(suggestions.recents.count, 1)
    }

    func testFavoritesAreCappedInSidebarOrderAndCountWhatIsHidden() {
        let all = (0..<(EmptyStateSuggestions.itemLimit + 3)).map { favorite("F\($0)") }

        let suggestions = EmptyStateSuggestions(favorites: all, recents: [], savedViews: [])

        XCTAssertEqual(suggestions.favorites.map(\.ticker), all.prefix(EmptyStateSuggestions.itemLimit).map(\.ticker))
        XCTAssertEqual(suggestions.hiddenFavoriteCount, 3)
    }

    func testRecentsAreCappedAfterDeduplicating() {
        let favorites = [favorite("R0")]
        let recents = (0..<(EmptyStateSuggestions.itemLimit + 2)).map { recent("R\($0)") }

        let suggestions = EmptyStateSuggestions(favorites: favorites, recents: recents, savedViews: [])

        XCTAssertEqual(suggestions.recents.count, EmptyStateSuggestions.itemLimit)
        XCTAssertEqual(suggestions.recents.first?.symbol, "R1")
    }

    func testSavedViewsAreNewestFirst() {
        let suggestions = EmptyStateSuggestions(
            favorites: [], recents: [],
            savedViews: [savedView("Old", daysAgo: 5), savedView("New", daysAgo: 0), savedView("Mid", daysAgo: 2)])

        XCTAssertEqual(suggestions.savedViews.map(\.name), ["New", "Mid", "Old"])
    }

    func testPreviewSkipsNonMarketChartsAndStopsAtThree() {
        var portfolio = TickerConfig(symbol: "portfolio-1", source: .binance)
        portfolio.portfolioChart = PortfolioChartConfig(portfolioID: UUID(), kind: .value)
        let view = savedView(
            "Mixed",
            configs: [
                portfolio,
                TickerConfig(symbol: "A", source: .binance),
                TickerConfig(symbol: "B", source: .binance),
                TickerConfig(symbol: "C", source: .binance),
                TickerConfig(symbol: "D", source: .binance),
            ])

        XCTAssertEqual(EmptyStateSuggestions.previewConfigs(of: view).map(\.symbol), ["A", "B", "C"])
    }
}
