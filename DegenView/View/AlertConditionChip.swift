import SwiftUI

/// What an alert waits for, as a small tinted pill: "Crosses above", "Falls 5%".
struct AlertConditionChip: View {
    let systemImage: String
    let tint: Color
    let label: String

    init(condition: AlertCondition) {
        switch condition {
        case .crossesAbove: self.init(systemImage: "arrow.up.right", tint: .green, label: "Crosses above")
        case .crossesBelow: self.init(systemImage: "arrow.down.right", tint: .red, label: "Crosses below")
        case .risesBy(let percent, _, _):
            self.init(systemImage: "arrow.up", tint: .green, label: "Rises \(Self.percent(percent))")
        case .fallsBy(let percent, _, _):
            self.init(systemImage: "arrow.down", tint: .red, label: "Falls \(Self.percent(percent))")
        case .unsupported: self.init(systemImage: "questionmark", tint: .secondary, label: "Unsupported")
        }
    }

    /// A trigger record carries no condition, so a deleted rule's direction is read off the prices.
    init(observed: Decimal, target: Decimal) {
        if observed >= target {
            self.init(systemImage: "arrow.up.right", tint: .green, label: "Crossed above")
        } else {
            self.init(systemImage: "arrow.down.right", tint: .red, label: "Crossed below")
        }
    }

    private init(systemImage: String, tint: Color, label: String) {
        self.systemImage = systemImage
        self.tint = tint
        self.label = label
    }

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: systemImage).font(.system(size: 10, weight: .bold))
            Text(label).font(.caption.weight(.medium)).lineLimit(1)
        }
        .foregroundStyle(tint)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(tint.opacity(0.14), in: Capsule())
        .fixedSize()
    }

    private static func percent(_ value: Decimal) -> String {
        value.formatted(.number.precision(.fractionLength(0...2))) + "%"
    }
}
