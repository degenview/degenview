import SwiftUI

/// Which webhook endpoints an alert posts to, and what it says. Shared by the price-alert and
/// Pine-alert editors: selectable cards for the endpoints, then (for price alerts) the message.
struct WebhookEndpointPicker: View {
    @Binding var selection: [UUID]
    /// The webhook body for price alerts. Nil hides the field: a Pine alert's message comes from its script.
    var message: Binding<String>?
    /// Example values for the message preview.
    var exampleContext: AlertMessageContext?
    /// The "Webhooks / N selected" title. A parent that shows its own heading turns it off.
    var showsHeader = true

    @StateObject private var store = WebhookEndpointStore.shared

    /// Endpoints shown before the list scrolls.
    private static let visibleRows = 4
    private static let rowSpacing: CGFloat = 6

    private var summary: WebhookPickerLogic.Summary {
        WebhookPickerLogic.summary(selection: selection, endpoints: store.endpoints)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if showsHeader { header }

            if store.endpoints.isEmpty {
                WebhookPickerEmptyState()
            } else {
                list
            }

            if summary.danglingCount > 0 {
                warning(
                    "\(summary.danglingCount) selected webhook\(summary.danglingCount == 1 ? " was" : "s were") "
                        + "deleted and will be skipped.")
            }
            if summary.allPaused {
                warning("Every selected webhook is turned off in Settings, so nothing will be sent.")
            }

            if let message, summary.selectedCount > 0 {
                WebhookMessageEditor(text: message, exampleContext: exampleContext)
                    .padding(.top, 4)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .animation(.easeOut(duration: 0.18), value: summary)
    }

    // MARK: - Pieces

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text("Webhooks").font(.subheadline.weight(.semibold))
            Spacer(minLength: 8)
            if summary.selectedCount > 0 {
                Text("\(summary.selectedCount) selected")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Color.accentColor)
                    .contentTransition(.numericText())
            } else {
                Text("Optional").font(.caption).foregroundStyle(.tertiary)
            }
        }
    }

    @ViewBuilder private var list: some View {
        let rows = VStack(spacing: Self.rowSpacing) {
            ForEach(store.endpoints) { endpoint in
                WebhookPickerRow(
                    endpoint: endpoint, hasSecret: store.hasSecret(endpoint.id),
                    isSelected: selection.contains(endpoint.id)
                ) { toggle(endpoint.id) }
            }
        }
        if store.endpoints.count > Self.visibleRows {
            let height =
                CGFloat(Self.visibleRows) * WebhookPickerRow.height
                + CGFloat(Self.visibleRows - 1) * Self.rowSpacing
            ScrollView(showsIndicators: false) { rows.padding(.bottom, 14) }
                .frame(height: height)
                .mask(
                    VStack(spacing: 0) {
                        Rectangle()
                        LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom)
                            .frame(height: 14)
                    })
        } else {
            rows
        }
    }

    private func warning(_ text: String) -> some View {
        Label(text, systemImage: "exclamationmark.triangle.fill")
            .font(.caption)
            .foregroundStyle(.orange)
            .fixedSize(horizontal: false, vertical: true)
            .transition(.opacity)
    }

    private func toggle(_ id: UUID) {
        withAnimation(.easeOut(duration: 0.15)) {
            if let index = selection.firstIndex(of: id) {
                selection.remove(at: index)
            } else {
                selection.append(id)
            }
        }
    }
}
