import Foundation

/// A column the sidebar can show next to the symbol, which is always present.
enum WatchlistColumn: String, Codable, CaseIterable, Identifiable, Sendable {
    case last
    case change
    case changePercent
    case volume
    case exchange
    case status

    var id: String { rawValue }

    var title: String {
        switch self {
        case .last: return "Last"
        case .change: return "Chg"
        case .changePercent: return "Chg%"
        case .volume: return "Vol"
        case .exchange: return "Exchange"
        case .status: return "Status"
        }
    }

    /// Spoken name, since the header is abbreviated.
    var accessibilityTitle: String {
        switch self {
        case .last: return "Last price"
        case .change: return "Change"
        case .changePercent: return "Change percent"
        case .volume: return "Volume"
        case .exchange: return "Exchange"
        case .status: return "Market status"
        }
    }
}
