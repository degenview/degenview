import SwiftUI

/// Keeps the paper-trading engine's quote for one chart's market current.
///
/// It is a view, with `@ObservedObject` on the chart, so SwiftUI re-evaluates it whenever the
/// chart's price or refresh time changes. `ContentView` cannot do this job itself: it does not
/// observe its charts, so a watcher written there only ran when something unrelated redrew it.
///
/// It draws nothing. It is mounted behind each chart card.
struct PaperQuoteFeed: View {
    @ObservedObject var viewModel: ChartViewModel
    @ObservedObject var quote: ChartLiveQuote
    @ObservedObject var store: PaperTradingStore

    var body: some View {
        let sample = PaperQuoteSample.make(
            last: viewModel.currentPrice, bid: quote.bid, ask: quote.ask, bookUpdatedAt: quote.updatedAt,
            refreshedAt: viewModel.lastUpdated, isConnected: store.isConnected)
        // `initial` feeds a chart the moment it appears, so a quote restored from an earlier run
        // never outlives the first refresh.
        Color.clear
            .frame(width: 0, height: 0)
            .onChange(of: sample, initial: true) { _, sample in feed(sample) }
            .accessibilityHidden(true)
    }

    private func feed(_ sample: PaperQuoteSample) {
        guard isMarketChart, sample.isConnected, let last = sample.last else { return }
        let instrument = PaperInstrument.chart(
            symbol: viewModel.apiSymbol, displayName: viewModel.title, source: viewModel.source)
        store.stream(
            instrument: instrument, bid: sample.bid.map(PaperQuoteSample.decimal),
            ask: sample.ask.map(PaperQuoteSample.decimal), last: PaperQuoteSample.decimal(last))
    }

    /// Portfolio, CoinMarketCap and power-law cards show no tradable market.
    private var isMarketChart: Bool {
        !viewModel.isPortfolioChart && !viewModel.isBitcoinPowerLaw && viewModel.coinMarketCapChart == nil
    }
}
