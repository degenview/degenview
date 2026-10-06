import SwiftUI

/// The coin or company artwork for a paper instrument, resolved through `IconResolver` like every
/// other icon in the app; a monogram until it arrives.
struct PaperInstrumentIcon: View {
    let instrument: PaperInstrument
    var size: CGFloat = 18
    /// Badge the data source's logo on the corner. Off in tables, where the symbol is enough.
    var showsSource = false
    @State private var iconURL: URL?

    var body: some View {
        TickerIconView(
            symbol: instrument.baseSymbol, url: iconURL, size: size, source: showsSource ? instrument.source : nil
        )
        .task(id: instrument.key) {
            iconURL = await IconResolver.shared.iconURL(
                ticker: instrument.symbol, source: instrument.source, baseSymbol: instrument.baseSymbol)
        }
        .accessibilityHidden(true)
    }
}
