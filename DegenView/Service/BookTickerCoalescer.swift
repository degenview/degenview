import Foundation

/// Thins a firehose of best bid/ask updates to a few deliveries a second.
///
/// Binance's `bookTicker` pushes on every book change, which on a liquid pair is hundreds of times
/// a second; a chart only needs the latest. Each symbol keeps its newest quote, and one flush after
/// `delay` hands every pending symbol over, so the last update is never dropped (a quiet symbol
/// still gets its final quote delivered). Call it, and receive from it, on the main thread.
final class BookTickerCoalescer {
    struct Quote: Equatable {
        let bid: Double
        let ask: Double
    }

    /// Runs `block` after `delay` seconds. Injectable so tests can fire flushes by hand.
    typealias Scheduler = (TimeInterval, @escaping () -> Void) -> Void

    var deliver: ((String, Quote) -> Void)?
    private var pending: [String: Quote] = [:]
    private var isFlushScheduled = false
    /// Bumped by `cancel()` so a flush already scheduled before it does nothing.
    private var generation = 0
    private let delay: TimeInterval
    private let schedule: Scheduler

    init(
        delay: TimeInterval = 0.25,
        schedule: @escaping Scheduler = { delay, block in
            DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: block)
        }
    ) {
        self.delay = delay
        self.schedule = schedule
    }

    func submit(symbol: String, bid: Double, ask: Double) {
        pending[symbol] = Quote(bid: bid, ask: ask)
        guard !isFlushScheduled else { return }
        isFlushScheduled = true
        let scheduledGeneration = generation
        schedule(delay) { [weak self] in
            guard let self, self.generation == scheduledGeneration else { return }
            self.flush()
        }
    }

    /// Delivers everything pending now.
    func flush() {
        isFlushScheduled = false
        let batch = pending
        pending = [:]
        for (symbol, quote) in batch { deliver?(symbol, quote) }
    }

    /// Drops whatever is pending and invalidates any flush already scheduled.
    func cancel() {
        generation += 1
        isFlushScheduled = false
        pending = [:]
    }
}
