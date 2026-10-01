import Foundation

/// Drawings can be made with `na` coordinates (`line.new(na, na, na, na)`) and given real ones later; until every
/// coordinate is known they exist but are not drawn. A missing bar index is `missingIndex`, a missing price `NaN`.
enum PineDrawingCoordinate {
    static let missingIndex = Int.min

    static func isKnown(_ index: Int) -> Bool { index != missingIndex }
    static func isKnown(_ price: Double) -> Bool { price.isFinite }
}
