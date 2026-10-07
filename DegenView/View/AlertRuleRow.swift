import SwiftUI

/// One price alert as a card: coin and source logo, the condition, its target and how far the market is from it,
/// the state, and the actions on hover.
struct AlertRuleRow: View {
    let alert: PriceAlert
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let quote: MarketQuote?
    /// The newest trigger of this rule, shown on a triggered card.
    let lastFired: AlertTriggerEvent?
    let onEdit: () -> Void
    let onToggle: () -> Void
    let onDuplicate: () -> Void
    let onDelete: () -> Void
    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 12) {
            AlertAssetIcon(asset: alert.asset, info: info)
            identity
            Spacer(minLength: 8)
            AlertConditionChip(condition: alert.condition)
            targetColumn.frame(width: 132, alignment: .trailing)
            SettingsStatusBadge(text: status.text, tone: status.tone)
                .help(status.help)
                .frame(width: 136, alignment: .trailing)
            actions
        }
        .alertCard(isHovered: $isHovered)
        .opacity(alert.state == .paused ? 0.7 : 1)
        .onTapGesture(count: 2, perform: onEdit)
        .contextMenu { menuItems }
        .accessibilityElement(children: .contain)
    }

    private var identity: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(alert.asset.displayTicker).font(.headline).lineLimit(1)
                Image(systemName: alert.frequency == .once ? "1.circle" : "repeat")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .help(alert.frequency == .once ? "Triggers once" : "Triggers every time")
                if !alert.webhookEndpointIDs.isEmpty {
                    let count = alert.webhookEndpointIDs.count
                    Image(systemName: "arrow.up.forward.app")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .help("Posts to \(count) webhook\(count == 1 ? "" : "s")")
                }
            }
            Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
    }

    private var detail: String {
        alert.note.isEmpty ? "\(info.subtitle(for: alert.asset)) · \(alert.asset.source.displayName)" : alert.note
    }

    private var targetColumn: some View {
        VStack(alignment: .trailing, spacing: 2) {
            Text(alert.condition.target.map { alert.currency.formatAlertPrice($0) } ?? "—")
                .font(.body.weight(.semibold).monospacedDigit())
                .lineLimit(1)
            if let secondary {
                Text(secondary).font(.caption).foregroundStyle(.secondary).monospacedDigit().lineLimit(1)
            }
        }
        .help(helpText)
    }

    /// "0.8% away" for a live rule, "Fired 3 hr. ago" once it has.
    private var secondary: String? {
        if alert.state == .triggered, let lastFired {
            return "Fired " + lastFired.timestamp.formatted(.relative(presentation: .numeric, unitsStyle: .abbreviated))
        }
        guard alert.state == .active, let price = livePrice, let target = alert.condition.target, price > 0 else {
            return nil
        }
        let away = abs(target - price) / price * 100
        return away.formatted(.number.precision(.fractionLength(away < 10 ? 1 : 0))) + "% away"
    }

    /// The market price, when it is fresh and already in the alert's currency.
    private var livePrice: Decimal? {
        guard let quote, quote.isFresh, quote.currency == alert.currency else { return nil }
        return quote.price
    }

    private var helpText: String {
        livePrice.map { "Now \(alert.currency.formatAlertPrice($0))" } ?? ""
    }

    private var status: (text: String, tone: SettingsStatusBadge.Tone, help: String) {
        switch alert.state {
        case .active where !alert.armed:
            ("Re-arming", .warning, "Waiting for the price to move back across the level before it can fire again.")
        case .active where quote?.isFresh != true:
            ("Waiting for data", .warning, "Waiting for current market data.")
        case .active: ("Active", .good, "Watching the market.")
        case .paused: ("Paused", .neutral, "Paused. It will not fire until resumed.")
        case .triggered: ("Triggered", .warning, "Fired. Re-enable it to watch again.")
        case .unsupported: ("Unsupported", .bad, "This rule type is not supported by this version.")
        }
    }

    private var actions: some View {
        HStack(spacing: 2) {
            quickButton("pencil", label: "Edit", action: onEdit)
            quickButton(toggleIcon, label: toggleTitle, action: onToggle).disabled(alert.state == .unsupported)
            Menu {
                menuItems
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .semibold))
                    .frame(width: 26, height: 26)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("More actions")
        }
    }

    private func quickButton(_ systemImage: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 12, weight: .medium))
                .frame(width: 26, height: 26)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .opacity(isHovered ? 1 : 0)
        .help(label)
        .accessibilityLabel(label)
    }

    private var toggleTitle: String {
        switch alert.state {
        case .active: "Pause"
        case .triggered: "Re-enable"
        default: "Resume"
        }
    }

    private var toggleIcon: String { alert.state == .active ? "pause.fill" : "play.fill" }

    @ViewBuilder private var menuItems: some View {
        Button("Edit…", systemImage: "pencil", action: onEdit)
        Button(toggleTitle, systemImage: toggleIcon, action: onToggle).disabled(alert.state == .unsupported)
        Button("Duplicate", systemImage: "plus.square.on.square", action: onDuplicate)
        Divider()
        Button("Delete…", systemImage: "trash", role: .destructive, action: onDelete)
    }
}
