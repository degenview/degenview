import Foundation

struct SavedView: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String
    var tickers: [String]
    var timeRange: TimeRange
    var useLogScale: Bool
    var layoutMode: LayoutMode
    var createdAt: Date

    static func == (lhs: SavedView, rhs: SavedView) -> Bool {
        lhs.id == rhs.id
    }
}
