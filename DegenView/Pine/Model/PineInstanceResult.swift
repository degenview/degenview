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
    /// `alert()` / `alertcondition()` calls in the script — zero means an alert would never fire.
    var alertCallCount = 0
}
