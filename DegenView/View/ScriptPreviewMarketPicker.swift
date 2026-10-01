import SwiftUI

/// Popover for choosing the preview's market. Crypto and stocks only: a script runs on candles,
/// and the other chart types — prediction markets, CoinMarketCap indices, portfolios — have none.
struct ScriptPreviewMarketPicker: View {
    /// The kinds of market the preview can chart.
    static let assetTypes: [ChartAssetType] = [.crypto, .stock]

    let current: PreviewMarket
    let recents: [PreviewMarket]
    let onSelect: (PreviewMarket) -> Void

    @StateObject private var cryptoVM = TickerSearchViewModel(logPrefix: "[ScriptPreview]")
    @StateObject private var stockVM = TickerSearchViewModel(
        logPrefix: "[ScriptPreview/Stocks]",
        sources: { [DataSourceFactory.shared.alpaca] }
    )
    @State private var assetType: ChartAssetType
    @State private var cryptoText = ""
    @State private var stockText = ""

    @Environment(\.openSettings) private var openSettings
    @Environment(\.dismiss) private var dismiss

    init(current: PreviewMarket, recents: [PreviewMarket], onSelect: @escaping (PreviewMarket) -> Void) {
        self.current = current
        self.recents = recents
        self.onSelect = onSelect
        _assetType = State(initialValue: current.source == .alpaca ? .stock : .crypto)
    }

    private var searchVM: TickerSearchViewModel { assetType == .stock ? stockVM : cryptoVM }
    private var searchText: String { assetType == .stock ? stockText : cryptoText }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Picker("Market type", selection: $assetType) {
                ForEach(Self.assetTypes) { type in Text(type.rawValue).tag(type) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            switch assetType {
            case .stock: stockSearch
            default: cryptoSearch
            }
        }
        .padding(14)
        .frame(width: 380, height: 440)
        .onChange(of: cryptoVM.selectedResult) { _, result in commit(result) }
        .onChange(of: stockVM.selectedResult) { _, result in commit(result) }
        .onDisappear {
            cryptoVM.cancelSearch()
            stockVM.cancelSearch()
        }
    }

    // MARK: - Panes

    private var cryptoSearch: some View {
        VStack(spacing: 12) {
            SearchFieldRow(
                placeholder: "Symbol (e.g. BTC, ETH, PEPE)",
                text: $cryptoText,
                isSearching: cryptoVM.isSearching,
                onChange: { cryptoVM.scheduleSearch(query: $0) },
                onSubmit: { cryptoVM.selectedResult = cryptoVM.firstAvailableResult }
            )
            results(cryptoVM, sources: cryptoVM.orderedSources, text: cryptoText)
        }
    }

    private var stockSearch: some View {
        VStack(spacing: 12) {
            SearchFieldRow(
                placeholder: "US stock symbol or company name",
                text: $stockText,
                isSearching: stockVM.isSearching,
                onChange: { stockVM.scheduleSearch(query: $0) },
                onSubmit: { stockVM.selectedResult = stockVM.firstAvailableResult }
            )
            if !AlpacaCredentialsStore.isConfigured {
                HStack(spacing: 8) {
                    Label("Stocks need Alpaca market data.", systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Open Settings") {
                        UserDefaults.standard.set(SettingsTab.alpaca.rawValue, forKey: "settingsTab")
                        openSettings()
                    }
                    .controlSize(.small)
                }
            }
            results(stockVM, sources: [.alpaca], text: stockText)
        }
    }

    @ViewBuilder
    private func results(_ searchVM: TickerSearchViewModel, sources: [DataSourceType], text: String) -> some View {
        if sources.contains(where: { !(searchVM.searchResults[$0] ?? []).isEmpty }) {
            TickerSearchResultList(searchVM: searchVM, sources: sources, sizing: .fillAvailable)
        } else if text.trimmingCharacters(in: .whitespaces).isEmpty {
            recentsList
        } else if !searchVM.isSearching {
            Text("No markets found")
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Spacer()
        }
    }

    /// Markets tried before, newest first — a click to switch back, no search.
    private var recentsList: some View {
        VStack(alignment: .leading, spacing: 6) {
            let shown = recents.filter { ($0.source == .alpaca) == (assetType == .stock) }
            if shown.isEmpty {
                Text("Search for a market to run this script on.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Text("Recent").font(.caption).foregroundStyle(.secondary)
                ForEach(shown) { market in
                    Button {
                        select(market)
                    } label: {
                        HStack(spacing: 8) {
                            SourceLogoView(source: market.source, size: 16)
                            Text(market.ticker.uppercased()).font(.body.weight(.medium))
                            Text(market.source.displayName).font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            if market == current { Image(systemName: "checkmark").foregroundStyle(.tint) }
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .padding(.vertical, 4)
                }
                Spacer(minLength: 0)
            }
        }
    }

    // MARK: - Selection

    private func commit(_ result: TickerSearchResult?) {
        guard let result else { return }
        select(PreviewMarket(ticker: result.fullSymbol, source: result.source))
    }

    private func select(_ market: PreviewMarket) {
        onSelect(market)
        dismiss()
    }
}
