import Foundation

/// Everything one applied Pine instance currently has to show. Replaced wholesale on every
/// successful execution — never partially mutated, so the legend/pane/overlay/settings
/// readers never see a half-updated instance.
struct PineInstanceResult: Sendable {
    var output: PineVisualOutput = .empty
    var declaration: PineDeclarationMetadata?
    var inputSchema: PineInputSchema = PineInputSchema()
    var diagnostics: [PineDiagnostic] = []
    var status: String = "Evaluating…"
}
