import Foundation

/// Where the crosshair tool's pointer is, in chart terms rather than screen terms.
///
/// Stored as a position along the plot rather than as a time. The charts in a tab share
/// a candle *count*, so the same fraction is the same candle slot on every card and the
/// lines coincide — which is the point of drawing them on every chart at once.
///
/// It does mean that sources whose candles are a different size, like CoinGecko's, read a
/// different moment at the same column. Each chart labels its own, so the line never
/// claims a time that isn't its own.
struct Crosshair: Equatable {
    /// `ChartViewModel.uniqueID` of the chart the pointer is over. The horizontal line
    /// and the price pill are drawn only there; price scales are per chart.
    let ownerID: String
    /// 0 at the left edge of the plot, 1 at the right. Free-follows the pointer.
    let xFraction: CGFloat
    let price: Double
}

/// The one crosshair a tab has, kept off `ContentViewModel` on purpose.
///
/// `ContentViewModel` publishing this would fire `objectWillChange` on every mouse move,
/// re-running `ContentView.body` and with it every card's indicator computation and
/// candle `Canvas`. Held separately, only the thin overlay that observes it redraws.
@MainActor
final class CrosshairTracker: ObservableObject {
    @Published private(set) var current: Crosshair?

    func update(_ crosshair: Crosshair) {
        current = crosshair
    }

    func clear() {
        current = nil
    }

    /// Clear only if `id` still owns the crosshair. Moving the pointer between two cards
    /// fires the new card's enter and the old card's exit in no guaranteed order, and a
    /// blind clear would wipe the reading that was just taken.
    func clear(owner id: String) {
        guard current?.ownerID == id else { return }
        current = nil
    }
}
