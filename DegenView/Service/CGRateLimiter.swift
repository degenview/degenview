import Foundation

/// Enforces minimum interval between CoinGecko API calls to stay under ~25 req/min.
///
/// Callers *reserve* their slot before sleeping. Reserving after the sleep would
/// let every concurrent caller read the same free slot during the suspension and
/// fire simultaneously — with one chart per coin that means an instant burst and
/// a 429 for everyone.
///
/// Shared process-wide: OHLC fetches and icon lookups draw on the same public-tier
/// budget, so they have to queue behind one limiter or they provoke each other's 429s.
actor CGRateLimiter {
    static let shared = CGRateLimiter()

    /// Earliest instant the next unreserved call may run.
    private var nextSlot: Date = .distantPast

    /// The public tier's ceiling isn't published and moves around, so the gap is
    /// found by feedback rather than assumed: widen on every 429, ease back down
    /// while calls succeed.
    private var gap: TimeInterval
    /// Floor the gap decays back to. Per-instance: GeckoTerminal is a separate host
    /// with its own budget, so it queues on its own limiter at its own pace.
    private let baseGap: TimeInterval

    init(gap: TimeInterval = Timeout.coingeckoRateLimitGap) {
        self.gap = gap
        self.baseGap = gap
    }

    /// Wait until this caller's reserved slot. Returns immediately if one is free now.
    func waitForSlot() async {
        let now = Date()
        let slot = max(now, nextSlot)
        nextSlot = slot.addingTimeInterval(gap)

        let wait = slot.timeIntervalSince(now)
        guard wait > 0 else { return }
        try? await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
    }

    /// Back off after a 429 — pushes out every slot not yet taken and widens the
    /// gap. Doesn't sleep: the caller's retry blocks on `waitForSlot()` instead.
    func backoff(seconds: Int) {
        gap = min(gap * Timeout.coingeckoRateLimitGrowth, Timeout.coingeckoRateLimitMaxGap)

        let resume = Date().addingTimeInterval(Double(seconds))
        if resume > nextSlot { nextSlot = resume }

        #if DEBUG
            print("[CoinGecko] Rate limiter gap → \(String(format: "%.1f", gap))s")
        #endif
    }

    /// Narrow the gap gradually while the API is keeping up.
    func noteSuccess() {
        gap = max(gap * Timeout.coingeckoRateLimitDecay, baseGap)
    }
}
