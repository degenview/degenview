import SwiftUI

/// "Send to webhooks" in an alert editor: off by default, so the editor stays short for the many alerts
/// that never post anywhere. Turning it on reveals the endpoint picker (and, for price alerts, the message).
struct WebhookSendSection: View {
    @Binding var isOn: Bool
    @Binding var selection: [UUID]
    /// The webhook body for price alerts. Nil hides the field: a script alert's message comes from its script.
    var message: Binding<String>?
    var exampleContext: AlertMessageContext?

    @StateObject private var store = WebhookEndpointStore.shared

    private var subtitle: String {
        guard isOn else { return "Also post this alert to your own server or bot." }
        let summary = WebhookPickerLogic.summary(selection: selection, endpoints: store.endpoints)
        if summary.selectedCount > 0 { return "\(summary.selectedCount) selected" }
        return store.endpoints.isEmpty ? "Add a webhook to choose where to post." : "Choose where to post."
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            toggleCard
            if isOn {
                WebhookEndpointPicker(
                    selection: $selection, message: message, exampleContext: exampleContext, showsHeader: false
                )
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .animation(.easeOut(duration: 0.18), value: isOn)
    }

    private var toggleCard: some View {
        HStack(spacing: 12) {
            Image(systemName: "arrow.up.forward.app")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(isOn ? Color.accentColor : .secondary)
                .frame(width: 30, height: 30)
                .background(
                    (isOn ? Color.accentColor : Color.secondary).opacity(0.14),
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                )
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("Send to webhooks").font(.subheadline.weight(.semibold))
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(isOn && subtitle.hasSuffix("selected") ? Color.accentColor : .secondary)
                    .contentTransition(.opacity)
            }
            Spacer(minLength: 8)
            Toggle("Send to webhooks", isOn: $isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .onTapGesture { isOn.toggle() }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}
