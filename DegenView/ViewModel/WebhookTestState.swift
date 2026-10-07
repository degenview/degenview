import Foundation

/// What the Settings page shows for a webhook's Test button, derived from a typed delivery result.
/// Wording lives here, never in the domain types.
enum WebhookTestState: Equatable {
    case notTested
    case testing
    case delivered(statusCode: Int?, duration: TimeInterval)
    case failed(WebhookDeliveryError, statusCode: Int?, duration: TimeInterval)
    case timedOut(duration: TimeInterval)

    init(_ result: WebhookDeliveryResult) {
        switch result.error {
        case nil: self = .delivered(statusCode: result.statusCode, duration: result.duration)
        case .timeout?: self = .timedOut(duration: result.duration)
        case let error?: self = .failed(error, statusCode: result.statusCode, duration: result.duration)
        }
    }

    var title: String {
        switch self {
        case .notTested: "Not tested"
        case .testing: "Testing…"
        case .delivered: "Delivered"
        case .failed: "Failed"
        case .timedOut: "Timed out"
        }
    }

    /// `HTTP 200 · 143 ms`, `3.0 s`, or a short reason. Never a raw system error.
    var detail: String? {
        switch self {
        case .notTested, .testing:
            return nil
        case .delivered(let status, let duration):
            return Self.join(status.map { "HTTP \($0)" }, Self.milliseconds(duration))
        case .timedOut(let duration):
            return String(format: "%.1f s", duration)
        case .failed(let error, let status, let duration):
            switch error {
            case .httpFailure:
                return Self.join(status.map { "HTTP \($0)" }, Self.milliseconds(duration))
            case .networkFailure: return "Could not connect"
            case .invalidURL, .unsupportedScheme, .unsupportedPort: return "Invalid address"
            case .secretUnavailable: return "Secret not found in Keychain"
            case .invalidHeader: return "Invalid header"
            case .secretInHost: return "Secret can't be in the host"
            case .missingEndpoint: return "Webhook not found"
            case .disabledEndpoint: return "Webhook is off"
            case .timeout: return String(format: "%.1f s", duration)
            }
        }
    }

    var tone: SettingsStatusBadge.Tone {
        switch self {
        case .notTested, .testing: .neutral
        case .delivered: .good
        case .failed, .timedOut: .bad
        }
    }

    private static func milliseconds(_ duration: TimeInterval) -> String {
        "\(Int((duration * 1000).rounded())) ms"
    }

    private static func join(_ parts: String?...) -> String {
        parts.compactMap { $0 }.joined(separator: " · ")
    }
}
