import Foundation

/// A user-defined function, with the name sets needed to give each call a clean scope.
struct PineRuntimeFunction {
    let parameters: [PineParameter]
    let body: [PineStatement]
    /// Parameters and every name declared in the body; restored after each call.
    let locals: Set<String>
    /// `var`/`varip` locals, which persist per call site between calls.
    let persistentLocals: Set<String>

    init(parameters: [PineParameter], body: [PineStatement]) {
        var locals = Set(parameters.map(\.name))
        var persistent = Set<String>()
        Self.collectDeclarations(body, into: &locals, persistent: &persistent)
        self.parameters = parameters
        self.body = body
        self.locals = locals
        self.persistentLocals = persistent
    }

    private static func collectDeclarations(
        _ statements: [PineStatement], into names: inout Set<String>, persistent: inout Set<String>
    ) {
        for statement in statements {
            switch statement {
            case .declaration(let name, _, let mode, let value, _):
                names.insert(name)
                if mode != .ordinary { persistent.insert(name) }
                if case .statementExpression(let block, _) = value {
                    collectDeclarations([block], into: &names, persistent: &persistent)
                }
            case .forRange(let variable, _, _, _, _, _): names.insert(variable)
            case .forIn(let index, let value, _, _, _):
                if let index { names.insert(index) }
                names.insert(value)
            case .tupleDeclaration(let tupleNames, _, _): names.formUnion(tupleNames)
            default: break
            }
            for block in statement.nestedBlocks {
                collectDeclarations(block, into: &names, persistent: &persistent)
            }
        }
    }
}
