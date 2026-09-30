import Foundation

/// `chart.fg_color` / `chart.bg_color`, which follow the app's light or dark appearance.
struct PineChartTheme: Equatable, Hashable, Sendable {
    var foreground: UInt32
    var background: UInt32
    static let dark = PineChartTheme(foreground: 0xd1d4_dcff, background: 0x1317_22ff)
    static let light = PineChartTheme(foreground: 0x1317_22ff, background: 0xffff_ffff)
}
