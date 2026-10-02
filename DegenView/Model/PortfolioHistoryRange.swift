import Foundation

/// How far back the Overview's value chart looks.
enum PortfolioHistoryRange: String, CaseIterable, Identifiable {
    case oneDay = "1D"
    case oneWeek = "1W"
    case oneMonth = "1M"
    case oneYear = "1Y"
    case all = "ALL"

    var id: String { rawValue }

    var duration: TimeInterval? {
        switch self {
        case .oneDay: 86_400
        case .oneWeek: 7 * 86_400
        case .oneMonth: 31 * 86_400
        case .oneYear: 365 * 86_400
        case .all: nil
        }
    }

    /// The span in words, for captions beside a change figure.
    var periodName: String {
        switch self {
        case .oneDay: "24h"
        case .oneWeek: "7 days"
        case .oneMonth: "30 days"
        case .oneYear: "1 year"
        case .all: "all time"
        }
    }

    func filter(_ snapshots: [PortfolioSnapshot], now: Date = Date()) -> [PortfolioSnapshot] {
        guard let duration else { return snapshots }
        let cutoff = now.addingTimeInterval(-duration)
        return snapshots.filter { $0.timestamp >= cutoff }
    }
}
