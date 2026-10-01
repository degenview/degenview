import Foundation

struct PineCompiledProgram: Sendable {
    let source: String
    let statements: [PineStatement]
    let declaration: PineDeclarationMetadata
    let inputSchema: PineInputSchema
    let diagnostics: [PineDiagnostic]
    /// Names declared with `method`, which `value.name(…)` resolves to ahead of builtins.
    var methodNames: Set<String> = []
    var isValid: Bool { !diagnostics.contains { $0.severity == .error } }
    /// Every `alert()` and `alertcondition()` call, in source order.
    var alertCallSites: [PineAlertCallSite] { PineAlertCallValidator.callSites(in: statements) }
}
