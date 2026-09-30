import Foundation

/// `math.*`. Pure: the only session input is `mintick`, for `math.round_to_mintick`.
enum PineMath {
    private static let unary: [String: (Double) -> Double] = [
        "math.sqrt": { sqrt($0) }, "math.log": { log($0) }, "math.exp": { exp($0) },
        "math.log10": { log10($0) }, "math.sin": { sin($0) }, "math.cos": { cos($0) },
        "math.tan": { tan($0) }, "math.asin": { asin($0) }, "math.acos": { acos($0) },
        "math.atan": { atan($0) }, "math.todegrees": { $0 * 180 / .pi },
        "math.toradians": { $0 * .pi / 180 },
    ]

    static func call(_ name: String, _ values: [PineRuntimeValue], mintick: Double) -> PineRuntimeValue {
        let first = values.first ?? .na
        if let function = unary[name] { return first.number.map { .float(function($0)) } ?? .na }
        switch name {
        case "math.max", "math.min": return extremum(isMax: name == "math.max", values)
        case "math.abs":
            if case .int(let x) = first { return .int(x == .min ? .max : abs(x)) }
            return first.number.map { .float(abs($0)) } ?? .na
        case "math.round": return round(values)
        case "math.floor", "math.ceil":
            guard let x = first.number, x.isFinite else { return .na }
            return .integral(name == "math.floor" ? x.rounded(.down) : x.rounded(.up))
        case "math.sign":
            guard let x = first.number else { return .na }
            return .float(x > 0 ? 1 : x < 0 ? -1 : 0)
        case "math.avg":
            guard let numbers = allNumbers(values) else { return .na }
            return .float(numbers.reduce(0, +) / Double(numbers.count))
        case "math.round_to_mintick":
            guard let x = first.number, x.isFinite, mintick > 0 else { return .na }
            return .float((x / mintick).rounded() * mintick)
        case "math.pow":
            guard values.count > 1, let x = first.number, let y = values[1].number else { return .na }
            return .float(pow(x, y))
        default: return .na
        }
    }

    /// Every value as a number, or nil when there are none or any is not numeric.
    private static func allNumbers(_ values: [PineRuntimeValue]) -> [Double]? {
        let numbers = values.compactMap(\.number)
        return !numbers.isEmpty && numbers.count == values.count ? numbers : nil
    }

    private static func extremum(isMax: Bool, _ values: [PineRuntimeValue]) -> PineRuntimeValue {
        guard let numbers = allNumbers(values) else { return .na }
        if values.allSatisfy(\.isInt) {
            let ints = values.compactMap { value -> Int? in
                if case .int(let x) = value { return x }
                return nil
            }
            return (isMax ? ints.max() : ints.min()).map(PineRuntimeValue.int) ?? .na
        }
        return (isMax ? numbers.max() : numbers.min()).map(PineRuntimeValue.float) ?? .na
    }

    private static func round(_ values: [PineRuntimeValue]) -> PineRuntimeValue {
        guard let x = values.first?.number, x.isFinite else { return .na }
        if values.count > 1, let precision = values[1].number {
            let scale = pow(10, precision.rounded())
            return .float((x * scale).rounded() / scale)
        }
        return .integral(x.rounded())
    }
}
