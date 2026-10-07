import SwiftUI

/// Settings ▸ Webhooks: the reusable endpoints that price alerts and Pine alerts can post to.
struct WebhookSettingsView: View {
    @StateObject private var store = WebhookEndpointStore.shared
    @State private var editing: EditorTarget?
    @State private var pendingDelete: WebhookEndpoint?
    @State private var testStates: [UUID: WebhookTestState] = [:]
    @State private var testTasks: [UUID: Task<Void, Never>] = [:]
    @State private var testPayload = WebhookSettingsView.defaultTestPayload
    @State private var failure: String?

    static let defaultTestPayload = #"{"event":"DegenView webhook test"}"#

    private struct EditorTarget: Identifiable {
        let endpoint: WebhookEndpoint?
        var id: UUID { endpoint?.id ?? UUID(uuidString: "00000000-0000-0000-0000-000000000000")! }
    }

    var body: some View {
        SettingsPage(
            systemImage: "arrow.up.forward.app.fill", title: "Webhooks",
            subtitle: "Send alert messages to external HTTP endpoints, such as a trading bot or a chat channel.",
            content: {
                if store.loadFailed {
                    NoticeCard(
                        systemImage: "exclamationmark.triangle.fill", tint: .orange,
                        title: "Saved webhooks could not be read",
                        detail: "Changes are turned off so nothing is overwritten. Restart DegenView to try again.")
                }

                SettingsSection(
                    title: "Endpoints",
                    subtitle: "Each alert chooses which of these it posts to. Turning one off pauses it everywhere."
                ) {
                    if store.endpoints.isEmpty {
                        SettingsCard {
                            Text("No webhooks yet. Add one to start sending alerts to your own server.")
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                    }
                    ForEach(store.endpoints) { endpoint in
                        WebhookEndpointRow(
                            endpoint: endpoint,
                            address: store.address(for: endpoint.id),
                            hasSecret: store.hasSecret(endpoint.id),
                            testState: testStates[endpoint.id] ?? .notTested,
                            onTest: { test(endpoint) },
                            onEdit: { editing = EditorTarget(endpoint: endpoint) },
                            onToggle: { toggle(endpoint) },
                            onDelete: { pendingDelete = endpoint })
                    }
                }

                SettingsSection(
                    title: "Test message",
                    subtitle:
                        "Sent by each Test button Valid JSON goes out as application/json, anything else as text/plain."
                ) {
                    SettingsCard {
                        TextField("Message", text: $testPayload, axis: .vertical)
                            .textFieldStyle(.plain)
                            .lineLimit(1...4)
                            .font(.body.monospaced())
                    }
                }

                SettingsSection(title: "How it works") {
                    Text(
                        "Alerts send their final message as the request body. "
                            + "A secret is kept in your Keychain and added to the URL or a header where you place it. "
                            + "Each alert triggers one attempt per webhook, with a 3 second limit and no retries, "
                            + "so a slow or failing server can never send the same message twice."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            },
            footer: {
                HStack {
                    if let failure { SettingsStatusMessage(text: failure, isError: true) }
                    Spacer()
                    Button {
                        editing = EditorTarget(endpoint: nil)
                    } label: {
                        Label("Add Webhook", systemImage: "plus")
                    }
                    .disabled(store.loadFailed)
                }
            }
        )
        .sheet(item: $editing) { target in
            WebhookEndpointEditorSheet(existing: target.endpoint, store: store)
        }
        .confirmationDialog(
            "Delete “\(pendingDelete?.name ?? "")”?", isPresented: deleteBinding, titleVisibility: .visible,
            presenting: pendingDelete
        ) { endpoint in
            Button("Delete", role: .destructive) { delete(endpoint) }
        } message: { endpoint in
            Text(deleteMessage(for: endpoint))
        }
        .onDisappear { testTasks.values.forEach { $0.cancel() } }
    }

    private var deleteBinding: Binding<Bool> {
        Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } })
    }

    private func deleteMessage(for endpoint: WebhookEndpoint) -> String {
        let count = store.referenceCount(to: endpoint.id)
        guard count > 0 else { return "This webhook isn’t used by any alert." }
        return
            "This webhook is currently used by \(count) \(count == 1 ? "alert" : "alerts"). "
            + "Deleting it will stop webhook delivery for \(count == 1 ? "that alert" : "those alerts")."
    }

    private func delete(_ endpoint: WebhookEndpoint) {
        do {
            try store.delete(id: endpoint.id)
            testStates[endpoint.id] = nil
            failure = nil
        } catch {
            failure = error.localizedDescription
        }
    }

    private func toggle(_ endpoint: WebhookEndpoint) {
        do {
            try store.setEnabled(!endpoint.isEnabled, for: endpoint.id)
            failure = nil
        } catch {
            failure = error.localizedDescription
        }
    }

    /// Uses the production service in `.test` mode: the same validation, request and timeout as an
    /// alert, but a disabled endpoint can still be tried.
    private func test(_ endpoint: WebhookEndpoint) {
        testTasks[endpoint.id]?.cancel()
        testStates[endpoint.id] = .testing
        let message = testPayload
        testTasks[endpoint.id] = Task {
            let result = await WebhookDeliveryService.shared.deliver(message: message, to: endpoint, mode: .test)
            guard !Task.isCancelled else { return }
            testStates[endpoint.id] = WebhookTestState(result)
        }
    }
}
