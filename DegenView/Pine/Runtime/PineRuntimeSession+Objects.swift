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
            guard case .ref(.object, let id) = current else {
                if current == .na { throw naObject(String(part), range) }
                return nil
            }
            guard let object = working.instances[id] else { throw naObject(String(part), range) }
            guard let value = object.fields[String(part)] else {
                throw noSuchField(String(part), in: object.typeName, range)
            }
            current = value
        }
        return current
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
