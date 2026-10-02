import Foundation

/// A user-defined function, with the name sets needed to give each call a clean scope.
struct PineRuntimeFunction {
    let parameters: [PineParameter]
    let body: [PineStatement]
    /// Parameters and every name declared in the body; restored after each call.
    let locals: Set<String>
    /// `var`/`varip` locals, which persist per call site between calls.
    let persistentLocals: Set<String>
    /// The library that defines the function; its body runs in that library's scope.
    var scope: String?
    /// Whether the importing script may call it (a library's helpers are private).
    var isExported = true

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

    /// Whether `value` is something this function's first parameter accepts, which is how a method is
    /// picked by its receiver. An untyped parameter accepts anything.
    func acceptsReceiver(_ value: PineRuntimeValue, instances: [Int: PineObject]) -> Bool {
        guard let first = parameters.first else { return false }
        if let name = first.typeName {
            switch value {
            case .ref(.object, let id): return instances[id]?.typeName == name
            case .string(let text): return text.hasPrefix(name + ".")  // an enum member
            default: return false
            }
        }
        guard let type = first.type else { return true }
        switch (type, value) {
        case (.int, .int), (.float, .int), (.float, .float), (.bool, .bool), (.string, .string),
            (.color, .color), (.time, .int):
            return true
        case (.array, .ref(.array, _)), (.map, .ref(.map, _)), (.matrix, .ref(.matrix, _)),
            (.line, .ref(.line, _)), (.label, .ref(.label, _)), (.box, .ref(.box, _)),
            (.table, .ref(.table, _)):
            return true
        case (.object, .ref(let kind, _)): return [.object, .linefill, .polyline].contains(kind)
        default: return false
        }
    }
}
