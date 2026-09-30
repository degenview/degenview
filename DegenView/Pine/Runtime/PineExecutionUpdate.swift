import Foundation

/// What one call into the execution controller produced.
struct PineExecutionUpdate: Sendable {
    /// The script's output after the last execution.
    var output: PineVisualOutput
    /// `barstate.*` of each execution performed, in order. Empty when the scheduler ran nothing.
    var executions: [PineBarFlags] = []
    /// Alerts raised on realtime bars, for delivery. Historical executions never contribute.
    var alerts: [PineAlertEvent] = []
    /// The bar the last execution ran on.
    var barID: PineBarID?
}
