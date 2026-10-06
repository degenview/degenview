import SwiftUI

/// The one "PAPER" marker — on the panel, the ticket and the account sheets — so simulated money is
/// never mistaken for the real thing.
struct PaperBadge: View {
    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "doc.text.fill").font(.system(size: 9, weight: .bold))
            Text("PAPER").font(.system(size: 10.5, weight: .bold)).tracking(0.4)
        }
        .foregroundStyle(Color.accentColor)
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(Color.accentColor.opacity(0.14), in: Capsule())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Paper trading")
    }
}
