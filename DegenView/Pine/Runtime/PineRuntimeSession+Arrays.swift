import Foundation

extension PineRuntimeSession {
    /// Parameter names per operation, in positional order. Operations not listed take `id`.
    private static let arrayParameters: [String: [String]] = [
        "array.push": ["id", "value"], "array.unshift": ["id", "value"],
        "array.includes": ["id", "value"], "array.indexof": ["id", "value"],
        "array.get": ["id", "index"], "array.remove": ["id", "index"],
        "array.set": ["id", "index", "value"], "array.insert": ["id", "index", "value"],
        "array.sort": ["id", "order"], "array.sort_indices": ["id", "order"], "array.join": ["id", "separator"],
        "array.concat": ["id", "other"], "array.slice": ["id", "index_from", "index_to"],
        "array.variance": ["id", "biased"], "array.stdev": ["id", "biased"],
        "array.percentile_nearest_rank": ["id", "percentage"],
        "array.percentile_linear_interpolation": ["id", "percentage"],
        "array.covariance": ["id", "id2", "biased"],
    ]

    func arrayCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if call.name.hasPrefix("array.new_") {
            let b = try bind(call, ["size", "initial_value"], &context)
            let size = max(0, b["size"].intValue ?? 0)
            return newArray(Array(repeating: b["initial_value"] ?? .na, count: size))
        }
        if call.name == "array.from" { return newArray(try allArguments(call, &context)) }
        let b = try bind(call, Self.arrayParameters[call.name] ?? ["id"], &context)
        guard case .ref(.array, let id)? = b["id"], var items = working.arrays[id] else {
            throw PineDiagnostic.error(
                "PINE4011", .runtime, "\(call.name) requires an array; got na.", call.range)
        }
        let result: PineRuntimeValue
        if let mutated = try mutateArray(call, b, id: id, &items) {
            result = mutated
        } else if let queried = try queryArray(call, b, items) {
            result = queried
        } else {
            throw call.unknownFunction
        }
        working.arrays[id] = items
        return result
    }

    private func newArray(_ items: [PineRuntimeValue]) -> PineRuntimeValue {
        let id = allocate()
        working.arrays[id] = items
        return .ref(.array, id)
    }

    /// A validated element index; `allowEnd` admits `count` itself (insert, slice start).
    private func arrayIndex(
        _ call: PineCall, _ value: PineRuntimeValue?, count: Int, allowEnd: Bool = false
    ) throws -> Int {
        guard let i = value.intValue, i >= 0, i < count + (allowEnd ? 1 : 0) else {
            let shown = value.intValue.map(String.init) ?? "na"
            throw PineDiagnostic.error(
                "PINE4010", .runtime,
                "\(call.name) index \(shown) is out of bounds (size \(count)).", call.range)
        }
        return i
    }

    private func requireArray(_ call: PineCall, _ value: PineRuntimeValue?) throws -> [PineRuntimeValue] {
        guard case .ref(.array, let id)? = value, let items = working.arrays[id] else {
            throw PineDiagnostic.error(
                "PINE4011", .runtime, "\(call.name) requires two arrays.", call.range)
        }
        return items
    }

    /// Operations that change `items`. Returns nil when `call` is not one of them.
    private func mutateArray(
        _ call: PineCall, _ b: [String: PineRuntimeValue], id: Int, _ items: inout [PineRuntimeValue]
    ) throws -> PineRuntimeValue? {
        let value = b["value"] ?? .na
        switch call.name {
        case "array.push": items.append(value)
        case "array.unshift": items.insert(value, at: 0)
        case "array.set": items[try arrayIndex(call, b["index"], count: items.count)] = value
        case "array.insert":
            items.insert(value, at: try arrayIndex(call, b["index"], count: items.count, allowEnd: true))
        case "array.remove": return items.remove(at: try arrayIndex(call, b["index"], count: items.count))
        case "array.shift": return items.isEmpty ? .na : items.removeFirst()
        case "array.pop": return items.popLast() ?? .na
        case "array.clear": items.removeAll()
        case "array.reverse": items.reverse()
        case "array.sort": sort(&items, descending: b["order"].textValue == "order.descending")
        case "array.concat":
            items += try requireArray(call, b["other"])
            return .ref(.array, id)
        default: return nil
        }
        return .void
    }

    /// Numbers first (ascending or descending), then everything else by its text; `na` last.
    private func sort(_ items: inout [PineRuntimeValue], descending: Bool) {
        let mintick = self.mintick
        items.sort { Self.precedes($0, $1, descending: descending, mintick: mintick) }
    }

    private static func precedes(
        _ lhs: PineRuntimeValue, _ rhs: PineRuntimeValue, descending: Bool, mintick: Double
    ) -> Bool {
        switch (lhs.number, rhs.number) {
        case (let x?, let y?): return descending ? x > y : x < y
        case (nil, _?): return false
        case (_?, nil): return true
        default:
            return PineFormat.format(lhs, nil, mintick: mintick) < PineFormat.format(rhs, nil, mintick: mintick)
        }
    }

    /// `array.sort_indices`: the indexes that would sort `items`, ties keeping their original order.
    private func sortedIndices(_ items: [PineRuntimeValue], descending: Bool) -> [PineRuntimeValue] {
        let mintick = self.mintick
        return items.indices.sorted { a, b in
            if Self.precedes(items[a], items[b], descending: descending, mintick: mintick) { return true }
            if Self.precedes(items[b], items[a], descending: descending, mintick: mintick) { return false }
            return a < b
        }.map(PineRuntimeValue.int)
    }

    /// Operations that only read `items`. Returns nil when `call` is not one of them.
    private func queryArray(
        _ call: PineCall, _ b: [String: PineRuntimeValue], _ items: [PineRuntimeValue]
    ) throws -> PineRuntimeValue? {
        let value = b["value"] ?? .na
        switch call.name {
        case "array.get": return items[try arrayIndex(call, b["index"], count: items.count)]
        case "array.size": return .int(items.count)
        case "array.sort_indices":
            return newArray(sortedIndices(items, descending: b["order"].textValue == "order.descending"))
        case "array.first": return items.first ?? .na
        case "array.last": return items.last ?? .na
        case "array.includes": return .bool(items.contains(value))
        case "array.indexof": return .int(items.firstIndex(of: value) ?? -1)
        case "array.join":
            let separator = b["separator"].textValue ?? ","
            return .string(
                items.map { PineFormat.format($0, nil, mintick: mintick) }.joined(separator: separator))
        case "array.copy": return newArray(items)
        case "array.slice": return try slice(call, b, items)
        case "array.sum", "array.avg", "array.max", "array.min": return aggregate(call.name, items)
        case "array.median", "array.mode", "array.range", "array.variance", "array.stdev",
            "array.percentile_nearest_rank", "array.percentile_linear_interpolation":
            return statistic(call.name, b, items)
        case "array.covariance": return try covariance(call, b, items)
        default: return nil
        }
    }

    private func slice(
        _ call: PineCall, _ b: [String: PineRuntimeValue], _ items: [PineRuntimeValue]
    ) throws -> PineRuntimeValue {
        let from = try arrayIndex(call, b["index_from"], count: items.count, allowEnd: true)
        let to = b["index_to"].intValue ?? items.count
        guard to >= from, to <= items.count else {
            throw PineDiagnostic.error(
                "PINE4010", .runtime, "array.slice end index \(to) is out of bounds.", call.range)
        }
        return newArray(Array(items[from..<to]))
    }

    /// The statistics over the numbers in the array (`na` entries are ignored); `na` when there are none.
    /// `variance`, `stdev` and `covariance` are the population figures unless `biased = false`.
    private func statistic(
        _ name: String, _ b: [String: PineRuntimeValue], _ items: [PineRuntimeValue]
    ) -> PineRuntimeValue {
        let numbers = items.compactMap(\.number).sorted()
        guard !numbers.isEmpty else { return .na }
        let count = Double(numbers.count)
        switch name {
        case "array.median":
            let middle = numbers.count / 2
            return .float(
                numbers.count % 2 == 1 ? numbers[middle] : (numbers[middle - 1] + numbers[middle]) / 2)
        case "array.mode":
            var best = numbers[0]
            var bestCount = 0
            var run = 0
            for (index, value) in numbers.enumerated() {
                run = index > 0 && numbers[index - 1] == value ? run + 1 : 1
                if run > bestCount {
                    best = value
                    bestCount = run
                }
            }
            return .float(best)
        case "array.range": return .float((numbers.last ?? 0) - numbers[0])
        case "array.variance", "array.stdev":
            let mean = numbers.reduce(0, +) / count
            let squares = numbers.reduce(0) { $0 + ($1 - mean) * ($1 - mean) }
            let biased = b["biased"]?.bool ?? true
            guard biased || numbers.count > 1 else { return .na }
            let variance = squares / (biased ? count : count - 1)
            return .float(name == "array.variance" ? variance : variance.squareRoot())
        case "array.percentile_nearest_rank":
            guard let percentage = b["percentage"]?.number else { return .na }
            let rank = max(1, Int((percentage / 100 * count).rounded(.up)))
            return .float(numbers[min(numbers.count, rank) - 1])
        default:  // percentile_linear_interpolation
            guard let percentage = b["percentage"]?.number else { return .na }
            let position = min(max(percentage, 0), 100) / 100 * (count - 1)
            let lower = Int(position.rounded(.down))
            let upper = min(numbers.count - 1, lower + 1)
            return .float(numbers[lower] + (numbers[upper] - numbers[lower]) * (position - Double(lower)))
        }
    }

    /// `array.covariance(id1, id2, biased)` over the first `min(size)` pairs.
    private func covariance(
        _ call: PineCall, _ b: [String: PineRuntimeValue], _ items: [PineRuntimeValue]
    ) throws -> PineRuntimeValue {
        let other = try requireArray(call, b["id2"])
        let pairs = zip(items, other).compactMap { x, y in x.number.flatMap { a in y.number.map { (a, $0) } } }
        guard pairs.count > 1 || (pairs.count == 1 && (b["biased"]?.bool ?? true)) else { return .na }
        let count = Double(pairs.count)
        let meanX = pairs.reduce(0) { $0 + $1.0 } / count
        let meanY = pairs.reduce(0) { $0 + $1.1 } / count
        let total = pairs.reduce(0) { $0 + ($1.0 - meanX) * ($1.1 - meanY) }
        return .float(total / ((b["biased"]?.bool ?? true) ? count : count - 1))
    }

    private func aggregate(_ name: String, _ items: [PineRuntimeValue]) -> PineRuntimeValue {
        let numbers = items.compactMap(\.number)
        switch name {
        case "array.sum":
            let total = numbers.reduce(0, +)
            let integral = items.allSatisfy { $0.isInt || $0 == .na }
            return integral ? .integral(total) : .float(total)
        case "array.avg":
            return numbers.isEmpty ? .na : .float(numbers.reduce(0, +) / Double(numbers.count))
        case "array.max": return numbers.max().map(PineRuntimeValue.float) ?? .na
        default: return numbers.min().map(PineRuntimeValue.float) ?? .na
        }
    }
}
