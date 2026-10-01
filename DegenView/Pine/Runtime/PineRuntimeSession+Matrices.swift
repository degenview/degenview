import Foundation

/// `matrix.*`: `matrix.new<T>(rows, columns, initial)` and the operations a matrix takes, also as methods
/// (`m.get(r, c)`). Element-wise matrix arithmetic and linear algebra are not supported.
extension PineRuntimeSession {
    private static let matrixParameters: [String: [String]] = [
        "new": ["rows", "columns", "initial_value"], "get": ["id", "row", "column"],
        "set": ["id", "row", "column", "value"], "row": ["id", "row"], "col": ["id", "column"],
        "add_row": ["id", "row", "array_id"], "add_col": ["id", "column", "array_id"],
        "remove_row": ["id", "row"], "remove_col": ["id", "column"], "fill": ["id", "value"],
    ]

    func matrixCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let member = String(call.name.dropFirst("matrix.".count))
        let b = try bind(call, Self.matrixParameters[member] ?? ["id"], &context)
        if member == "new" {
            let id = allocate()
            working.matrices[id] = PineMatrix(
                rows: b["rows"].intValue ?? 0, columns: b["columns"].intValue ?? 0,
                initial: b["initial_value"] ?? .na)
            return .ref(.matrix, id)
        }
        guard case .ref(.matrix, let id)? = b["id"], var matrix = working.matrices[id] else {
            throw PineDiagnostic.error(
                "PINE4028", .runtime, "\(call.name) requires a matrix; got na.", call.range)
        }
        let result: PineRuntimeValue
        switch member {
        case "get":
            let (row, column) = try matrixCell(b, matrix, call)
            result = matrix[row, column]
        case "set":
            let (row, column) = try matrixCell(b, matrix, call)
            matrix[row, column] = b["value"] ?? .na
            result = .void
        case "rows": result = .int(matrix.rows)
        case "columns": result = .int(matrix.columns)
        case "elements_count": result = .int(matrix.count)
        case "row": result = matrixArray(matrix.row(try matrixIndex(b["row"], matrix.rows, call)))
        case "col": result = matrixArray(matrix.column(try matrixIndex(b["column"], matrix.columns, call)))
        case "add_row":
            let at = try matrixIndex(b["row"], matrix.rows, call, allowEnd: true)
            matrix.insertRow(at: at, matrixValues(b["array_id"]))
            result = .void
        case "add_col":
            let at = try matrixIndex(b["column"], matrix.columns, call, allowEnd: true)
            matrix.insertColumn(at: at, matrixValues(b["array_id"]))
            result = .void
        case "remove_row":
            guard matrix.rows > 0 else { throw matrixBounds(call, "row", nil, 0) }
            matrix.removeRow(at: try matrixIndex(b["row"], matrix.rows, call, defaultIndex: matrix.rows - 1))
            result = .void
        case "remove_col":
            guard matrix.columns > 0 else { throw matrixBounds(call, "column", nil, 0) }
            matrix.removeColumn(
                at: try matrixIndex(b["column"], matrix.columns, call, defaultIndex: matrix.columns - 1))
            result = .void
        case "fill":
            matrix.fill(b["value"] ?? .na)
            result = .void
        case "copy":
            let copy = allocate()
            working.matrices[copy] = matrix
            return .ref(.matrix, copy)
        case "transpose":
            let copy = allocate()
            working.matrices[copy] = matrix.transposed()
            return .ref(.matrix, copy)
        case "avg", "min", "max":
            let numbers = matrix.allElements.compactMap(\.number)
            guard !numbers.isEmpty else { return .na }
            switch member {
            case "avg": return .float(numbers.reduce(0, +) / Double(numbers.count))
            case "min": return .float(numbers.min() ?? 0)
            default: return .float(numbers.max() ?? 0)
            }
        default: throw call.unknownFunction
        }
        working.matrices[id] = matrix
        return result
    }

    private func matrixCell(
        _ b: [String: PineRuntimeValue], _ matrix: PineMatrix, _ call: PineCall
    ) throws -> (Int, Int) {
        guard let row = b["row"].intValue, let column = b["column"].intValue,
            matrix.contains(row: row, column: column)
        else {
            let shown = "\(b["row"].intValue.map(String.init) ?? "na"), \(b["column"].intValue.map(String.init) ?? "na")"
            throw PineDiagnostic.error(
                "PINE4010", .runtime,
                "\(call.name) cell (\(shown)) is out of bounds (\(matrix.rows)×\(matrix.columns)).", call.range)
        }
        return (row, column)
    }

    /// A row or column index; `allowEnd` admits `count` (append) and an absent one means the end.
    private func matrixIndex(
        _ value: PineRuntimeValue?, _ count: Int, _ call: PineCall, allowEnd: Bool = false,
        defaultIndex: Int? = nil
    ) throws -> Int {
        guard let value, value != .na else { return defaultIndex ?? count }
        guard let index = value.intValue, index >= 0, index < count + (allowEnd ? 1 : 0) else {
            throw matrixBounds(call, "index", value.intValue, count)
        }
        return index
    }

    private func matrixBounds(_ call: PineCall, _ what: String, _ shown: Int?, _ count: Int) -> PineDiagnostic {
        .error(
            "PINE4010", .runtime,
            "\(call.name) \(what) \(shown.map(String.init) ?? "na") is out of bounds (size \(count)).", call.range)
    }

    private func matrixValues(_ value: PineRuntimeValue?) -> [PineRuntimeValue] {
        guard case .ref(.array, let id)? = value else { return [] }
        return working.arrays[id] ?? []
    }

    private func matrixArray(_ items: [PineRuntimeValue]) -> PineRuntimeValue {
        let id = allocate()
        working.arrays[id] = items
        return .ref(.array, id)
    }
}
