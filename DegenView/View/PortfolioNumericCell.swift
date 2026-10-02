import SwiftUI

/// A right-aligned, fixed-width-digit figure for a table cell.
struct PortfolioNumericCell: View {
    let text: String
    var color: Color = .primary

    var body: some View {
        Text(text)
            .monospacedDigit()
            .foregroundStyle(color)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .trailing)
    }
}
