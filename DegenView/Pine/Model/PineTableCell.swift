import Foundation

struct PineTableCell: Sendable, Equatable {
    var column: Int
    var row: Int
    var text: String
    var textColor: UInt32
    var backgroundColor: UInt32?
    var textSize: PineSize
}
