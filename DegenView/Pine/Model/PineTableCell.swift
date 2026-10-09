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
    var textHorizontalAlign: PineTextAlign = .center
    var textVerticalAlign: PineTextAlign = .center
    /// Minimum size as a percentage of the plot's width and height; 0 sizes the column or row to its text.
    var width: Double = 0
    var height: Double = 0
}
