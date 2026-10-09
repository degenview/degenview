import Foundation

struct PineMarkerOutput: Sendable, Identifiable {
    let id: Int
    var kind: PineMarkerKind
    var title: String? = nil
    var values: [Bool]
    var character: String?
    var color: UInt32
    var location: PineMarkerLocation
    var style: PineMarkerShape
    /// Per-bar series value, parallel to `values`; used by `location.absolute`.
    var prices: [Double?] = []
    /// Per-bar colors, parallel to `values`.
    var colors: [UInt32?] = []
    var size = PineSize.auto
    var display = PineDisplay.all
}
