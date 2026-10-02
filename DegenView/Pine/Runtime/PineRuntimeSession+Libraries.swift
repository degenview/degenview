import Foundation

/// Name resolution for imported libraries. A library's code runs in its own scope (`context.scope`, the import
/// path): its functions, types, enums and constants shadow nothing of the script's and are invisible to it
/// except through the alias and only when exported.
extension PineRuntimeSession {
    private static let separator = "::"

    /// The library a registry key (`"user/name/1::Type"`) belongs to; nil for the script's own.
    static func library(of key: String) -> String? {
        key.range(of: separator).map { String(key[..<$0.lowerBound]) }
    }

    /// The user function `name` names from where the code is running: the running library's own, the
    /// script's own, or `alias.name` of an imported library.
    func scopedFunction(
        _ name: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> PineRuntimeFunction? {
        if let scope = context.scope {
            if let own = functions["\(scope)\(Self.separator)\(name)"] { return own }
        } else if let own = functions[name] {
            return own
        }
        guard let (path, member) = linkage.resolve(name, scope: context.scope ?? ""),
            let function = functions["\(path)\(Self.separator)\(member)"]
        else { return nil }
        try requireExport(member, of: path, name, range)
        return function
    }

    /// The registry key of the type `name` refers to from the running code, if it is one.
    func typeKey(
        _ name: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> String? {
        if let scope = context.scope {
            let own = "\(scope)\(Self.separator)\(name)"
            if types[own] != nil { return own }
        } else if types[name] != nil {
            return name
        }
        guard let (path, member) = linkage.resolve(name, scope: context.scope ?? ""),
            types["\(path)\(Self.separator)\(member)"] != nil
        else { return nil }
        try requireExport(member, of: path, name, range)
        return "\(path)\(Self.separator)\(member)"
    }

    /// Whether `name` must not read the script's variables: library code only sees its own locals.
    func isolatedFromScript(_ name: String, _ context: PineRuntimeContext) -> Bool {
        guard context.scope != nil else { return false }
        let root = name.split(separator: ".", maxSplits: 1).first.map(String.init) ?? name
        return !(context.locals?.contains(root) ?? false)
    }

    /// A constant or enum member a library defines, read from inside it or through an import alias. Nil when
    /// `name` is neither, so the ordinary lookup carries on.
    func libraryIdentifier(
        _ name: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        if let scope = context.scope, isolatedFromScript(name, context),
            let value = try libraryMember(name, of: scope, range, context)
        {
            return value
        }
        guard let (path, member) = linkage.resolve(name, scope: context.scope ?? "") else { return nil }
        guard let value = try libraryMember(member, of: path, range, context) else { return nil }
        try requireExport(member.split(separator: ".").first.map(String.init) ?? member, of: path, name, range)
        return value
    }

    /// `Mode.fast` written as a method receiver, in the script or a library.
    func enumMember(
        _ name: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        if let value = try libraryIdentifier(name, range, context), case .string = value { return value }
        guard context.scope == nil, let dot = name.firstIndex(of: "."), enums[String(name[..<dot])] != nil
        else { return nil }
        return try resolveIdentifier(name, range, context)
    }

    /// Evaluates `expression` the way `function` would: a default parameter value belongs to its definition.
    func evaluate(
        _ expression: PineExpression, inScopeOf function: PineRuntimeFunction,
        _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let saved = (context.scope, context.locals)
        context.scope = function.scope
        context.locals = function.scope == nil ? nil : function.locals
        defer { (context.scope, context.locals) = saved }
        return try eval(expression, &context)
    }

    /// Evaluates `expression` in `library`'s scope (a type's field default), or the script's when nil.
    func evaluate(
        _ expression: PineExpression, inLibrary library: String?, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let saved = (context.scope, context.locals)
        context.scope = library
        context.locals = library == nil ? nil : []
        defer { (context.scope, context.locals) = saved }
        return try eval(expression, &context)
    }

    private func libraryMember(
        _ name: String, of path: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        if let value = try libraryConstant(name, in: path, range, context) { return value }
        guard let dot = name.firstIndex(of: ".") else { return nil }
        let enumName = String(name[..<dot])
        let key = "\(path)\(Self.separator)\(enumName)"
        guard let members = enums[key] else { return nil }
        let member = String(name[name.index(after: dot)...])
        guard members.contains(where: { $0.name == member }) else {
            throw PineDiagnostic.error(
                "PINE4025", .runtime, "Enum '\(enumName)' has no member '\(member)'.", range)
        }
        return .string("\(key).\(member)")
    }

    private func libraryConstant(
        _ name: String, in path: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        guard let expression = linkage.constants[path]?[name] else { return nil }
        let key = "\(path)\(Self.separator)\(name)"
        if let cached = libraryConstants[key] { return cached }
        guard constantsInProgress.insert(key).inserted else {
            throw PineDiagnostic.error(
                "PINE4032", .runtime, "Library constant '\(name)' is defined in terms of itself.", range)
        }
        defer { constantsInProgress.remove(key) }
        var inner = context
        inner.scope = path
        inner.locals = []
        let value = try eval(expression, &inner)
        if case .ref = value { return value }
        libraryConstants[key] = value
        return value
    }

    private func requireExport(
        _ member: String, of path: String, _ written: String, _ range: PineSourceRange
    ) throws {
        guard linkage.exports[path]?.contains(member) == true else {
            throw PineDiagnostic.error(
                "PINE4031", .runtime, "'\(written)' is not exported by the library.", range)
        }
    }
}
