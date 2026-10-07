import SwiftUI

/// Chooses which webhook endpoints an existing Pine alert posts to.
struct PineAlertWebhooksSheet: View {
    let subscription: PineAlertSubscription
    @Environment(\.dismiss) private var dismiss
    @State private var selection: [UUID]

    init(subscription: PineAlertSubscription) {
        self.subscription = subscription
        _selection = State(initialValue: subscription.webhookEndpointIDs)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Webhooks for \(subscription.scriptName)").font(.headline)
            Text(
                "The alert's message is posted as is. Frequency and re-arming work exactly as they do for notifications."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            WebhookEndpointPicker(selection: $selection)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Save") {
                    PineAlertCoordinator.shared.setWebhooks(selection, for: subscription.id)
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 420)
    }
}
