import SwiftUI

/// The icon for a chart's market: the coin or company artwork, with its data source's logo small
/// on the bottom-right corner. Resolves the artwork itself through `IconResolver`, so every header
/// that shows a chart looks the same and re-resolves when the chart is pointed at another market.
struct ChartIconView: View {
    @ObservedObject var viewModel: ChartViewModel
    var size: CGFloat = Icon.size
    /// Off where the source is already obvious, such as a favorites list.
    var showsSource = true

    @State private var iconURL: URL?

    var body: some View {
        TickerIconView(
            symbol: viewModel.baseSymbol, url: iconURL, size: size,
            source: showsSource ? viewModel.source : nil
        )
        .task(id: viewModel.iconKey) {
            iconURL = nil
            iconURL = await IconResolver.shared.iconURL(
                ticker: viewModel.ticker,
                source: viewModel.source,
                baseSymbol: viewModel.baseSymbol
            )
        }
    }
}
