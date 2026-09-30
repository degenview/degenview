import Foundation

struct PineTableOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var position: PineTablePosition
    var columns: Int
    var rows: Int
    var backgroundColor: UInt32?
    var borderColor: UInt32?
    var borderWidth: Int
    var frameColor: UInt32?
    var frameWidth: Int
    var cells: [PineTableCell] = []
}
