import Foundation

/// The five sections of the Paper Trading panel.
enum PaperManagerTab: String, CaseIterable, Identifiable {
    case positions = "Positions"
    case orders = "Orders"
    case history = "History"
    case accountHistory = "Account History"
    case journal = "Trading Journal"

    var id: String { rawValue }

    /// The short label on the tab bar; the raw value stays the long, stable name.
    var title: String {
        switch self {
        case .positions: "Positions"
        case .orders: "Orders"
        case .history: "History"
        case .accountHistory: "Trades"
        case .journal: "Journal"
        }
    }

    var systemImage: String {
        switch self {
        case .positions: "chart.line.uptrend.xyaxis"
        case .orders: "list.bullet.rectangle"
        case .history: "clock"
        case .accountHistory: "checkmark.seal"
        case .journal: "book.closed"
        }
    }
}
