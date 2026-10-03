import SwiftUI

/// Buy / Sell as two full-width buttons; the chosen side fills with its colour, so the ticket's
/// direction is clear before anything else on it is read.
struct PaperSideSelector: View {
    @Binding var selection: PaperOrderSide

    var body: some View {
        HStack(spacing: 2) {
            ForEach(PaperOrderSide.allCases, id: \.self) { side in
                segment(side)
            }
        }
        .padding(3)
        .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 11, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Order side")
    }

    private func segment(_ side: PaperOrderSide) -> some View {
        let isSelected = selection == side
        return Button {
            withAnimation(.easeOut(duration: 0.15)) { selection = side }
        } label: {
            Text(side.label)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(isSelected ? Color.white : Color.secondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
                .background {
                    if isSelected {
                        RoundedRectangle(cornerRadius: 8, style: .continuous).fill(side.tint.opacity(0.92))
                    }
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(side.label)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
