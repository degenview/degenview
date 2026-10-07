import Combine
import Foundation

/// In-process fan-out of `AlertDomainEvent`s. Publishers say what happened; subscribers
/// (`UnseenAlertsStore` today) decide what to do about it.
///
/// Every alert trigger path in the app must publish here: price alerts from `AlertStore.reload()`
/// (the one place agent- and app-evaluated triggers meet) and script alerts from
/// `PineAlertStore.record(_:dedupeKey:)`.
@MainActor
final class AlertEventBus {
    static let shared = AlertEventBus()

    private let subject = PassthroughSubject<AlertDomainEvent, Never>()

    var events: AnyPublisher<AlertDomainEvent, Never> { subject.eraseToAnyPublisher() }

    func publish(_ event: AlertDomainEvent) {
        subject.send(event)
    }
}
