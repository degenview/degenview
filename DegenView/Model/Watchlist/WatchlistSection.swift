import Foundation

/// A labelled divider. The instruments after it, up to the next section, belong to it.
struct WatchlistSection: Identifiable, Codable, Equatable, Sendable {
    let id: UUID
    var title: String
    var isCollapsed: Bool

    init(id: UUID = UUID(), title: String, isCollapsed: Bool = false) {
        self.id = id
        self.title = title
        self.isCollapsed = isCollapsed
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        title = try container.decode(String.self, forKey: .title)
        isCollapsed = try container.decodeIfPresent(Bool.self, forKey: .isCollapsed) ?? false
    }
}
