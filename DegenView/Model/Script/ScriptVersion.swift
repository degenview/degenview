import Foundation

struct ScriptVersion: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var scriptID: UUID
    var createdAt: Date
    var source: String
    var compileStatus: CompileStatus
}
