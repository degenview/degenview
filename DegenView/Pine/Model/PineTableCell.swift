import Foundation

struct PineTableCell: Sendable, Equatable {
    var column: Int
    var row: Int
    var text: String
    var textColor: UInt32
    var backgroundColor: UInt32?
    var textSize: PineSize
    /// Columns and rows the cell covers after `table.merge_cells`.
    var columnSpan = 1
    var rowSpan = 1
}
