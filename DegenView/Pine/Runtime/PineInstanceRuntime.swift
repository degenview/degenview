import Foundation

/// One applied Pine instance's execution plumbing — its own feed, detached task, generation
/// counter and dataset. Owned by `ChartViewModel`, one per `scriptInstances` entry, never
/// shared. Pure bookkeeping; no Pine logic lives here.
final class PineInstanceRuntime {
    let instanceID: UUID
    var feed: AsyncStream<PineFeedOperation>.Continuation?
    var feedTask: Task<Void, Never>?
    var generation = 0
    var dataset: PineDatasetKey?
    /// Cached after source resolution so alert/context hashing doesn't need to re-fetch from
    /// `ScriptStore` synchronously.
    var resolvedSource: String?

    init(instanceID: UUID) { self.instanceID = instanceID }

    func stop() {
        feed?.finish()
        feedTask?.cancel()
        feed = nil
        feedTask = nil
    }
}
