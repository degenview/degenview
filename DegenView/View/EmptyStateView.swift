import SwiftUI

/// What a new tab shows before it has a chart: a search launcher, then one-click ways back
/// into the user's own markets — favorites, recently added markets and saved views.
struct EmptyStateView: View {
    let suggestions: EmptyStateSuggestions
    /// Whether to offer stocks too; they need an Alpaca feed, so a chip would go nowhere without one.
    let offersStocks: Bool
    let onAddTapped: () -> Void
    let onOpenFavorite: (FavoriteItem) -> Void
    let onOpenRecent: (RecentMarket) -> Void
    let onOpenView: (SavedView) -> Void
    let onOpenSuggestion: (TickerSearchResult) -> Void
    let onRemoveRecent: (RecentMarket) -> Void
    let onClearRecents: () -> Void
    let onShowFavorites: () -> Void

    var body: some View {
        GeometryReader { proxy in
            ScrollView {
                VStack(spacing: 28) {
                    hero
                    launcher
                    if suggestions.hasAny {
                        sections
                    } else {
                        starterSections
                        tip
                    }
                }
                .frame(maxWidth: UI.emptyStateMaxWidth)
                .padding(.horizontal, 24)
                .padding(.vertical, 36)
                .frame(maxWidth: .infinity, minHeight: proxy.size.height)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
    }

    // MARK: - Hero

    private var hero: some View {
        VStack(spacing: 12) {
            Image(systemName: "chart.line.uptrend.xyaxis")
                .font(.system(size: 28, weight: .semibold))
                .foregroundStyle(.tint)
                .frame(width: 64, height: 64)
                .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .accessibilityHidden(true)

            VStack(spacing: 4) {
                Text("Start a new chart")
                    .font(.title2.weight(.semibold))
                Text(
                    suggestions.hasAny
                        ? "Search a market, or pick up where you left off."
                        : "Search crypto, stocks and prediction markets."
                )
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            }
        }
    }

    private var launcher: some View {
        SearchLauncher(action: onAddTapped)
    }

    private var tip: some View {
        Label("Save a layout with Save View and it will show up here.", systemImage: "lightbulb")
            .font(.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
    }

    // MARK: - Sections

    private var sections: some View {
        VStack(spacing: 16) {
            if !suggestions.favorites.isEmpty {
                favoritesSection
            }
            if !suggestions.recents.isEmpty {
                recentsSection
            }
            if !suggestions.savedViews.isEmpty {
                savedViewsSection
            }
        }
    }

    /// Nothing of the user's own yet, so offer the same starting points as Add Chart.
    private var starterSections: some View {
        VStack(spacing: 16) {
            SuggestionSection(title: "Suggestions", systemImage: "sparkles") {
                EmptyView()
            } content: {
                LazyVGrid(columns: starterColumns, spacing: 8) {
                    ForEach(MarketSuggestions.crypto, id: \.self) { base in
                        MarketChip(ticker: "\(base)USDT", source: .binance, displayName: nil, title: base) {
                            onOpenSuggestion(MarketSuggestions.cryptoResult(base))
                        }
                    }
                }
            }
            if offersStocks {
                SuggestionSection(title: "Popular stocks and ETFs", systemImage: "building.columns") {
                    EmptyView()
                } content: {
                    LazyVGrid(columns: starterColumns, spacing: 8) {
                        ForEach(MarketSuggestions.stocks, id: \.self) { symbol in
                            MarketChip(ticker: symbol, source: .alpaca, displayName: nil, title: symbol) {
                                onOpenSuggestion(MarketSuggestions.stockResult(symbol))
                            }
                        }
                    }
                }
            }
        }
    }

    private var starterColumns: [GridItem] {
        [GridItem(.adaptive(minimum: UI.emptyStateStarterChipMinWidth), spacing: 8)]
    }

    private var chipColumns: [GridItem] {
        [GridItem(.adaptive(minimum: UI.emptyStateChipMinWidth), spacing: 8)]
    }

    private var favoritesSection: some View {
        SuggestionSection(title: "Favorites", systemImage: "star.fill") {
            if suggestions.hiddenFavoriteCount > 0 {
                Button("\(suggestions.hiddenFavoriteCount) more", action: onShowFavorites)
                    .buttonStyle(.link)
                    .font(.caption)
                    .help("Show the favorites sidebar")
            }
        } content: {
            LazyVGrid(columns: chipColumns, spacing: 8) {
                ForEach(suggestions.favorites) { item in
                    MarketChip(
                        ticker: item.config.symbol, source: item.config.source,
                        displayName: item.config.displayName,
                        title: item.ticker, subtitle: item.name
                    ) { onOpenFavorite(item) }
                }
            }
        }
    }

    private var recentsSection: some View {
        SuggestionSection(title: "Recent", systemImage: "clock.arrow.circlepath") {
            Button("Clear", action: onClearRecents)
                .buttonStyle(.link)
                .font(.caption)
        } content: {
            LazyVGrid(columns: chipColumns, spacing: 8) {
                ForEach(suggestions.recents) { market in
                    MarketChip(
                        ticker: market.fullSymbol, source: market.source, displayName: nil,
                        title: market.symbol, subtitle: market.source.displayName
                    ) { onOpenRecent(market) }
                    .contextMenu {
                        Button("Remove from Recent") { onRemoveRecent(market) }
                    }
                }
            }
        }
    }

    private var savedViewsSection: some View {
        SuggestionSection(title: "Saved Views", systemImage: "folder.fill") {
            EmptyView()
        } content: {
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: UI.emptyStateCardMinWidth), spacing: 8)], spacing: 8
            ) {
                ForEach(suggestions.savedViews) { view in
                    SavedViewCard(view: view) { onOpenView(view) }
                }
            }
        }
    }
}

// MARK: - Launcher

/// A search box that is really a button: it opens Add Chart, where the typing happens.
private struct SearchLauncher: View {
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(isHovered ? Color.accentColor : .secondary)
                Text("Search crypto, stocks, prediction markets…")
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            .font(.body)
            .padding(.horizontal, 14)
            .frame(minHeight: UI.emptyStateLauncherHeight)
            .background(
                .quaternary.opacity(isHovered ? 0.7 : 0.5),
                in: RoundedRectangle(cornerRadius: 12, style: .continuous)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(isHovered ? Color.accentColor.opacity(0.6) : Color.primary.opacity(0.12))
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovered)
        .help("Add Chart")
        .accessibilityLabel("Add Chart")
        .accessibilityHint("Search for a market")
    }
}

// MARK: - Sections

/// A titled card, like the suggestion and recents cards in the Add Chart sheet.
private struct SuggestionSection<Accessory: View, Content: View>: View {
    let title: String
    let systemImage: String
    @ViewBuilder let accessory: Accessory
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(title, systemImage: systemImage)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                accessory
            }
            content
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.5)))
    }
}

/// The button every chip and card is: a quiet tile that picks up the accent on hover.
private struct SuggestionTile<Content: View>: View {
    let action: () -> Void
    @ViewBuilder let content: Content
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            content
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    Color.primary.opacity(isHovered ? 0.09 : 0.05),
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(isHovered ? Color.accentColor.opacity(0.45) : Color.primary.opacity(0.1))
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovered)
    }
}

// MARK: - Items

/// A coin's artwork, resolved the way a chart header does it.
struct MarketIcon: View {
    @StateObject private var viewModel: ChartViewModel
    let size: CGFloat

    init(ticker: String, source: DataSourceType, displayName: String?, size: CGFloat) {
        _viewModel = StateObject(
            wrappedValue: ChartViewModel(ticker: ticker, source: source, displayName: displayName))
        self.size = size
    }

    var body: some View {
        ChartIconView(viewModel: viewModel, size: size, showsSource: false)
    }
}

/// One market offered as a shortcut: artwork, ticker and a secondary line.
private struct MarketChip: View {
    let ticker: String
    let source: DataSourceType
    let displayName: String?
    let title: String
    /// A second line under the title; chips without one are just an icon and a ticker.
    var subtitle: String? = nil
    let action: () -> Void

    var body: some View {
        SuggestionTile(action: action) {
            HStack(spacing: 8) {
                MarketIcon(ticker: ticker, source: source, displayName: displayName, size: 24)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title)
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if let subtitle {
                        Text(subtitle)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
            }
        }
        .help("Add \(title) to this tab")
        .accessibilityLabel(subtitle.map { "\(title), \($0)" } ?? title)
    }
}

/// One saved view: its name, what is in it, and the first few markets as artwork.
private struct SavedViewCard: View {
    let view: SavedView
    let action: () -> Void

    private var subtitle: String {
        let count = view.tickerConfigs.count
        let charts = count == 1 ? "1 chart" : "\(count) charts"
        return "\(charts) · \(view.timeRange.rawValue)"
    }

    private var previews: [TickerConfig] { EmptyStateSuggestions.previewConfigs(of: view) }

    var body: some View {
        SuggestionTile(action: action) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Text(view.name)
                        .font(.callout.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tertiary)
                }
                HStack(spacing: 0) {
                    Text(subtitle)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 6)
                    HStack(spacing: -5) {
                        ForEach(previews, id: \.chartID) { config in
                            MarketIcon(
                                ticker: config.symbol, source: config.source,
                                displayName: config.displayName, size: 18
                            )
                            .overlay(Circle().strokeBorder(Color(nsColor: .windowBackgroundColor), lineWidth: 1.5))
                        }
                    }
                }
            }
        }
        .help("Open \(view.name)")
        .accessibilityLabel("\(view.name), \(subtitle)")
    }
}

#Preview("Full") {
    let sample = EmptyStateSuggestions(
        favorites: [
            FavoriteItem(name: "Bitcoin", ticker: "BTC", config: TickerConfig(symbol: "BTCUSDT", source: .binance)),
            FavoriteItem(name: "Ethereum", ticker: "ETH", config: TickerConfig(symbol: "ETH-USD", source: .coinbase)),
        ],
        recents: [
            RecentMarket(
                TickerSearchResult(
                    symbol: "SOL/USDT", fullSymbol: "SOLUSDT", source: .binance, price: nil, metadata: [:]))
        ],
        savedViews: [
            SavedView(
                name: "Majors", tickers: ["BTCUSDT", "ETHUSDT"], timeRange: .oneDay, createdAt: Date(),
                tickerConfigs: [
                    TickerConfig(symbol: "BTCUSDT", source: .binance),
                    TickerConfig(symbol: "ETHUSDT", source: .binance),
                ], candleCount: 100)
        ])
    EmptyStateView(
        suggestions: sample, offersStocks: true, onAddTapped: {}, onOpenFavorite: { _ in },
        onOpenRecent: { _ in }, onOpenView: { _ in }, onOpenSuggestion: { _ in }, onRemoveRecent: { _ in },
        onClearRecents: {}, onShowFavorites: {}
    )
    .frame(width: 700, height: 600)
}

#Preview("Empty") {
    EmptyStateView(
        suggestions: EmptyStateSuggestions(favorites: [], recents: [], savedViews: []), offersStocks: true,
        onAddTapped: {}, onOpenFavorite: { _ in }, onOpenRecent: { _ in }, onOpenView: { _ in },
        onOpenSuggestion: { _ in }, onRemoveRecent: { _ in }, onClearRecents: {}, onShowFavorites: {}
    )
    .frame(width: 700, height: 600)
}
