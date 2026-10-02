import Foundation

struct PineCompiledProgram: Sendable {
    let source: String
    let statements: [PineStatement]
    let declaration: PineDeclarationMetadata
    let inputSchema: PineInputSchema
    let diagnostics: [PineDiagnostic]
    /// Names declared with `method`, which `value.name(…)` resolves to ahead of builtins.
    var methodNames: Set<String> = []
    /// Names a library declares with `export`: functions, constants, types, enums and methods.
    var exportedNames: Set<String> = []
    /// The libraries this script imports, with the compiled program of each that was found.
    var imports: [PineImport] = []
    var isValid: Bool { !diagnostics.contains { $0.severity == .error } }
    /// Every `alert()` and `alertcondition()` call, in source order.
    var alertCallSites: [PineAlertCallSite] { PineAlertCallValidator.callSites(in: statements) }
}
