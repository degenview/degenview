import Foundation

struct LocalScript: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var name: String
    var type: ScriptType
    var source: String
    var latestRevisionID: UUID?
    var createdAt: Date
    var modifiedAt: Date
    var lastOpenedAt: Date?
    var isFavorite: Bool
    var compileRecord: ScriptCompileRecord?
}

extension Notification.Name {
    static let localScriptsDidChange = Notification.Name("DegenView.localScriptsDidChange")
}
