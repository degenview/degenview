import Foundation

/// The result of feeding the controller an event.
enum PineExecutionOutcome: Sendable {
    /// The event produced output (possibly with no execution, for a strategy waiting for the close).
    case updated(PineExecutionUpdate)
    /// Nothing changed: a duplicate, late or stale message.
    case unchanged
    /// The feed no longer matches the committed history; rebuild from a full snapshot.
    case needsRebuild(PineCandleAggregator.RebuildReason?)
    /// The script failed at runtime. The controller stays failed until rebuilt.
    case failed(PineDiagnostic)
}
