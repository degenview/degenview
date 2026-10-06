import SwiftUI

/// The colours Paper Trading draws with, in one place: buy/sell, profit/loss, and the margin-buffer
/// warning ramp. Everything is a system colour, so light and dark themes both stay legible.
enum PaperTradingStyle {
    static let buy = Color.green
    static let sell = Color.red

    /// Green above zero, red below, quiet at zero.
    static func pnl(_ value: Decimal) -> Color {
        if value > 0 { return .green }
        if value < 0 { return .red }
        return .secondary
    }

    /// Margin buffer is available funds over equity: plenty is quiet, thin is orange, nearly out is red.
    static func marginBuffer(_ ratio: Decimal) -> Color {
        if ratio < Decimal(string: "0.1")! { return .red }
        if ratio < Decimal(string: "0.25")! { return .orange }
        return .primary
    }
}
