import Foundation

/// Instances of script-defined types: construction, field reads and `copy()`.
extension PineRuntimeSession {
    /// `Type.new(…)`. Nil when `call` does not name a declared type's constructor.
    func constructorCall(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        guard call.name.hasSuffix(".new"), let fields = types[String(call.name.dropLast(4))] else {
            return nil
        }
        let typeName = String(call.name.dropLast(4))
        let names = fields.map(\.name)
        let supplied = try bind(call, names, &context)
        if let unknown = supplied.keys.sorted().first(where: { !names.contains($0) }) {
            throw noSuchField(unknown, in: typeName, call.range)
        }
        var values: [String: PineRuntimeValue] = [:]
        for field in fields {
            if let value = supplied[field.name] {
                values[field.name] = value
            } else if let initial = field.defaultValue {
                values[field.name] = try eval(initial, &context)
            } else {
                values[field.name] = .na
            }
        }
        return newInstance(PineObject(typeName: typeName, fields: values))
    }

    func newInstance(_ object: PineObject) -> PineRuntimeValue {
        let id = allocate()
        working.instances[id] = object
        return .ref(.object, id)
    }

    /// `object.copy()`: a new instance with the same field values (shallow, as in Pine).
    func copyInstance(_ value: PineRuntimeValue, _ range: PineSourceRange) throws -> PineRuntimeValue {
        guard case .ref(.object, let id) = value, let object = working.instances[id] else {
            throw naObject("copy", range)
        }
        return newInstance(object)
    }

    /// `base.field.field…` where `base` is a variable holding an object. Nil when the name does not
    /// start with such a variable, so builtin namespaces and enumeration constants are untouched.
    func fieldPath(_ name: String, _ range: PineSourceRange) throws -> PineRuntimeValue? {
        let parts = name.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count > 1, var current = working.variables[String(parts[0])] else { return nil }
        for part in parts.dropFirst() {
            if case .ref(.object, _) = current {
                current = try readField(String(part), of: current, range)
            } else if current == .na {
                throw naObject(String(part), range)
            } else {
                return nil
            }
        }
        return current
    }

    /// `holder.field` for an object handle. `na` and unknown fields are runtime errors.
    func readField(
        _ field: String, of holder: PineRuntimeValue, _ range: PineSourceRange
    ) throws -> PineRuntimeValue {
        guard case .ref(.object, let id) = holder, let object = working.instances[id] else {
            throw naObject(field, range)
        }
        guard let value = object.fields[field] else {
            throw noSuchField(field, in: object.typeName, range)
        }
        return value
    }

    /// `target := value` / `target += value` where `target` is a field path.
    func runFieldAssignment(
        _ target: PineExpression, _ op: PineAssignmentOperator, _ expression: PineExpression,
        _ range: PineSourceRange, _ context: inout PineRuntimeContext
    ) throws -> Step {
        let owner: String
        let field: String
        let holder: PineRuntimeValue?
        switch target {
        case .identifier(let path, _):
            guard let dot = path.lastIndex(of: ".") else { throw notAssignable(range) }
            owner = String(path[..<dot])
            field = String(path[path.index(after: dot)...])
            holder = owner.contains(".") ? try fieldPath(owner, range) : working.variables[owner]
        case .member(let base, let name, _):
            owner = "value"
            field = name
            holder = try eval(base, &context)
        default: throw notAssignable(range)
        }
        guard case .ref(.object, let id)? = holder, var object = working.instances[id] else {
            if holder == .na { throw naObject(field, range) }
            throw PineDiagnostic.error(
                "PINE4020", .runtime, "'\(owner)' is not an object, so '\(field)' cannot be assigned.", range)
        }
        guard let old = object.fields[field] else { throw noSuchField(field, in: object.typeName, range) }
        let rhs = try eval(expression, &context)
        let value = PineOperators.apply(op, old: old, rhs)
        object.fields[field] = value
        working.instances[id] = object
        return Step(value: value)
    }

    private func notAssignable(_ range: PineSourceRange) -> PineDiagnostic {
        .error("PINE2016", .runtime, "Only a variable or a field can be assigned to.", range)
    }

    private func noSuchField(
        _ field: String, in typeName: String, _ range: PineSourceRange
    ) -> PineDiagnostic {
        .error("PINE4019", .runtime, "Type '\(typeName)' has no field '\(field)'.", range)
    }

    private func naObject(_ field: String, _ range: PineSourceRange) -> PineDiagnostic {
        .error("PINE4018", .runtime, "Cannot read '\(field)' of an object that is na.", range)
    }
}
