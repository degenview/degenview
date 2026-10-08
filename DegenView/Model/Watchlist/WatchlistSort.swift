import Foundation

/// Which value orders the instruments inside each section. `manual` is the stored order.
enum WatchlistSortKey: String, Codable, CaseIterable, Identifiable, Sendable {
    case manual
    case symbol
    case last
    case change
    case changePercent
    case volume

    var id: String { rawValue }

    var title: String {
        switch self {
        case .manual: return "Manual"
        case .symbol: return "Symbol"
        case .last: return "Last"
        case .change: return "Change"
        case .changePercent: return "Change %"
        case .volume: return "Volume"
        }
    }
}

struct WatchlistSort: Codable, Equatable, Sendable {
    var key: WatchlistSortKey = .manual
    var ascending = true

    static let manual = WatchlistSort()
}
