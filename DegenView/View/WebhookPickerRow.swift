import SwiftUI

/// One webhook in an alert editor's picker: a compact selectable card in the same language as
/// `ChoiceCard`. A webhook that is switched off in Settings can still be picked; it is dimmed and says so.
struct WebhookPickerRow: View {
    static let height: CGFloat = 48

    let endpoint: WebhookEndpoint
    let hasSecret: Bool
    let isSelected: Bool
    let action: () -> Void

    @State private var isHovered = false

    private var fill: Color {
        if isSelected { return Color.accentColor.opacity(0.08) }
        return isHovered ? Color.primary.opacity(0.06) : Color.primary.opacity(0.025)
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: "arrow.up.forward.app")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary)
                    .frame(width: 30, height: 30)
                    .background(
                        (isSelected ? Color.accentColor : Color.secondary).opacity(0.14),
                        in: RoundedRectangle(cornerRadius: 8, style: .continuous))

                VStack(alignment: .leading, spacing: 1) {
                    Text(endpoint.name)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                        .truncationMode(.tail)
                    HStack(spacing: 4) {
                        if hasSecret {
                            Image(systemName: "lock.fill").font(.system(size: 8))
                                .accessibilityHidden(true)
                        }
                        Text(WebhookPickerLogic.subtitle(for: endpoint))
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                Spacer(minLength: 8)

                if !endpoint.isEnabled {
                    Text("Off")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(Color.primary.opacity(0.08), in: Capsule())
                }
                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 16))
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary.opacity(0.6))
            }
            .padding(.horizontal, 12)
            .frame(height: Self.height)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(fill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(
                        isSelected ? Color.accentColor : Color.primary.opacity(0.12), lineWidth: isSelected ? 1.5 : 1)
            )
            .opacity(endpoint.isEnabled ? 1 : 0.65)
            .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .help(endpoint.isEnabled ? "" : "Turned off in Settings: skipped until you turn it back on")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(endpoint.name), \(WebhookPickerLogic.subtitle(for: endpoint))")
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
}
