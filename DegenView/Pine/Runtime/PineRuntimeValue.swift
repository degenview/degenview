import Foundation

enum PineRuntimeValue: Equatable, Sendable {
    case int(Int)
    case float(Double)
    case bool(Bool)
    case string(String)
    case color(UInt32)
    case tuple([PineRuntimeValue])
    /// Handle to a runtime-owned object (array, drawing, plot).
    case ref(PineRefKind, Int)
    case na, void

    var number: Double? {
        switch self {
        case .int(let v): return Double(v)
        case .float(let v): return v
        default: return nil
        }
    }

    var bool: Bool? {
        if case .bool(let v) = self { return v }
        return nil
    }

    var isInt: Bool {
        if case .int = self { return true }
        return false
    }

    /// The number as an `Int`, or nil when not numeric or out of exact range.
    var intValue: Int? { number.flatMap { Int(pine: $0) } }

    var textValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    /// A whole number as `.int` when it is exactly representable, otherwise `.float`.
    static func integral(_ value: Double) -> PineRuntimeValue {
        Int(pine: value).map(PineRuntimeValue.int) ?? .float(value)
    }
}

extension Optional where Wrapped == PineRuntimeValue {
    /// The number as an `Int`, or nil when absent, not numeric, or out of exact range.
    var intValue: Int? { self?.number.flatMap { Int(pine: $0) } }

    var textValue: String? {
        if case .string(let s)? = self { return s }
        return nil
    }

    /// Absent -> `fallback`; a color -> itself; `na` (or anything else) -> no color.
    func colorValue(fallback: UInt32?) -> UInt32? {
        guard let value = self else { return fallback }
        if case .color(let c) = value { return c }
        return nil
    }
}
