import Foundation

/// Runs at most one search at a time, delayed so a burst of keystrokes only costs one
/// request round.
///
/// Cancellation is what does the debouncing: every keystroke cancels the pending task
/// before it wakes, so only the last one gets past the sleep.
@MainActor
final class SearchDebouncer {
    private var task: Task<Void, Never>?
    private let delay: UInt64

    init(delay: UInt64 = Timeout.searchDebounceNS) {
        self.delay = delay
    }

    /// Schedule `work`, replacing any pending run. `work` starts only if the delay
    /// elapses without another `schedule` call.
    func schedule(_ work: @escaping @MainActor () async -> Void) {
        task?.cancel()
        task = Task { @MainActor in
            try? await Task.sleep(nanoseconds: delay)
            guard !Task.isCancelled else { return }
            await work()
        }
    }

    /// Drop the pending run without starting it.
    func cancel() {
        task?.cancel()
        task = nil
    }
}
