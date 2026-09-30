import Foundation

struct PinePlotOutput: Sendable, Identifiable {
    let id: Int
    var title: String?
    var values: [Double?]
    var color: UInt32
    var lineWidth: Int
    var style: PinePlotStyle
    /// Per-bar colors, parallel to `values`. `nil` falls back to `color`.
    var colors: [UInt32?] = []
    /// `display.*` bit mask; without `PineDisplay.pane` the plot is kept (a `fill()` may
    /// reference it) but neither drawn nor counted for the pane's value axis.
    var display = PineDisplay.all
    /// Baseline of `style_histogram` / `style_columns` / `style_area` plots.
    var histBase = 0.0
}
