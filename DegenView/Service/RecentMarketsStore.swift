import Foundation

/// The last markets the user picked in the Add Chart sheet, newest first, shared by every tab.
///
/// Lives in the `setting` table of the app database like other small pieces of user data.
/// Prediction markets are not kept: a multi-choice market is several series, which a single
/// remembered result can't reproduce.
@MainActor
final class RecentMarketsStore: ObservableObject {
    static let shared = RecentMarketsStore()
    static let limit = 10
    static let settingKey = "addTicker.recentMarkets"

    @Published private(set) var items: [RecentMarket]
    private let database: AppDatabase

    init(database: AppDatabase = .shared) {
        self.database = database
        items = database.setting([RecentMarket].self, key: Self.settingKey) ?? []
    }

    /// Moves `result` to the front, once, and drops whatever falls past the limit.
    func record(_ result: TickerSearchResult) {
        guard !result.source.isPredictionMarket else { return }
        let market = RecentMarket(result)
        items = Array(([market] + items.filter { $0.id != market.id }).prefix(Self.limit))
        save()
    }

    func remove(_ market: RecentMarket) {
        items.removeAll { $0.id == market.id }
        save()
    }

    func clear() {
        items = []
        save()
    }

    private func save() {
        database.setSetting(items.isEmpty ? nil : items, key: Self.settingKey)
    }
}
