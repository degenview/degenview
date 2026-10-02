import Foundation

/// A `PineSecurityDataProvider` that holds no data and remembers what it was asked for. Running a script
/// against it (it answers every series with no candles) shows which series the script's `request.security`
/// calls read, so they can be fetched before the real run.
final class PineSecurityRecorder: PineSecurityDataProvider, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [PineSecurityKey] = []

    /// The series asked for, in first-asked order, without repeats.
    var keys: [PineSecurityKey] { lock.withLock { recorded } }

    func candles(for key: PineSecurityKey) -> [KlineData]? {
        lock.withLock { if !recorded.contains(key) { recorded.append(key) } }
        return []
    }
}
