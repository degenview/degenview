import Foundation

/// Something that happened to an alert, published on `AlertEventBus` the moment the app learns of it.
enum AlertDomainEvent: Equatable, Sendable {
    /// A price alert fired: the id of its `AlertTriggerEvent`.
    case priceAlertTriggered(eventID: UUID, at: Date)
    /// A Pine script alert was delivered: the id of its `PineAlertNotification`.
    case scriptAlertTriggered(eventID: UUID, at: Date)

    var eventID: UUID {
        switch self {
        case .priceAlertTriggered(let eventID, _), .scriptAlertTriggered(let eventID, _): eventID
        }
    }

    var occurredAt: Date {
        switch self {
        case .priceAlertTriggered(_, let date), .scriptAlertTriggered(_, let date): date
        }
    }
}
