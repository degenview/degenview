import Foundation

struct PineDiagnostic: Error, Codable, Equatable, Sendable, Identifiable {
    var id: String { "\(code):\(range.start.offset):\(message)" }
    let code: String
    let severity: PineDiagnosticSeverity
    let category: PineDiagnosticCategory
    let message: String
    let range: PineSourceRange
}
