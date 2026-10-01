import Foundation

/// A `matrix<T>`: row-major, rows and columns can grow and shrink. Element types are not tracked.
struct PineMatrix {
    private(set) var rows: Int
    private(set) var columns: Int
    private var elements: [PineRuntimeValue]

    init(rows: Int, columns: Int, initial: PineRuntimeValue) {
        self.rows = max(0, rows)
        self.columns = max(0, columns)
        elements = Array(repeating: initial, count: self.rows * self.columns)
    }

    var count: Int { elements.count }
    var allElements: [PineRuntimeValue] { elements }

    func contains(row: Int, column: Int) -> Bool {
        (0..<rows).contains(row) && (0..<columns).contains(column)
    }

    subscript(row: Int, column: Int) -> PineRuntimeValue {
        get { elements[row * columns + column] }
        set { elements[row * columns + column] = newValue }
    }

    func row(_ index: Int) -> [PineRuntimeValue] { (0..<columns).map { self[index, $0] } }
    func column(_ index: Int) -> [PineRuntimeValue] { (0..<rows).map { self[$0, index] } }

    /// Inserts a row before `index` (`rows` appends). Missing cells are `na`.
    mutating func insertRow(at index: Int, _ values: [PineRuntimeValue]) {
        let row = (0..<columns).map { $0 < values.count ? values[$0] : .na }
        elements.insert(contentsOf: row, at: index * columns)
        rows += 1
    }

    mutating func insertColumn(at index: Int, _ values: [PineRuntimeValue]) {
        var next: [PineRuntimeValue] = []
        next.reserveCapacity((columns + 1) * rows)
        for r in 0..<rows {
            for c in 0...columns {
                if c == index {
                    next.append(r < values.count ? values[r] : .na)
                } else {
                    next.append(self[r, c < index ? c : c - 1])
                }
            }
        }
        elements = next
        columns += 1
    }

    mutating func removeRow(at index: Int) {
        elements.removeSubrange(index * columns..<(index + 1) * columns)
        rows -= 1
    }

    mutating func removeColumn(at index: Int) {
        var next: [PineRuntimeValue] = []
        for r in 0..<rows {
            for c in 0..<columns where c != index { next.append(self[r, c]) }
        }
        elements = next
        columns -= 1
    }

    mutating func fill(_ value: PineRuntimeValue) {
        elements = Array(repeating: value, count: elements.count)
    }

    func transposed() -> PineMatrix {
        var result = PineMatrix(rows: columns, columns: rows, initial: .na)
        for r in 0..<rows {
            for c in 0..<columns { result[c, r] = self[r, c] }
        }
        return result
    }
}
