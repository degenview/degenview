import Foundation

/// `display.*`: where a plot is shown. Scripts do arithmetic on these, so the builtin
/// constants are the raw integers.
struct PineDisplay: OptionSet, Sendable {
    let rawValue: Int

    static let pane = PineDisplay(rawValue: 1)
    static let dataWindow = PineDisplay(rawValue: 2)
    static let priceScale = PineDisplay(rawValue: 4)
    static let statusLine = PineDisplay(rawValue: 8)
    static let hidden: PineDisplay = []
    static let all: PineDisplay = [.pane, .dataWindow, .priceScale, .statusLine]
}
