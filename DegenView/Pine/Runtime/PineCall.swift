import Foundation

/// One call expression as the interpreter sees it.
struct PineCall {
    let name: String
    let arguments: [PineArgument]
    /// Source-order call-site number; keys per-site runtime state.
    let site: Int
    let range: PineSourceRange

    var unknownFunction: PineDiagnostic {
        .error("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
    }
}
