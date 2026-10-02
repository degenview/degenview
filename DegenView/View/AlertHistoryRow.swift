import SwiftUI

/// One trigger as a card: coin and source logo, what fired, the price it fired at against the target, and when.
struct AlertHistoryRow: View {
    let event: AlertTriggerEvent
    @ObservedObject var info: PortfolioAssetInfoViewModel
    /// The rule that fired, while it still exists; its condition names the direction exactly.
    let rule: PriceAlert?
    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 12) {
            AlertAssetIcon(asset: event.asset, info: info)
            identity
            Spacer(minLength: 8)
            if event.delivery.state == .failed {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .help(event.delivery.error ?? "The notification could not be delivered.")
                    .accessibilityLabel("Notification failed")
            }
            chip
            priceColumn.frame(width: 132, alignment: .trailing)
            timeColumn.frame(width: 92, alignment: .trailing)
        }
        .alertCard(isHovered: $isHovered)
        .accessibilityElement(children: .combine)
    }

    private var identity: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(event.asset.displayTicker).font(.headline).lineLimit(1)
                if event.origin == .catchUp {
                    Label("Delayed", systemImage: "clock.badge.exclamationmark")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.orange)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(Color.orange.opacity(0.14), in: Capsule())
                        .help("Seen late: the price crossed while DegenView was not watching.")
                }
            }
            Text("\(info.subtitle(for: event.asset)) · \(event.asset.source.displayName)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    @ViewBuilder private var chip: some View {
        if let rule {
            AlertConditionChip(condition: rule.condition)
        } else {
            AlertConditionChip(observed: event.observedValue, target: event.target)
        }
    }

    private var priceColumn: some View {
        VStack(alignment: .trailing, spacing: 2) {
            Text(event.currency.formatAlertPrice(event.observedValue))
                .font(.body.weight(.semibold).monospacedDigit())
                .lineLimit(1)
            Text("Target \(event.currency.formatAlertPrice(event.target))")
                .font(.caption)
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .lineLimit(1)
        }
    }

    private var timeColumn: some View {
        VStack(alignment: .trailing, spacing: 2) {
            Text(event.timestamp.formatted(date: .omitted, time: .shortened)).monospacedDigit()
            Text(event.timestamp.formatted(.relative(presentation: .numeric, unitsStyle: .abbreviated)))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}
