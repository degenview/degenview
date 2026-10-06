import SwiftUI

/// The alerts a script raised while it ran, in the same collapsible box as its diagnostics: how
/// many fired on live bars (the ones that notify) and how many on history, in the header; newest
/// first when open. Starts closed — it is a log, not a problem to fix.
struct PineAlertsSection: View {
    let alerts: [PineAlertEvent]
    @State private var isExpanded = false

    /// The most rows listed; older alerts are summarised, not drawn.
    static let limit = 100

    struct Summary: Equatable {
        var live = 0
        var historical = 0
    }

    static func summary(for alerts: [PineAlertEvent]) -> Summary {
        alerts.reduce(into: Summary()) { summary, alert in
            if alert.isRealtime { summary.live += 1 } else { summary.historical += 1 }
        }
    }

    /// The latest `limit` alerts, newest first.
    static func visible(_ alerts: [PineAlertEvent]) -> [PineAlertEvent] {
        Array(alerts.suffix(limit).reversed())
    }

    var body: some View {
        let summary = Self.summary(for: alerts)
        DisclosureGroup(isExpanded: $isExpanded) {
            VStack(alignment: .leading, spacing: 6) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Self.visible(alerts)) { row($0) }
                    }
                }
                .frame(maxHeight: 150)
                if alerts.count > Self.limit {
                    Text("Showing the latest \(Self.limit) of \(alerts.count)")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.top, 6)
        } label: {
            header(summary)
        }
        .font(.caption)
        .animation(.snappy, value: isExpanded)
        .pineReportBox()
    }

    // MARK: - Pieces

    private func header(_ summary: Summary) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "bell.badge.fill").foregroundStyle(Color.accentColor)
            Text("Alerts").font(.caption.weight(.semibold))
            if summary.live > 0 { PineCountChip(text: "\(summary.live) live", tint: .green) }
            if summary.historical > 0 {
                PineCountChip(text: "\(summary.historical) historical", tint: .secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilitySummary(summary))
    }

    private func row(_ alert: PineAlertEvent) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: alert.isRealtime ? "bell.fill" : "clock")
                .font(.system(size: 10))
                .foregroundStyle(alert.isRealtime ? Color.accentColor : .secondary)
                .frame(width: 14)
                .accessibilityHidden(true)
            Text(alert.message.isEmpty ? "No message" : alert.message)
                .foregroundStyle(alert.message.isEmpty ? .secondary : .primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            Text(alert.time.formatted(.dateTime.month(.abbreviated).day().hour().minute()))
                .font(.caption2.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .font(.caption)
        .help(
            "\(alert.isRealtime ? "Live bar, notifies" : "Historical bar, never notifies") · "
                + alert.frequency.displayName)
    }

    private func accessibilitySummary(_ summary: Summary) -> String {
        var parts = ["Alerts"]
        if summary.live > 0 { parts.append("\(summary.live) live") }
        if summary.historical > 0 { parts.append("\(summary.historical) historical") }
        return parts.joined(separator: ", ")
    }
}
