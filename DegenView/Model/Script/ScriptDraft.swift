import Foundation

struct ScriptDraft: Codable, Equatable, Sendable {
    var scriptID: UUID
    var source: String
    var modifiedAt: Date
    var basedOnRevisionID: UUID?
}
