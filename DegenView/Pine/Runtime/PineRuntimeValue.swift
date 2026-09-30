import Foundation

indirect enum PineRuntimeValue: Equatable, Sendable {
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
}
