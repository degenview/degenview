import Foundation

/// The compact "Webhooks 2/3 delivered" line for one alert trigger. The trigger itself is a
/// separate fact: a failed webhook never makes the alert a failed alert.
struct WebhookDeliverySummary: Equatable {
    let records: [WebhookDeliveryRecord]

    var attempted: Int { records.count }
    var delivered: Int { records.filter { $0.state == .delivered }.count }
    var failed: Int { records.filter { $0.state == .failed }.count }
    var pending: Int { records.filter { $0.state == .pending }.count }
    var isEmpty: Bool { records.isEmpty }

    var title: String {
        if pending > 0 && delivered + failed == 0 { return "Webhooks sending…" }
        return "Webhooks \(delivered)/\(attempted) delivered"
    }

    var tone: SettingsStatusBadge.Tone {
        if pending > 0 { return .neutral }
        if failed == 0 { return .good }
        return delivered == 0 ? .bad : .warning
    }

    /// `HTTP 200 · 143 ms`, a short reason, or `Sending…`, for one endpoint's row.
    static func detail(for record: WebhookDeliveryRecord) -> String {
        switch record.state {
        case .pending:
            return "Sending…"
        case .delivered, .failed:
            let result = WebhookDeliveryResult(
                endpointID: record.endpointID, timestamp: record.timestamp, duration: record.duration ?? 0,
                statusCode: record.statusCode, error: record.state == .failed ? (record.error ?? .networkFailure) : nil)
            return WebhookTestState(result).detail ?? WebhookTestState(result).title
        }
    }
}
