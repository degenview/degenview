import Foundation

enum CompileStatus: String, Codable, CaseIterable, Sendable {
    case notCompiled, valid, warning, error
}
