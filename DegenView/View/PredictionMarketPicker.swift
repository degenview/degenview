import SwiftUI

/// The "Prediction Markets" pane: the shared search pane with a Polymarket / Kalshi
/// dropdown in front of its search field. Both providers keep their own view model, so
/// changing the dropdown re-runs the current query against the other one without losing
/// either's results or selection.
struct PredictionMarketPicker: View {
    @Binding var provider: DataSourceType
    @ObservedObject var polymarketVM: PredictionMarketSearchViewModel
    @ObservedObject var kalshiVM: PredictionMarketSearchViewModel
    @Binding var searchText: String
    var sizing: SearchResultListSizing
    var showsStatus = true
    var suggestions: [SuggestionChipGrid.Item]? = nil
    var onCommitResult: ((TickerSearchResult) -> Void)? = nil

    /// View model behind `provider`.
    static func viewModel(
        for provider: DataSourceType,
        polymarket: PredictionMarketSearchViewModel,
        kalshi: PredictionMarketSearchViewModel
    ) -> PredictionMarketSearchViewModel {
        provider == .kalshi ? kalshi : polymarket
    }

    private func activeVM(for provider: DataSourceType) -> PredictionMarketSearchViewModel {
        Self.viewModel(for: provider, polymarket: polymarketVM, kalshi: kalshiVM)
    }

    var body: some View {
        PredictionMarketSearchPane(
            searchVM: activeVM(for: provider),
            searchText: $searchText,
            sizing: sizing,
            showsStatus: showsStatus,
            provider: $provider,
            suggestions: suggestions,
            onCommitResult: onCommitResult
        )
        .onChange(of: provider) { oldValue, newValue in
            activeVM(for: oldValue).cancelSearch()
            activeVM(for: newValue).scheduleSearch(query: searchText)
        }
    }
}
