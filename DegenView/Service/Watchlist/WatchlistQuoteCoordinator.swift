import Foundation

/// Keeps `WatchlistQuoteBook` current for the markets watchlists are showing.
///
/// Sidebars (one per window) say which markets they need and whether they are on screen. Needs
/// are merged by `InstrumentID`, so a market that sits in three lists, or in two windows, is
/// requested once. Nothing runs while no sidebar is active; the last quotes stay in the book.
///
/// Prices come from one batched request per source per round (never one per row), plus a single
/// shared Coinbase socket. A source that fails or is rate limited backs off alone, keeping its last
/// prices marked stale. A symbol the provider rejects is found by splitting the batch and then
/// left out for ten minutes. Nothing here touches the database except the one-off chain lookup
/// for old DEX favorites, reported through `onChainResolved`.
@MainActor
final class WatchlistQuoteCoordinator {
    static let shared: WatchlistQuoteCoordinator = {
        let coordinator = WatchlistQuoteCoordinator()
        coordinator.onChainResolved = { instrument, chain in
            try? WatchlistStore.shared.setChain(chain, for: instrument)
        }
        return coordinator
    }()

    struct Configuration {
        /// How often the loop wakes. Each source is refreshed at its own `WatchlistFreshness.refreshInterval`,
        /// which is a multiple of this.
        var pollInterval: Duration = .seconds(5)
        /// A source that fails is tried again after this long, doubling up to the cap, so a brief outage
        /// costs a few seconds rather than a minute. A rate limit backs off further.
        var failureBackoff: ClosedRange<TimeInterval> = 5...30
        var rateLimitBackoff: ClosedRange<TimeInterval> = 15...60
        var rejectedRetry: TimeInterval = 600
        /// Coinbase products the socket has not answered yet are filled from REST after this long...
        var coinbaseFallbackDelay: TimeInterval = 6
        /// ...and any product the socket has been quiet about is refreshed from REST this often.
        var coinbaseRefresh: TimeInterval = 30
        var coinbaseFlush: Duration = .milliseconds(250)
        var chainLookupsPerRound = 5
    }

    private struct Consumer {
        var instruments: Set<InstrumentID>
        var isActive: Bool
    }

    private struct SourceState {
        var backoff: TimeInterval = 0
        var nextAllowed = Date.distantPast
        var lastPoll = Date.distantPast
    }

    private let provider: any WatchlistQuoteProvider
    private let book: WatchlistQuoteBook
    private let coinbase: any CoinbaseTickSource
    private let clock: () -> Date
    private let runsLoop: Bool
    private let config: Configuration

    /// Called once when a DEX pair's network is discovered, so it can be saved with the list.
    var onChainResolved: ((InstrumentID, String) -> Void)?

    private var consumers: [UUID: Consumer] = [:]
    private var known: [InstrumentID: WatchlistQuote] = [:]
    /// When each price was last confirmed by its provider; what freshness is judged by.
    private var received: [InstrumentID: Date] = [:]
    private var sources: [DataSourceType: SourceState] = [:]
    private var rejected: [InstrumentID: Date] = [:]
    private var chains: [InstrumentID: String] = [:]
    private var unresolvedChains: Set<InstrumentID> = []
    private var coinbaseProducts: [String: InstrumentID] = [:]
    private var coinbaseSubscribedAt: [InstrumentID: Date] = [:]
    private var pendingCoinbase: [InstrumentID: WatchlistQuote] = [:]
    private var flushTask: Task<Void, Never>?
    private var loop: Task<Void, Never>?
    private var isPolling = false

    init(
        provider: (any WatchlistQuoteProvider)? = nil,
        book: WatchlistQuoteBook? = nil,
        coinbase: any CoinbaseTickSource = CoinbaseWebSocketService(),
        clock: @escaping () -> Date = Date.init,
        runsLoop: Bool = true,
        configuration: Configuration = Configuration()
    ) {
        self.provider = provider ?? LiveWatchlistQuoteProvider.shared
        self.book = book ?? .shared
        self.coinbase = coinbase
        self.clock = clock
        self.runsLoop = runsLoop
        self.config = configuration
    }

    // MARK: Subscriptions

    /// What `consumer` (a window's sidebar) needs now. An inactive consumer keeps its place but
    /// asks for nothing until it is active again.
    func update(consumer: UUID, instruments: [InstrumentID], isActive: Bool) {
        consumers[consumer] = Consumer(instruments: Set(instruments), isActive: isActive)
        reconcile()
    }

    func release(consumer: UUID) {
        consumers[consumer] = nil
        reconcile()
    }

    /// Every market an active sidebar needs, once each.
    var subscribedInstruments: Set<InstrumentID> {
        consumers.values.filter(\.isActive).reduce(into: []) { $0.formUnion($1.instruments) }
    }

    func subscriberCount(for instrument: InstrumentID) -> Int {
        consumers.values.filter { $0.isActive && $0.instruments.contains(instrument) }.count
    }

    /// Whether the polling loop is running (it is not while every sidebar is hidden).
    var isRunning: Bool { loop != nil }

    private func reconcile() {
        let wanted = subscribedInstruments
        guard !wanted.isEmpty else {
            loop?.cancel()
            loop = nil
            coinbase.disconnect()
            coinbaseProducts = [:]
            flushTask?.cancel()
            flushTask = nil
            return
        }

        for instrument in wanted where instrument.source == .coinMarketCap && known[instrument] == nil {
            publish(instrument, .unsupported("Index charts have no market quote"))
        }
        // Prices that aged while nothing was polling must not read as live on return.
        ageQuotes(now: clock())
        syncCoinbase(wanted)

        if runsLoop, loop == nil {
            loop = Task { [weak self] in
                while !Task.isCancelled {
                    guard let self else { return }
                    await self.pollOnce()
                    try? await Task.sleep(for: self.config.pollInterval)
                }
            }
        }
    }

    // MARK: Polling

    /// One round: every due source gets one batched request. Safe to call by hand; overlapping
    /// calls are dropped.
    func pollOnce() async {
        guard !isPolling else { return }
        isPolling = true
        defer { isPolling = false }

        let now = clock()
        ageQuotes(now: now)
        let wanted = subscribedInstruments
        guard !wanted.isEmpty else { return }

        await resolveMissingChains(in: wanted)

        var plan: [DataSourceType: [InstrumentID]] = [:]
        for instrument in wanted {
            guard let source = polled(instrument, now: now) else { continue }
            plan[source, default: []].append(instrument)
        }
        plan = plan.filter { source, _ in isDue(source, now: now) }
        for source in plan.keys { sources[source, default: SourceState()].lastPoll = now }

        let provider = provider
        let chains = chains
        let outcomes = await withTaskGroup(of: (DataSourceType, Outcome).self) { group in
            for (source, instruments) in plan {
                let requests = instruments.sorted { $0.key < $1.key }.map { request(for: $0, chains: chains) }
                group.addTask { (source, await Self.fetch(provider: provider, source: source, requests: requests)) }
            }
            var all: [DataSourceType: Outcome] = [:]
            for await (source, outcome) in group { all[source] = outcome }
            return all
        }

        let received = clock()
        for (source, outcome) in outcomes {
            apply(outcome, source: source, instruments: plan[source] ?? [], now: received)
        }
    }

    private func isDue(_ source: DataSourceType, now: Date) -> Bool {
        let state = sources[source] ?? SourceState()
        guard now >= state.nextAllowed else { return false }
        // A second of slack: the loop sleeps its interval after each round, so rounds drift a little late.
        return now.timeIntervalSince(state.lastPoll) >= WatchlistFreshness.refreshInterval(for: source) - 1
    }

    /// The source to poll `instrument` through, or nil when it should not be polled this round.
    private func polled(_ instrument: InstrumentID, now: Date) -> DataSourceType? {
        if let until = rejected[instrument], until > now { return nil }
        switch instrument.source {
        case .coinMarketCap: return nil
        case .coinbase:
            // The socket supplies prices; REST fills a product it has not answered, and refreshes one it has
            // gone quiet on, so a thin market's price is never left to age.
            if let last = received[instrument] {
                return now.timeIntervalSince(last) >= config.coinbaseRefresh ? .coinbase : nil
            }
            guard let since = coinbaseSubscribedAt[instrument],
                now.timeIntervalSince(since) >= config.coinbaseFallbackDelay
            else { return nil }
            return .coinbase
        case .dexscreener:
            return (instrument.chain ?? chains[instrument]) == nil ? nil : .dexscreener
        default: return instrument.source
        }
    }

    private func request(for instrument: InstrumentID, chains: [InstrumentID: String]) -> QuoteRequest {
        var metadata: [String: String] = [:]
        if let chain = instrument.chain ?? chains[instrument] { metadata["chain"] = chain }
        return QuoteRequest(symbol: Self.requestSymbol(for: instrument), metadata: metadata)
    }

    /// The id each provider's quote call expects.
    private static func requestSymbol(for instrument: InstrumentID) -> String {
        instrument.apiSymbol
    }

    private struct Outcome: Sendable {
        var quotes: [String: SourceQuote] = [:]
        var rejected: Set<String> = []
        var failure: WatchlistQuoteFailure?
    }

    /// Fetches a batch. When the provider rejects it outright, halves it until the bad symbols are
    /// found, so one delisted market does not blank the list.
    private static func fetch(
        provider: any WatchlistQuoteProvider, source: DataSourceType, requests: [QuoteRequest], chunk: Int? = nil
    ) async -> Outcome {
        let size = chunk ?? chunkSize(for: source)
        if requests.count > size {
            var merged = Outcome()
            for start in stride(from: 0, to: requests.count, by: size) {
                let part = await fetch(
                    provider: provider, source: source, requests: Array(requests[start..<min(start + size, requests.count)]),
                    chunk: size)
                merged.quotes.merge(part.quotes) { first, _ in first }
                merged.rejected.formUnion(part.rejected)
                merged.failure = merged.failure ?? part.failure
            }
            return merged
        }
        do {
            return Outcome(quotes: try await provider.quotes(source: source, requests: requests))
        } catch {
            let failure = WatchlistQuoteFailure.classify(error)
            guard case .rejected = failure else { return Outcome(failure: failure) }
            guard requests.count > 1 else { return Outcome(rejected: [requests[0].symbol], failure: nil) }
            let middle = requests.count / 2
            let first = await fetch(provider: provider, source: source, requests: Array(requests[..<middle]), chunk: size)
            let second = await fetch(provider: provider, source: source, requests: Array(requests[middle...]), chunk: size)
            var merged = Outcome(quotes: first.quotes.merging(second.quotes) { a, _ in a })
            merged.rejected = first.rejected.union(second.rejected)
            merged.failure = first.failure ?? second.failure
            return merged
        }
    }

    private static func chunkSize(for source: DataSourceType) -> Int {
        switch source {
        case .binance: 100
        case .alpaca: 200
        case .coingecko: 250
        case .coinbase: 20
        default: 100
        }
    }

    private func apply(_ outcome: Outcome, source: DataSourceType, instruments: [InstrumentID], now: Date) {
        var state = sources[source] ?? SourceState()
        var updates: [InstrumentID: WatchlistQuote] = [:]

        switch outcome.failure {
        case .rateLimited?, .failed?:
            let range = outcome.failure == .rateLimited ? config.rateLimitBackoff : config.failureBackoff
            state.backoff = min(max(state.backoff * 2, range.lowerBound), range.upperBound)
            state.nextAllowed = now.addingTimeInterval(state.backoff)
            for instrument in instruments { updates[instrument] = fallback(for: instrument, note: outcome.failure?.message) }
        case .credentialsMissing?:
            state.nextAllowed = now.addingTimeInterval(60)
            for instrument in instruments { updates[instrument] = .unsupported(outcome.failure?.message) }
        case .rejected?, nil:
            state.backoff = 0
            state.nextAllowed = .distantPast
            // A symbol is only called bad when its neighbours answered; if nothing did, the source is down.
            let sourceAnswered = !outcome.quotes.isEmpty
            for instrument in instruments {
                let symbol = Self.requestSymbol(for: instrument)
                if let quote = outcome.quotes[symbol] {
                    updates[instrument] = WatchlistQuote(source: source, quote: quote, receivedAt: now)
                    received[instrument] = now
                    rejected[instrument] = nil
                } else if outcome.rejected.contains(symbol), sourceAnswered {
                    rejected[instrument] = now.addingTimeInterval(config.rejectedRetry)
                    updates[instrument] = WatchlistQuote(freshness: .unavailable, note: "The provider doesn't know this market")
                } else {
                    updates[instrument] = fallback(for: instrument, note: "The provider returned no quote")
                }
            }
        }
        sources[source] = state
        for (instrument, quote) in updates { known[instrument] = quote }
        book.apply(updates)
    }

    /// The last known price, marked stale; or an unavailable quote if there never was one.
    private func fallback(for instrument: InstrumentID, note: String?) -> WatchlistQuote {
        if let last = known[instrument], last.last != nil { return last.with(freshness: .stale) }
        return WatchlistQuote(freshness: .unavailable, note: note)
    }

    private func publish(_ instrument: InstrumentID, _ quote: WatchlistQuote) {
        known[instrument] = quote
        book.apply([instrument: quote])
    }

    /// Downgrades prices whose window has passed since they were stamped.
    func ageQuotes(now: Date) {
        var updates: [InstrumentID: WatchlistQuote] = [:]
        for (instrument, quote) in known where quote.freshness.isCurrent {
            guard let confirmed = received[instrument] else { continue }
            if WatchlistFreshness.evaluate(source: instrument.source, receivedAt: confirmed, now: now) == .stale {
                updates[instrument] = quote.with(freshness: .stale)
            }
        }
        guard !updates.isEmpty else { return }
        known.merge(updates) { _, new in new }
        book.apply(updates)
    }

    // MARK: DEX chains

    private func resolveMissingChains(in wanted: Set<InstrumentID>) async {
        let missing = wanted.filter {
            $0.source == .dexscreener && $0.chain == nil && chains[$0] == nil && !unresolvedChains.contains($0)
        }
        for instrument in missing.sorted(by: { $0.key < $1.key }).prefix(config.chainLookupsPerRound) {
            if let chain = await provider.resolveChain(address: instrument.symbol) {
                chains[instrument] = chain
                onChainResolved?(instrument, chain)
            } else {
                // Don't ask again this session; the row keeps its dash and says why.
                unresolvedChains.insert(instrument)
                publish(instrument, WatchlistQuote(freshness: .unavailable, note: "Couldn't find this pair's network"))
            }
        }
    }

    // MARK: Coinbase socket

    private func syncCoinbase(_ wanted: Set<InstrumentID>) {
        let now = clock()
        var products: [String: InstrumentID] = [:]
        for instrument in wanted where instrument.source == .coinbase {
            products[instrument.apiSymbol.uppercased()] = instrument
        }
        guard Set(products.keys) != Set(coinbaseProducts.keys) else { return }

        for instrument in products.values where coinbaseSubscribedAt[instrument] == nil {
            coinbaseSubscribedAt[instrument] = now
        }
        coinbaseProducts = products
        if products.isEmpty {
            coinbase.disconnect()
        } else {
            coinbase.connect(products: Array(products.keys).sorted()) { [weak self] tick in
                MainActor.assumeIsolated { self?.receive(tick) }
            }
        }
    }

    func receive(_ tick: CoinbaseTick) {
        guard let instrument = coinbaseProducts[tick.productID.uppercased()] else { return }
        pendingCoinbase[instrument] = WatchlistQuote(coinbaseTick: tick, receivedAt: clock())
        guard flushTask == nil else { return }
        flushTask = Task { [weak self, interval = config.coinbaseFlush] in
            try? await Task.sleep(for: interval)
            guard !Task.isCancelled else { return }
            self?.flushCoinbase()
        }
    }

    /// Publishes the latest tick of each product; a busy feed becomes a few updates a second.
    func flushCoinbase() {
        flushTask = nil
        guard !pendingCoinbase.isEmpty else { return }
        let batch = pendingCoinbase
        pendingCoinbase = [:]
        let now = clock()
        for instrument in batch.keys { received[instrument] = now }
        known.merge(batch) { _, new in new }
        book.apply(batch)
    }
}
