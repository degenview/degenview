import SwiftUI

/// One marker on the chart: a dashed rule across the plot ending in a pill — a tinted tag (LONG,
/// TP, LMT), the size and price, and optionally the open P&L — with a close / cancel button.
///
/// The pill sits on a material rather than a solid colour, so its text is `primary` and stays
/// legible over any candle colour in light and dark themes.
struct PaperChartMarkerPill: View {
    let tag: String
    let tint: Color
    let text: String
    var pnlText: String?
    var pnlColor: Color = .secondary
    let closeLabel: String
    let onClose: () -> Void
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 0) {
            DashedRule()
                .stroke(tint.opacity(0.7), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                .frame(height: 1)
            pill
        }
        .onHover { isHovering = $0 }
    }

    private var pill: some View {
        HStack(spacing: 0) {
            Text(tag)
                .font(.system(size: 10, weight: .bold))
                .tracking(0.3)
                .foregroundStyle(tint)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(tint.opacity(0.18))
            HStack(spacing: 6) {
                Text(text).font(.caption2.monospacedDigit()).foregroundStyle(.primary)
                if let pnlText {
                    Text(pnlText).font(.caption2.weight(.semibold).monospacedDigit()).foregroundStyle(pnlColor)
                }
                Button(action: onClose) {
                    Image(systemName: "xmark").font(.system(size: 8, weight: .bold))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .opacity(isHovering ? 1 : 0.55)
                .accessibilityLabel(closeLabel)
                .help(closeLabel)
            }
            .padding(.horizontal, 7)
        }
        .lineLimit(1)
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(tint.opacity(0.8)))
    }

    private struct DashedRule: Shape {
        func path(in rect: CGRect) -> Path {
            Path { path in
                path.move(to: CGPoint(x: rect.minX, y: rect.midY))
                path.addLine(to: CGPoint(x: rect.maxX, y: rect.midY))
            }
        }
    }
}
