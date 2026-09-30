import Foundation

struct ScriptCompileRecord: Codable, Equatable, Sendable {
    var sourceHash: String
    var compilerVersion: String
    var pineVersion: Int?
    var status: CompileStatus
    var diagnostics: [PineDiagnostic]
    var declaration: PineDeclarationMetadata?
    var compiledAt: Date
}
