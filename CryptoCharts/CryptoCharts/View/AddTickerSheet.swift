import SwiftUI

struct AddTickerSheet: View {
    @ObservedObject var contentViewModel: ContentViewModel

    @State private var inputText = ""
    @State private var searchResults: [DataSourceType: [TickerSearchResult]] = [:]
    @State private var isSearching = false
    @State private var selectedResult: TickerSearchResult?
    @State private var addError: String?
    @State private var searchTask: Task<Void, Never>?

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
                        scheduleSearch()
                    }
                    .onSubmit {
                        // Pick first result on Enter if available
                        if let first = firstAvailableResult {
                            selectedResult = first
                        }
                    }

                if isSearching {
                    ProgressView()
                        .scaleEffect(0.7)
                        .frame(width: 20, height: 20)
                }
            }

            // Suggestions
            if searchResults.isEmpty && !isSearching {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Suggestions")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    LazyVGrid(columns: Array(repeating: .init(.flexible()), count: 5), spacing: 8) {
                        ForEach(suggestions, id: \.self) { ticker in
                            Button(ticker) {
                                inputText = ticker
                                scheduleSearch()
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                        }
                    }
                }
            }

            // Search results grouped by source
            if !searchResults.isEmpty {
                List {
                    ForEach(orderedSources, id: \.self) { source in
                        if let results = searchResults[source], !results.isEmpty {
                            Section {
                                ForEach(results) { result in
                                    searchResultRow(result)
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
                .frame(minHeight: 100, maxHeight: 300)
            }

            // No results
            if !isSearching && !inputText.trimmingCharacters(in: .whitespaces).isEmpty
                && searchResults.isEmpty && searchTask == nil {
                Text("No results found")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            // Selected result
            if let selected = selectedResult {
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
                    searchTask?.cancel()
                    dismiss()
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.escape)

                Button("Add") {
                    addTicker()
                }
                .buttonStyle(.borderedProminent)
                .disabled(selectedResult == nil)
                .keyboardShortcut(.return)
            }
            .padding(.top, 8)
        }
        .padding(24)
        .frame(width: 440)
        .onDisappear {
            searchTask?.cancel()
        }
    }

    // MARK: - Search Result Row

    private func searchResultRow(_ result: TickerSearchResult) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(result.symbol)
                    .font(.body.weight(.medium))

                if let chain = result.chain, let dex = result.dex {
                    Text("\(chain.capitalized) · \(dex.capitalized)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else if result.source == .coingecko {
                    Text("via CoinGecko")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else if result.source == .binance {
                    Text("via Binance")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if let price = result.price {
                Text(price, format: .currency(code: "USD").precision(.fractionLength(2...6)))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onTapGesture {
            selectedResult = result
        }
        .background(selectedResult == result
            ? Color.accentColor.opacity(0.15)
            : Color.clear
        )
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }

    // MARK: - Search

    private var orderedSources: [DataSourceType] {
        var sources = DataSourceType.allCases
        // Sort: sources with results first, then by enum order
        sources.sort { a, b in
            let aHas = !(searchResults[a]?.isEmpty ?? true)
            let bHas = !(searchResults[b]?.isEmpty ?? true)
            if aHas != bHas { return aHas }
            return a.rawValue < b.rawValue
        }
        return sources
    }

    private var firstAvailableResult: TickerSearchResult? {
        for source in orderedSources {
            if let results = searchResults[source], let first = results.first {
                return first
            }
        }
        return nil
    }

    private func scheduleSearch() {
        searchTask?.cancel()

        let text = inputText.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            searchResults = [:]
            selectedResult = nil
            return
        }

        let captured = text
        searchTask = Task { @MainActor in
            // Debounce 300ms
            try? await Task.sleep(nanoseconds: 300_000_000)
            guard !Task.isCancelled, inputText.trimmingCharacters(in: .whitespaces) == captured else { return }

            isSearching = true
            defer { isSearching = false }

            let sources = DataSourceFactory.shared.allSources
            var newResults: [DataSourceType: [TickerSearchResult]] = [:]

            // Search all sources in parallel
            await withTaskGroup(of: (DataSourceType, [TickerSearchResult]?).self) { group in
                for source in sources {
                    group.addTask {
                        do {
                            let results = try await source.searchTickers(query: captured)
                            return (source.type, results)
                        } catch {
                            print("[AddTicker] \(source.type.displayName) search failed: \(error.localizedDescription)")
                            return (source.type, nil)
                        }
                    }
                }

                for await (type, results) in group {
                    if let r = results, !r.isEmpty {
                        newResults[type] = r
                    }
                }
            }

            guard !Task.isCancelled else { return }
            searchResults = newResults

            // Keep selection if still in results, otherwise clear
            if let selected = selectedResult,
               !newResults.values.flatMap({ $0 }).contains(selected) {
                selectedResult = nil
            }
        }
    }

    // MARK: - Add

    private func addTicker() {
        guard let selected = selectedResult else { return }

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
