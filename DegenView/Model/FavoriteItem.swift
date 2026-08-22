import Foundation

/// One app-wide saved market shortcut.
struct FavoriteItem: Identifiable, Codable, Equatable {
    let id: UUID
    let name: String
    let ticker: String
    let config: TickerConfig

    init(id: UUID = UUID(), name: String, ticker: String, config: TickerConfig) {
        self.id = id
        self.name = name
        self.ticker = ticker
        self.config = config
    }
}
