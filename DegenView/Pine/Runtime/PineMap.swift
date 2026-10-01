import Foundation

/// A `map<K, V>`: insertion-ordered, like Pine's. Keys are int, float, bool, string or color; a whole
/// float and the int with the same value are one key, because a script that passes `2` where a float
/// key is expected means `2.0`.
struct PineMap {
    enum Key: Hashable {
        case int(Int)
        case float(UInt64)
        case bool(Bool)
        case string(String)
        case color(UInt32)

        /// Nil for a value that cannot be a key (`na`, handles, tuples).
        init?(_ value: PineRuntimeValue) {
            switch value {
            case .int(let x): self = .int(x)
            case .float(let x):
                // Only a float that is exactly a whole number below 2^53 folds into the int key.
                if x == x.rounded(), abs(x) < 9_007_199_254_740_992 {
                    self = .int(Int(x))
                } else {
                    self = .float(x.bitPattern)
                }
            case .bool(let x): self = .bool(x)
            case .string(let x): self = .string(x)
            case .color(let x): self = .color(x)
            default: return nil
            }
        }

        var value: PineRuntimeValue {
            switch self {
            case .int(let x): .int(x)
            case .float(let bits): .float(Double(bitPattern: bits))
            case .bool(let x): .bool(x)
            case .string(let x): .string(x)
            case .color(let x): .color(x)
            }
        }
    }

    private(set) var order: [Key] = []
    private var storage: [Key: PineRuntimeValue] = [:]

    var count: Int { order.count }
    var pairs: [(PineRuntimeValue, PineRuntimeValue)] { order.map { ($0.value, storage[$0] ?? .na) } }
    var keys: [PineRuntimeValue] { order.map(\.value) }
    var values: [PineRuntimeValue] { order.map { storage[$0] ?? .na } }

    func contains(_ key: Key) -> Bool { storage[key] != nil }
    func value(for key: Key) -> PineRuntimeValue? { storage[key] }

    /// Stores `value`, returning what the key held before.
    @discardableResult
    mutating func put(_ value: PineRuntimeValue, for key: Key) -> PineRuntimeValue? {
        let previous = storage.updateValue(value, forKey: key)
        if previous == nil { order.append(key) }
        return previous
    }

    @discardableResult
    mutating func remove(_ key: Key) -> PineRuntimeValue? {
        guard let removed = storage.removeValue(forKey: key) else { return nil }
        order.removeAll { $0 == key }
        return removed
    }

    mutating func removeAll() {
        order = []
        storage = [:]
    }
}
