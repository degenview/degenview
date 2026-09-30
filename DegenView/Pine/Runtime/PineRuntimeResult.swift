import Foundation

struct PineRuntimeResult: Sendable {
    var output: PineVisualOutput
    var diagnostics: [PineDiagnostic]
    var barStates: [PineBarFlags]
}
