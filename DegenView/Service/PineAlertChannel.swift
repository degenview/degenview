import Foundation

/// Somewhere a Pine alert can be delivered. Pine knows nothing about channels: a webhook would be
/// one more conformance registered with `PineAlertDispatcher`, with no change to the runtime.
protocol PineAlertChannel: Sendable {
    /// For logs.
    var name: String { get }
    func deliver(_ notification: PineAlertNotification) async throws
}
