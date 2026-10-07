import Foundation

/// Why a webhook delivery did not succeed. Domain state only: UI wording is derived elsewhere.
enum WebhookDeliveryError: String, Codable, Error, Equatable, Sendable {
    case invalidURL
    case unsupportedScheme
    case unsupportedPort
    case disabledEndpoint
    case missingEndpoint
    /// The URL or a header uses `{{secret}}` but the Keychain has no readable secret for it.
    case secretUnavailable
    /// A header name or value is not allowed, including after the secret is substituted in.
    case invalidHeader
    /// `{{secret}}` was placed in the host part of the URL.
    case secretInHost
    case timeout
    case networkFailure
    /// The server answered with a status outside 200...299. Redirects land here too: they are
    /// never followed.
    case httpFailure
}

/// What triggered a delivery. Part of the history key, so one trigger reaches an endpoint once.
enum WebhookDeliverySource: String, Codable, Equatable, Sendable {
    case price
    case pine
    case test
}

/// The outcome of one attempt at one endpoint.
struct WebhookDeliveryResult: Equatable, Sendable {
    let endpointID: UUID
    let timestamp: Date
    let duration: TimeInterval
    let statusCode: Int?
    let error: WebhookDeliveryError?

    var succeeded: Bool { error == nil }
}

enum WebhookDeliveryState: String, Codable, Equatable, Sendable {
    /// Claimed but not finished. A row that stays here was interrupted (the process quit
    /// mid-request); it is never retried, because the request may have reached the server.
    case pending
    case delivered
    case failed
}

/// A persisted attempt. Holds no URL, payload or response body.
struct WebhookDeliveryRecord: Identifiable, Equatable, Sendable {
    let id: UUID
    let endpointID: UUID
    let source: WebhookDeliverySource
    /// The alert trigger it belongs to: an `AlertTriggerEvent.id` or a `PineAlertNotification.id`.
    let eventID: UUID?
    var timestamp: Date
    var state: WebhookDeliveryState
    var statusCode: Int?
    var duration: TimeInterval?
    var error: WebhookDeliveryError?
}
