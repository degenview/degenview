import Foundation

/// One market's quote, observable on its own so a price tick redraws only that row.
@MainActor
final class WatchlistQuoteCell: ObservableObject {
    @Published private(set) var quote: WatchlistQuote?

    /// Publishes only when the quote actually changed.
    func update(_ new: WatchlistQuote) {
        guard quote != new else { return }
        quote = new
    }
}
