import SwiftUI

struct AddTickerSheet: View {
    @ObservedObject var contentViewModel: ContentViewModel

    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[AddTicker]")
    @StateObject private var polymarketVM = PolymarketSearchViewModel(logPrefix: "[AddTicker/Polymarket]")

    @State private var selectedTab: Tab = .crypto
    @State private var inputText = ""
    @State private var polymarketText = ""
    @State private var addError: String?

    @Environment(\.dismiss) private var dismiss

    private let suggestions = ["BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK"]

    enum Tab: String, CaseIterable {
        case crypto = "Crypto"
        case polymarket = "Polymarket"
    }

    /// Whichever pane is showing owns the selection the Add button commits.
    private var activeSelection: TickerSearchResult? {
        switch selectedTab {
        case .crypto:     return searchVM.selectedResult
        case .polymarket: return polymarketVM.selectedResult
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            // Header
            Text("Add Chart")
                .font(.headline)

            Picker("", selection: $selectedTab) {
                ForEach(Tab.allCases, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)

            switch selectedTab {
            case .crypto:
                cryptoTab
            case .polymarket:
                PolymarketSearchPane(
                    searchVM: polymarketVM,
                    searchText: $polymarketText,
                    resultsMaxHeight: UI.addTickerResultsMaxHeight
                )
            }

            // Error from add attempt
            if let error = addError {
                HStack {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            // Action buttons
            HStack(spacing: 12) {
                Button("Cancel") {
                    cancelSearches()
                    dismiss()
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.escape)

                Button("Add") {
                    addTicker()
                }
                .buttonStyle(.borderedProminent)
                .disabled(activeSelection == nil)
                .keyboardShortcut(.return)
            }
            .padding(.top, 8)
        }
        .padding(24)
        .frame(width: UI.addTickerSheetWidth)
        .onChange(of: selectedTab) { addError = nil }
        .onDisappear {
            cancelSearches()
        }
    }

    // MARK: - Crypto Tab

    private var cryptoTab: some View {
        VStack(spacing: 16) {
            SearchFieldRow(
                placeholder: "Ticker symbol (e.g. BTC or PEPE)",
                text: $inputText,
                isSearching: searchVM.isSearching,
                onChange: { searchVM.scheduleSearch(query: $0) },
                onSubmit: {
                    if let first = searchVM.firstAvailableResult {
                        searchVM.selectedResult = first
                    }
                }
            )

            // Suggestions
            if searchVM.searchResults.isEmpty && !searchVM.isSearching {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Suggestions")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    LazyVGrid(columns: Array(repeating: .init(.flexible()), count: UI.suggestionGridColumns), spacing: 8) {
                        ForEach(suggestions, id: \.self) { ticker in
                            Button(ticker) {
                                inputText = ticker
                                searchVM.scheduleSearch(query: ticker)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                        }
                    }
                }
            }

            // Search results grouped by source
            if !searchVM.searchResults.isEmpty {
                List {
                    ForEach(searchVM.orderedSources, id: \.self) { source in
                        if let results = searchVM.searchResults[source], !results.isEmpty {
                            Section {
                                ForEach(results) { result in
                                    SearchResultRow(
                                        result: result,
                                        isSelected: searchVM.selectedResult == result,
                                        onSelect: { searchVM.selectedResult = result }
                                    )
                                }
                            } header: {
                                Label(source.displayName, systemImage: source.icon)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .listStyle(.inset)
                .frame(minHeight: UI.addTickerResultsMinHeight, maxHeight: UI.addTickerResultsMaxHeight)
            }

            // No results
            if !searchVM.isSearching && !inputText.trimmingCharacters(in: .whitespaces).isEmpty
                && searchVM.searchResults.isEmpty {
                Text("No results found")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            // Selected result
            if let selected = searchVM.selectedResult {
                SelectedResultBanner(prefix: "Selected", result: selected)
            }
        }
    }

    // MARK: - Add

    private func cancelSearches() {
        searchVM.cancelSearch()
        polymarketVM.cancelSearch()
    }

    private func addTicker() {
        guard let selected = activeSelection else { return }

        Task { @MainActor in
            do {
                // Polymarket search already handed us the market artwork; seed the
                // resolver so the new card paints it without another round trip.
                if selected.source == .polymarket {
                    await IconResolver.shared.remember(
                        ticker: selected.fullSymbol,
                        source: .polymarket,
                        url: selected.imageURL
                    )
                }

                try await contentViewModel.addTicker(
                    symbol: selected.fullSymbol,
                    source: selected.source,
                    displayName: selected.source == .polymarket ? selected.symbol : nil
                )
                dismiss()
            } catch {
                addError = error.localizedDescription
            }
        }
    }
}

#Preview {
    AddTickerSheet(contentViewModel: ContentViewModel())
}
