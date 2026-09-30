import Foundation

enum PineDiagnosticCategory: String, Codable, Sendable {
    case lexical, syntax, semantic, unsupported, resource, runtime, cancellation
}
