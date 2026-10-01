import Foundation

/// `map.*`: `map.new<K, V>()` and the operations a map takes, also as methods (`zones.get(k)`).
extension PineRuntimeSession {
    func mapCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if call.name == "map.new" {
            let id = allocate()
            working.maps[id] = PineMap()
            return .ref(.map, id)
        }
        let member = String(call.name.dropFirst("map.".count))
        let b = try bind(call, Self.mapParameters[member] ?? ["id"], &context)
        guard case .ref(.map, let id)? = b["id"], var map = working.maps[id] else {
            throw PineDiagnostic.error(
                "PINE4026", .runtime, "\(call.name) requires a map; got na.", call.range)
        }
        let result: PineRuntimeValue
        switch member {
        case "put":
            map.put(b["value"] ?? .na, for: try mapKey(b["key"], call))
            working.maps[id] = map
            result = .void
        case "get": result = map.value(for: try mapKey(b["key"], call)) ?? .na
        case "contains": result = .bool(map.contains(try mapKey(b["key"], call)))
        case "remove":
            result = map.remove(try mapKey(b["key"], call)) ?? .na
            working.maps[id] = map
        case "size": result = .int(map.count)
        case "clear":
            map.removeAll()
            working.maps[id] = map
            result = .void
        case "keys": result = newArrayValue(map.keys)
        case "values": result = newArrayValue(map.values)
        case "copy":
            let copy = allocate()
            working.maps[copy] = map
            result = .ref(.map, copy)
        case "put_all":
            guard case .ref(.map, let otherID)? = b["id2"], let other = working.maps[otherID] else {
                throw PineDiagnostic.error(
                    "PINE4026", .runtime, "map.put_all requires a map; got na.", call.range)
            }
            for (key, value) in other.pairs {
                if let key = PineMap.Key(key) { map.put(value, for: key) }
            }
            working.maps[id] = map
            result = .void
        default: throw call.unknownFunction
        }
        return result
    }

    private static let mapParameters: [String: [String]] = [
        "put": ["id", "key", "value"], "get": ["id", "key"], "contains": ["id", "key"],
        "remove": ["id", "key"], "put_all": ["id", "id2"],
    ]

    private func mapKey(_ value: PineRuntimeValue?, _ call: PineCall) throws -> PineMap.Key {
        guard let value, let key = PineMap.Key(value) else {
            throw PineDiagnostic.error(
                "PINE4027", .runtime, "\(call.name) needs a number, bool, string or color as its key.",
                call.range)
        }
        return key
    }

    private func newArrayValue(_ items: [PineRuntimeValue]) -> PineRuntimeValue {
        let id = allocate()
        working.arrays[id] = items
        return .ref(.array, id)
    }
}
