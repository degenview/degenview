import Foundation

/// A colour mark on a market. Flags belong to the market, not to a list, so a flagged
/// symbol keeps its colour in every watchlist that holds it.
enum WatchlistFlag: String, Codable, CaseIterable, Identifiable, Sendable {
    case red
    case orange
    case yellow
    case green
    case blue
    case purple

    var id: String { rawValue }
    var title: String { rawValue.capitalized }
}
