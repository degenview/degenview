import Foundation

/// How one watchlist looks. Sorting lives here but never rewrites the manual order.
struct WatchlistDisplaySettings: Codable, Equatable, Sendable {
    var columns: [WatchlistColumn] = [.last, .changePercent]
    var sort: WatchlistSort = .manual
    var showsDescription = true

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        columns = try container.decodeIfPresent([WatchlistColumn].self, forKey: .columns) ?? columns
        sort = try container.decodeIfPresent(WatchlistSort.self, forKey: .sort) ?? .manual
        showsDescription = try container.decodeIfPresent(Bool.self, forKey: .showsDescription) ?? true
    }
}
