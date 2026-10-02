import Foundation

/// Copies the collections and objects a `request.security` expression returned out of the call site's own
/// state and into the chart's. Handles are indexes into one state's dictionaries, so a handle that crossed
/// over unchanged would name an unrelated value (or nothing) in the chart's state.
///
/// Arrays, maps, matrices and instances are copied, following handles held by arrays and instances (a handle
/// reached twice is copied once, so shared structure and cycles survive). Handles held inside a map or a
/// matrix are not followed and become `na`, and so do drawing handles, which belong to the expression's
/// state and are never drawn.
struct PineReferenceExport {
    private struct Origin: Hashable {
        var kind: PineRefKind
        var id: Int
    }

    private let source: PineRuntimeState
    private var copies: [Origin: PineRuntimeValue] = [:]

    init(from source: PineRuntimeState) { self.source = source }

    mutating func export(_ value: PineRuntimeValue, into target: inout PineRuntimeState) -> PineRuntimeValue {
        switch value {
        case .tuple(let items): return .tuple(items.map { export($0, into: &target) })
        case .ref(let kind, let id): return copy(kind, id, into: &target)
        default: return value
        }
    }

    private mutating func copy(
        _ kind: PineRefKind, _ id: Int, into target: inout PineRuntimeState
    ) -> PineRuntimeValue {
        let origin = Origin(kind: kind, id: id)
        if let known = copies[origin] { return known }
        switch kind {
        case .array:
            guard let items = source.arrays[id] else { return .na }
            let handle = register(origin, &target)
            target.arrays[handle.id] = []
            target.arrays[handle.id] = items.map { export($0, into: &target) }
            return handle.value
        case .object:
            guard let object = source.instances[id] else { return .na }
            let handle = register(origin, &target)
            target.instances[handle.id] = object
            var fields: [String: PineRuntimeValue] = [:]
            for (name, field) in object.fields { fields[name] = export(field, into: &target) }
            target.instances[handle.id]?.fields = fields
            return handle.value
        case .map:
            guard let map = source.maps[id] else { return .na }
            let handle = register(origin, &target)
            target.maps[handle.id] = map
            return handle.value
        case .matrix:
            guard let matrix = source.matrices[id] else { return .na }
            let handle = register(origin, &target)
            target.matrices[handle.id] = matrix
            return handle.value
        default: return .na
        }
    }

    private mutating func register(
        _ origin: Origin, _ target: inout PineRuntimeState
    ) -> (id: Int, value: PineRuntimeValue) {
        target.nextReference += 1
        let value = PineRuntimeValue.ref(origin.kind, target.nextReference)
        copies[origin] = value
        return (target.nextReference, value)
    }
}
