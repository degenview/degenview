import SwiftUI

/// Shown in an alert editor when no webhook exists yet.
struct WebhookPickerEmptyState: View {
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "arrow.up.forward.app")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 30, height: 30)
                .background(Color.secondary.opacity(0.14), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text("No webhooks yet").font(.subheadline.weight(.semibold))
                Text("Send alerts to your own server or bot.").font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            SettingsLink { Text("Add in Settings") }
                .controlSize(.small)
        }
        .padding(.horizontal, 12)
        .frame(height: WebhookPickerRow.height + 8)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .accessibilityElement(children: .combine)
    }
}
