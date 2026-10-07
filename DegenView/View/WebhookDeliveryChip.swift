import SwiftUI

/// A small "Webhooks 2/3 delivered" chip for an alert's history row; click for each endpoint's result.
/// Shown only when the alert actually posted somewhere.
struct WebhookDeliveryChip: View {
    let eventID: UUID
    @State private var summary = WebhookDeliverySummary(records: [])
    @State private var showsDetail = false
    @StateObject private var endpoints = WebhookEndpointStore.shared

    var body: some View {
        Group {
            if !summary.isEmpty {
                Button {
                    showsDetail.toggle()
                } label: {
                    HStack(spacing: 5) {
                        Image(systemName: "arrow.up.forward.app").font(.system(size: 10, weight: .bold))
                        Text(summary.title).font(.caption.weight(.medium))
                    }
                    .foregroundStyle(summary.tone.color)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(summary.tone.color.opacity(0.12), in: Capsule())
                }
                .buttonStyle(.plain)
                .popover(isPresented: $showsDetail, arrowEdge: .bottom) { detail }
                .accessibilityLabel(summary.title)
            }
        }
        .task(id: eventID) { await load() }
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Webhook deliveries").font(.headline)
            ForEach(summary.records) { record in
                HStack(spacing: 8) {
                    Image(systemName: icon(record)).foregroundStyle(color(record))
                    Text(endpoints.endpoint(id: record.endpointID)?.name ?? "Deleted webhook")
                    Spacer(minLength: 12)
                    Text(WebhookDeliverySummary.detail(for: record)).foregroundStyle(.secondary).monospacedDigit()
                }
                .font(.callout)
            }
            Text("The alert fired either way; each webhook is attempted once, with no retries.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(14)
        .frame(minWidth: 280)
    }

    private func icon(_ record: WebhookDeliveryRecord) -> String {
        switch record.state {
        case .delivered: "checkmark.circle.fill"
        case .failed: "xmark.circle.fill"
        case .pending: "clock"
        }
    }

    private func color(_ record: WebhookDeliveryRecord) -> Color {
        switch record.state {
        case .delivered: .green
        case .failed: .red
        case .pending: .secondary
        }
    }

    /// Deliveries finish a few seconds after the trigger, so a fresh row checks back briefly.
    private func load() async {
        for attempt in 0..<6 {
            let records = AppDatabase.shared.webhookDeliveries(eventIDs: [eventID])[eventID] ?? []
            summary = WebhookDeliverySummary(records: records)
            let settled = !records.isEmpty && summary.pending == 0
            if settled || attempt == 5 { return }
            try? await Task.sleep(for: .seconds(2))
            if Task.isCancelled { return }
        }
    }
}
