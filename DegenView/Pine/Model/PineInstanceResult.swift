import Foundation

/// Everything one applied Pine instance currently has to show. Replaced wholesale on every
/// successful execution — never partially mutated, so the legend/pane/overlay/settings
/// readers never see a half-updated instance.
struct PineInstanceResult: Sendable {
    var output: PineVisualOutput = .empty
    var declaration: PineDeclarationMetadata?
    var inputSchema: PineInputSchema = PineInputSchema()
    var diagnostics: [PineDiagnostic] = []
    var state: State = .evaluating
    /// Whether the last execution ran on a realtime bar.
    var isLive = false
    /// `alert()` / `alertcondition()` calls in the script — zero means an alert would never fire.
    var alertCallCount = 0

    enum State: Sendable, Equatable {
        case evaluating
        case ready
        /// Nothing fresh to show; the text says why ("Compile failed", "Script no longer exists").
        case failed(String)
    }
}
