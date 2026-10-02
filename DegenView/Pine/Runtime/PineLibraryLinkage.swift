import Foundation

/// What the imported libraries contribute to a run, flattened out of the compiled import graph. Each library is
/// identified by its import path (`user/name/version`); its functions, types and enums are registered under
/// `"<path>::<name>"` so they cannot collide with the script's own or with another library's.
struct PineLibraryLinkage {
    /// Scope (a library path, or `""` for the script) -> import alias -> library path.
    var aliases: [String: [String: String]] = [:]
    /// Library path -> the names it exports.
    var exports: [String: Set<String>] = [:]
    /// Library path -> its top-level declarations (the constants), evaluated on first use.
    var constants: [String: [String: PineExpression]] = [:]
    var functions: [String: PineRuntimeFunction] = [:]
    var methods: [String: [PineRuntimeFunction]] = [:]
    var types: [String: [PineTypeField]] = [:]
    var enums: [String: [PineEnumMember]] = [:]

    init(_ program: PineCompiledProgram) {
        aliases[""] = Dictionary(program.imports.map { ($0.alias, $0.path) }, uniquingKeysWith: { first, _ in first })
        var seen: Set<String> = []
        visit(program.imports, seen: &seen)
    }

    private mutating func visit(_ imports: [PineImport], seen: inout Set<String>) {
        for entry in imports {
            guard let library = entry.library, seen.insert(entry.path).inserted else { continue }
            aliases[entry.path] = Dictionary(
                library.imports.map { ($0.alias, $0.path) }, uniquingKeysWith: { first, _ in first })
            register(library, key: entry.path)
            visit(library.imports, seen: &seen)
        }
    }

    private mutating func register(_ library: PineCompiledProgram, key: String) {
        exports[key] = library.exportedNames
        let declaredTypes = Set(
            library.statements.compactMap { statement -> String? in
                if case .typeDeclaration(let name, _, _) = statement { return name }
                if case .enumDeclaration(let name, _, _) = statement { return name }
                return nil
            })
        for statement in library.statements {
            switch statement {
            case .typeDeclaration(let name, let fields, _): types["\(key)::\(name)"] = fields
            case .enumDeclaration(let name, let members, _): enums["\(key)::\(name)"] = members
            case .declaration(let name, _, .ordinary, let expression, _): constants[key, default: [:]][name] = expression
            case .function(let name, let parameters, let body, _):
                var function = PineRuntimeFunction(
                    parameters: parameters.map { canonical($0, scope: key, declared: declaredTypes) }, body: body)
                function.scope = key
                function.isExported = library.exportedNames.contains(name)
                functions["\(key)::\(name)"] = function
                if library.methodNames.contains(name) { methods[name, default: []].append(function) }
            default: break
            }
        }
    }

    /// A parameter whose type name is spelled as the importing code spells it, with the canonical name instances
    /// carry: `alias.Type` becomes `path::Type`, and a type the library declares itself is qualified by its path.
    func canonical(_ parameter: PineParameter, scope: String, declared: Set<String> = []) -> PineParameter {
        guard let name = parameter.typeName else { return parameter }
        var result = parameter
        if let dot = name.firstIndex(of: "."), let path = aliases[scope]?[String(name[..<dot])] {
            result.typeName = "\(path)::\(name[name.index(after: dot)...])"
        } else if declared.contains(name), !scope.isEmpty {
            result.typeName = "\(scope)::\(name)"
        }
        return result
    }

    /// `alias.rest` as seen from `scope`: the library path and what follows the alias.
    func resolve(_ name: String, scope: String) -> (path: String, rest: String)? {
        guard let dot = name.firstIndex(of: "."), let path = aliases[scope]?[String(name[..<dot])] else {
            return nil
        }
        return (path, String(name[name.index(after: dot)...]))
    }
}
