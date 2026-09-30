import Foundation

extension PineDiagnostic {
    static func error(
        _ code: String, _ category: PineDiagnosticCategory, _ message: String,
        _ range: PineSourceRange
    ) -> PineDiagnostic {
        .init(code: code, severity: .error, category: category, message: message, range: range)
    }

    static func warning(
        _ code: String, _ category: PineDiagnosticCategory, _ message: String,
        _ range: PineSourceRange
    ) -> PineDiagnostic {
        .init(code: code, severity: .warning, category: category, message: message, range: range)
    }
}
