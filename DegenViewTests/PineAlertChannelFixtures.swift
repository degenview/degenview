import Foundation

@testable import DegenView

/// Remembers what it was asked to deliver.
final class RecordingPineAlertChannel: PineAlertChannel, @unchecked Sendable {
    let name = "recording"
    private let lock = NSLock()
    private var storage: [PineAlertNotification] = []

    var delivered: [PineAlertNotification] {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }

    func deliver(_ notification: PineAlertNotification) async throws {
        lock.lock()
        storage.append(notification)
        lock.unlock()
    }
}

/// Always fails.
struct FailingPineAlertChannel: PineAlertChannel {
    struct Failure: Error {}
    let name = "failing"

    func deliver(_ notification: PineAlertNotification) async throws {
        throw Failure()
    }
}
