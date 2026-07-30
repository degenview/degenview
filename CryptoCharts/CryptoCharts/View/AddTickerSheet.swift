import SwiftUI

struct AddTickerSheet: View {
    @ObservedObject var contentViewModel: ContentViewModel
    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[AddTicker]")

    @State private var inputText = ""
    @State private var addError: String?

    @Environment(\.dismiss) private var dismiss

    private let suggestions = ["BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK"]

    var body: some View {
        VStack(spacing: 16) {
            // Header
            Text("Add Ticker")
                .font(.headline)

            // Search input
            HStack(spacing: 8) {
                TextField("Ticker symbol (e.g. BTC or PEPE)", text: $inputText)
                    .textFieldStyle(.roundedBorder)
                    .font(.body)
                    .onChange(of: inputText) {
                        searchVM.scheduleSearch(query: inputText)
                    }
                    .onSubmit {
                        if let first = searchVM.firstAvailableResult {
                            searchVM.selectedResult = first
                        }
                    }

                if searchVM.isSearching {
                    ProgressView()
                        .scaleEffect(0.7)
                        .frame(width: 20, height: 20)
                }
            }

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
                HStack {
                    Label(
                        "Selected: \(selected.symbol)",
                        systemImage: selected.source.icon
                    )
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.primary)

                    if let price = selected.price {
                        Text(price, format: .currency(code: "USD").precision(.fractionLength(2...6)))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    Spacer()
                }
                .padding(8)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
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
                    searchVM.cancelSearch()
                    dismiss()
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.escape)

                Button("Add") {
                    addTicker()
                }
                .buttonStyle(.borderedProminent)
                .disabled(searchVM.selectedResult == nil)
                .keyboardShortcut(.return)
            }
            .padding(.top, 8)
        }
        .padding(24)
        .frame(width: UI.addTickerSheetWidth)
        .onDisappear {
            searchVM.cancelSearch()
        }
    }

    // MARK: - Add

    private func addTicker() {
        guard let selected = searchVM.selectedResult else { return }

        Task { @MainActor in
            do {
                try await contentViewModel.addTicker(
                    symbol: selected.fullSymbol,
                    source: selected.source
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
