import Foundation

/// Latest quotes by `InstrumentID.key`. Nothing here is persisted.
///
/// Rows observe a single `WatchlistQuoteCell`. Sorting by a quote value needs every value,
/// so `revision` ticks at most once a second instead of on every update.
@MainActor
final class WatchlistQuoteBook: ObservableObject {
    static let shared = WatchlistQuoteBook()

    /// Bumps (throttled) when any quote changes. Observe this only where all values matter.
    @Published private(set) var revision = 0

    private var cells: [String: WatchlistQuoteCell] = [:]
    private var revisionPending = false
    private let revisionInterval: Duration

    init(revisionInterval: Duration = .seconds(1)) {
        self.revisionInterval = revisionInterval
    }

    func cell(for instrument: InstrumentID) -> WatchlistQuoteCell {
        if let cell = cells[instrument.key] { return cell }
        let cell = WatchlistQuoteCell()
        cells[instrument.key] = cell
        return cell
    }

    func quote(for instrument: InstrumentID) -> WatchlistQuote? {
        cells[instrument.key]?.quote
    }

    /// Replaces quotes for the given keys; a cell whose quote is unchanged does not publish.
    func apply(_ quotes: [InstrumentID: WatchlistQuote]) {
        var changed = false
        for (instrument, quote) in quotes {
            let cell = cell(for: instrument)
            guard cell.quote != quote else { continue }
            cell.update(quote)
            changed = true
        }
        if changed { scheduleRevision() }
    }

    /// Marks every quote from `sources` with a new freshness, keeping the last known values.
    func markAll(from sources: Set<DataSourceType>, as freshness: WatchlistQuote.Freshness, among instruments: [InstrumentID]) {
        var update: [InstrumentID: WatchlistQuote] = [:]
        for instrument in instruments where sources.contains(instrument.source) {
            let existing = quote(for: instrument) ?? WatchlistQuote()
            update[instrument] = existing.with(freshness: freshness)
        }
        apply(update)
    }

    private func scheduleRevision() {
        guard !revisionPending else { return }
        revisionPending = true
        Task { [weak self, revisionInterval] in
            try? await Task.sleep(for: revisionInterval)
            guard let self else { return }
            self.revisionPending = false
            self.revision += 1
        }
    }
}
