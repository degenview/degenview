import Foundation

/// App-wide persisted favorites, shared by every tab/window.
@MainActor
final class FavoritesStore: ObservableObject {
    static let shared = FavoritesStore()

    @Published private(set) var items: [FavoriteItem]
    private let store = JSONStore<[FavoriteItem]>(filename: "favorites.json")

    private init() {
        items = store.load() ?? []
    }

    func add(_ result: TickerSearchResult) throws {
        guard !items.contains(where: {
            $0.config.source == result.source &&
            $0.config.symbol.caseInsensitiveCompare(result.fullSymbol) == .orderedSame
        }) else {
            throw TickerError.duplicate(result.symbol)
        }

        let labels = Self.labels(for: result)
        let displayName: String? = {
            guard result.source == .polymarket else { return nil }
            if let series = result.pmSeries, series.count > 1 {
                return result.eventTitle ?? result.symbol
            }
            return result.symbol
        }()
        let config = TickerConfig(
            symbol: result.fullSymbol,
            source: result.source,
            displayName: displayName,
            pmSeries: result.pmSeries
        )
        items.append(FavoriteItem(name: labels.name, ticker: labels.ticker, config: config))
        store.save(items)
    }

    func remove(_ item: FavoriteItem) {
        items.removeAll { $0.id == item.id }
        store.save(items)
    }

    private static func labels(for result: TickerSearchResult) -> (name: String, ticker: String) {
        if result.source == .polymarket {
            return (result.eventTitle ?? result.question ?? result.symbol, result.symbol)
        }

        if result.source == .alpaca {
            let parts = result.symbol.components(separatedBy: " — ")
            return (parts.count > 1 ? parts.dropFirst().joined(separator: " — ") : result.symbol,
                    result.fullSymbol.uppercased())
        }

        if result.source == .coingecko {
            let name = result.fullSymbol
                .split(separator: "-")
                .map { $0.capitalized }
                .joined(separator: " ")
            return (name, result.symbol.uppercased())
        }

        return (result.symbol, result.symbol.uppercased())
    }
}
