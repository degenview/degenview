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

    func contains(source: DataSourceType, symbol: String) -> Bool {
        items.contains {
            $0.config.source == source && $0.config.symbol.caseInsensitiveCompare(symbol) == .orderedSame
        }
    }

    /// Add or remove one provider-qualified instrument while preserving sidebar order.
    func toggle(config: TickerConfig, name: String, ticker: String) {
        if let index = items.firstIndex(where: {
            $0.config.source == config.source && $0.config.symbol.caseInsensitiveCompare(config.symbol) == .orderedSame
        }) {
            items.remove(at: index)
        } else {
            items.append(FavoriteItem(name: name, ticker: ticker, config: config))
        }
        store.save(items)
    }

    func add(_ result: TickerSearchResult) throws {
        guard !contains(source: result.source, symbol: result.fullSymbol) else {
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

    /// Reorder favorites using SwiftUI `List` move coordinates, then persist the
    /// resulting array so every tab and the next launch see the same arrangement.
    func move(fromOffsets offsets: IndexSet, toOffset destination: Int) {
        guard !offsets.isEmpty else { return }

        let moving = offsets.sorted().map { items[$0] }
        let removedBeforeDestination = offsets.count(in: 0..<destination)
        let offsetSet = Set(offsets)
        var reordered = items.enumerated().compactMap { index, item in
            offsetSet.contains(index) ? nil : item
        }
        let insertionIndex = max(0, min(destination - removedBeforeDestination, reordered.count))
        reordered.insert(contentsOf: moving, at: insertionIndex)

        guard reordered != items else { return }
        items = reordered
        store.save(items)
    }

    /// Move one dragged favorite to the position occupied by another row. SwiftUI's
    /// macOS sidebar lists don't activate `onMove` outside edit mode, so the explicit
    /// drop delegate uses stable IDs and funnels the mutation through here.
    func move(_ draggedID: UUID, to targetID: UUID) {
        guard let source = items.firstIndex(where: { $0.id == draggedID }),
            let target = items.firstIndex(where: { $0.id == targetID }),
            source != target
        else { return }

        move(
            fromOffsets: IndexSet(integer: source),
            toOffset: target > source ? target + 1 : target
        )
    }

    private static func labels(for result: TickerSearchResult) -> (name: String, ticker: String) {
        if result.source == .polymarket {
            return (result.eventTitle ?? result.question ?? result.symbol, result.symbol)
        }

        if result.source == .alpaca {
            let parts = result.symbol.components(separatedBy: " — ")
            return (
                parts.count > 1 ? parts.dropFirst().joined(separator: " — ") : result.symbol,
                result.fullSymbol.uppercased()
            )
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
