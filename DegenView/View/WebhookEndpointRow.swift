import SwiftUI

/// One endpoint in Settings ▸ Webhooks: state dot, name, description, the redacted address, the
/// result of the last test, and Test / Edit / Enable / Delete.
struct WebhookEndpointRow: View {
    let endpoint: WebhookEndpoint
    /// Already redacted: this view never sees the full URL.
    let address: String
    /// Whether a secret is saved in the Keychain for this endpoint.
    let hasSecret: Bool
    let testState: WebhookTestState
    let onTest: () -> Void
    let onEdit: () -> Void
    let onToggle: () -> Void
    let onDelete: () -> Void

    var body: some View {
        SettingsCard {
            HStack(alignment: .center, spacing: 12) {
                Circle()
                    .fill(endpoint.isEnabled ? Color.green : Color.secondary.opacity(0.5))
                    .frame(width: 9, height: 9)
                    .help(endpoint.isEnabled ? "Enabled" : "Disabled: alerts skip this webhook")

                VStack(alignment: .leading, spacing: 3) {
                    Text(endpoint.name).font(.subheadline.weight(.semibold))
                    if !endpoint.description.isEmpty {
                        Text(endpoint.description).font(.caption).foregroundStyle(.secondary)
                    }
                    HStack(spacing: 6) {
                        Text(endpoint.method.rawValue)
                            .font(.caption2.weight(.bold).monospaced())
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Color.primary.opacity(0.08), in: RoundedRectangle(cornerRadius: 4))
                        Text(address)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        if hasSecret {
                            Image(systemName: "lock.fill")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .help("Uses a secret kept in your Keychain")
                                .accessibilityLabel("Uses a secret")
                        }
                        if !endpoint.headers.isEmpty {
                            Text(endpoint.headers.count == 1 ? "1 header" : "\(endpoint.headers.count) headers")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                }

                Spacer(minLength: 12)

                VStack(alignment: .trailing, spacing: 3) {
                    SettingsStatusBadge(text: testState.title, tone: testState.tone)
                    if let detail = testState.detail {
                        Text(detail).font(.caption).foregroundStyle(.secondary)
                    }
                }

                Button(action: onTest) {
                    if testState == .testing { ProgressView().controlSize(.small) } else { Text("Test") }
                }
                .disabled(testState == .testing)
                .help("Send the test payload to this webhook, even if it is disabled")

                Menu {
                    Button("Edit…", action: onEdit)
                    Button(endpoint.isEnabled ? "Disable" : "Enable", action: onToggle)
                    Divider()
                    Button("Delete…", role: .destructive, action: onDelete)
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .accessibilityLabel("More actions for \(endpoint.name)")
            }
        }
        .opacity(endpoint.isEnabled ? 1 : 0.75)
    }
}
