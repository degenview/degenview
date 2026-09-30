import Foundation

enum ScriptType: String, Codable, CaseIterable, Sendable, Identifiable {
    case indicator, strategy, library
    var id: String { rawValue }
    var displayName: String { rawValue.capitalized }
}
