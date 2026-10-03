import SwiftUI

/// A count in a small tinted pill: "2 errors", "3 live".
struct PineCountChip: View {
    let text: String
    let tint: Color

    /// "2 errors", "1 warning": `noun` gets an "s" unless the count is 1.
    static func plural(_ count: Int, _ noun: String) -> String { "\(count) \(noun)\(count == 1 ? "" : "s")" }

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(tint.opacity(0.14), in: Capsule())
            .fixedSize()
    }
}
