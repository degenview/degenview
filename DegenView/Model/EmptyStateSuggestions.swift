import Foundation

/// What an empty tab offers instead of a blank page: favorites, recently added markets and
/// saved views, trimmed to fit and without the same market appearing twice.
struct EmptyStateSuggestions: Equatable {
    /// Chips shown per section; the rest stays reachable from the favorites sidebar or Add Chart.
    static let itemLimit = 8
    /// Ticker icons drawn on a saved-view card.
    static let previewLimit = 3

    /// Sidebar order, capped.
    let favorites: [FavoriteItem]
    /// Favorites that did not fit.
    let hiddenFavoriteCount: Int
    /// Newest first, capped, without markets that are already favorites.
    let recents: [RecentMarket]
    /// Newest first.
    let savedViews: [SavedView]

    var hasAny: Bool { !favorites.isEmpty || !recents.isEmpty || !savedViews.isEmpty }

    init(favorites: [FavoriteItem], recents: [RecentMarket], savedViews: [SavedView]) {
        self.favorites = Array(favorites.prefix(Self.itemLimit))
        hiddenFavoriteCount = max(0, favorites.count - Self.itemLimit)

        let favoriteKeys = Set(favorites.map { Self.key(source: $0.config.source, symbol: $0.config.symbol) })
        self.recents = Array(
            recents.filter { !favoriteKeys.contains(Self.key(source: $0.source, symbol: $0.fullSymbol)) }
                .prefix(Self.itemLimit))

        self.savedViews = savedViews.sorted { $0.createdAt > $1.createdAt }
    }

    /// The market charts of a saved view, in order. Portfolio, CoinMarketCap and power-law
    /// slots carry sentinel symbols that mean nothing as a ticker.
    static func previewConfigs(of view: SavedView) -> [TickerConfig] {
        let markets = view.tickerConfigs.filter {
            $0.portfolioChart == nil && $0.coinMarketCapChart == nil && $0.bitcoinPowerLaw == nil
        }
        return Array(markets.prefix(previewLimit))
    }

    private static func key(source: DataSourceType, symbol: String) -> String {
        "\(source.rawValue):\(symbol.uppercased())"
    }
}
