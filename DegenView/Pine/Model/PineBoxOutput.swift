import Foundation

struct PineBoxOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var left: Int
    var top: Double
    var right: Int
    var bottom: Double
    var borderColor: UInt32?
    var borderWidth: Int
    var backgroundColor: UInt32?
    var isComplete: Bool {
        PineDrawingCoordinate.isKnown(left) && PineDrawingCoordinate.isKnown(right)
            && PineDrawingCoordinate.isKnown(top) && PineDrawingCoordinate.isKnown(bottom)
    }
    var borderStyle: PineLineStyle = .solid
    /// Text inside the box (`text =` or `box.set_text`). Wrap, font family and formatting are not modelled.
    var text = ""
    var textColor: UInt32 = 0x0000_00ff
    var textSize: PineSize = .normal
    var textHorizontalAlign: PineTextAlign = .center
    var textVerticalAlign: PineTextAlign = .center
    /// Made with `xloc.bar_time`: setters take times, which are mapped to bar indexes.
    var timeAnchored = false
}
