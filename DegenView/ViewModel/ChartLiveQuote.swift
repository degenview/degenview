import Foundation

/// A chart's latest best bid and ask, from the live stream of the exchange it trades on.
///
/// A separate object from `ChartViewModel` on purpose: book updates arrive several times a second,
/// and only the paper-trading quote feed needs them. Putting them on the view model would redraw the
/// whole chart card for every one.
@MainActor
final class ChartLiveQuote: ObservableObject {
    @Published private(set) var bid: Double?
    @Published private(set) var ask: Double?
    /// When `bid` and `ask` were last set. Not published: it only matters when a quote is read.
    private(set) var updatedAt: Date?

    /// Records a new best bid and ask. Ignored unless both are present and sane (positive, bid not
    /// above ask), so a frame without a book never clears a good one.
    func apply(bid: Double?, ask: Double?, at date: Date = Date()) {
        guard let bid, let ask, bid > 0, ask >= bid else { return }
        if self.bid != bid { self.bid = bid }
        if self.ask != ask { self.ask = ask }
        updatedAt = date
    }

    func clear() {
        bid = nil
        ask = nil
        updatedAt = nil
    }
}
