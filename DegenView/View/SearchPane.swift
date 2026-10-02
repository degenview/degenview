import SwiftUI

enum SearchResultListSizing {
    case contentFitting(maxHeight: CGFloat)
    case fillAvailable
}

extension View {
    /// Result lists sit in a quiet rounded well, like the portfolio tables.
    fileprivate func searchResultsChrome() -> some View {
        self
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.6)))
    }
}

/// Shared source-grouped result list used by Add Chart and the Script Manager market picker.
struct TickerSearchResultList: View {
    let searchVM: TickerSearchViewModel
    let sources: [DataSourceType]
    let sizing: SearchResultListSizing
    var onCommitResult: ((TickerSearchResult) -> Void)? = nil

    private var rowCount: Int {
        sources.reduce(0) { $0 + (searchVM.searchResults[$1]?.count ?? 0) }
    }

    private var sectionCount: Int {
        sources.filter { !(searchVM.searchResults[$0] ?? []).isEmpty }.count
    }

    var body: some View {
        List {
            ForEach(Array(sources.enumerated()), id: \.element) { index, source in
                if let results = searchVM.searchResults[source], !results.isEmpty {
                    HStack(spacing: 6) {
                        SourceLogoView(source: source, size: 14)
                        Text(source.displayName)
                        Text("\(results.count)")
                            .font(.caption2.weight(.semibold))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .background(.quaternary, in: Capsule())
                    }
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
                    .listRowInsets(.init(top: 8, leading: 10, bottom: 2, trailing: 10))

                    ForEach(results) { result in
                        SearchResultRow(
                            result: result,
                            isSelected: searchVM.selectedResult == result,
                            onSelect: { searchVM.selectedResult = result },
                            onCommit: onCommitResult.map { commit in { commit(result) } }
                        )
                        .listRowSeparator(.hidden)
                        .listRowBackground(Color.clear)
                        .listRowInsets(.init(top: 1, leading: 6, bottom: 1, trailing: 6))
                    }

                    if hasNonemptySource(after: index) {
                        Divider()
                            .listRowSeparator(.hidden)
                            .listRowBackground(Color.clear)
                            .listRowInsets(.init(top: 4, leading: 10, bottom: 2, trailing: 10))
                    }
                }
            }
        }
        .searchResultsChrome()
        .modifier(
            SearchResultListSizeModifier(
                sizing: sizing, rowCount: rowCount, sectionCount: sectionCount))
    }

    private func hasNonemptySource(after index: Int) -> Bool {
        sources.dropFirst(index + 1).contains {
            !(searchVM.searchResults[$0] ?? []).isEmpty
        }
    }
}

private struct SearchResultListSizeModifier: ViewModifier {
    let sizing: SearchResultListSizing
    let rowCount: Int
    let sectionCount: Int

    func body(content: Content) -> some View {
        switch sizing {
        case .contentFitting(let maxHeight):
            content.frame(
                height: UI.searchResultsHeight(
                    rowCount: rowCount, sectionCount: sectionCount, maxHeight: maxHeight))
        case .fillAvailable:
            content.frame(maxHeight: .infinity)
        }
    }
}

/// Quick-fill suggestion chips shown before the user has typed a query.
struct SuggestionChipGrid: View {
    /// One chip: what it says, what it searches for, and the icon in front of it.
    struct Item: Hashable {
        enum Icon: Hashable {
            /// A coin or company logo, looked up for the chip's title as a ticker of the source.
            case logo(DataSourceType)
            case symbol(String)
            case none
        }

        let title: String
        let query: String
        var icon: Icon = .none
    }

    let caption: String
    let items: [Item]
    let onSelect: (Item) -> Void

    init(caption: String, items: [Item], onSelect: @escaping (Item) -> Void) {
        self.caption = caption
        self.items = items
        self.onSelect = onSelect
    }

    /// Tickers that search for themselves; with `iconSource`, each leads with its logo.
    init(
        caption: String, items: [String], iconSource: DataSourceType? = nil,
        onSelect: @escaping (String) -> Void
    ) {
        self.init(
            caption: caption,
            items: items.map { Item(title: $0, query: $0, icon: iconSource.map { .logo($0) } ?? .none) },
            onSelect: { onSelect($0.query) })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(caption, systemImage: "sparkles")
                .font(.subheadline.weight(.medium))
                .foregroundStyle(.secondary)

            LazyVGrid(
                columns: Array(repeating: .init(.flexible(), spacing: 8), count: UI.suggestionGridColumns), spacing: 8
            ) {
                ForEach(items, id: \.self) { item in
                    SuggestionChip(item: item) { onSelect(item) }
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.5)))
    }
}

private struct SuggestionChip: View {
    let item: SuggestionChipGrid.Item
    let action: () -> Void
    @State private var isHovered = false
    @State private var iconURL: URL?

    var body: some View {
        Button(action: action) {
            HStack(spacing: 7) {
                icon
                Text(item.title).lineLimit(1)
            }
            .font(.subheadline.weight(.medium))
            .foregroundStyle(.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity)
            .background(
                Color.primary.opacity(isHovered ? 0.09 : 0.05),
                in: RoundedRectangle(cornerRadius: 8, style: .continuous)
            )
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator.opacity(0.6)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
    }

    @ViewBuilder private var icon: some View {
        switch item.icon {
        case .logo(let source):
            TickerIconView(symbol: item.title, url: iconURL, size: 18)
                .task(id: item.title) {
                    iconURL = await IconResolver.shared.iconURL(
                        ticker: Self.marketTicker(item.title, source: source), source: source,
                        baseSymbol: item.title)
                }
        case .symbol(let name):
            Image(systemName: name)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Color.accentColor)
                .frame(width: 18, height: 18)
        case .none:
            EmptyView()
        }
    }

    /// The id the source knows the ticker by: Binance lists "BTC" as "BTCUSDT".
    private static func marketTicker(_ symbol: String, source: DataSourceType) -> String {
        source == .binance ? symbol + "USDT" : symbol
    }
}

/// Search text field with an inline progress spinner.
///
/// Shared by every search pane — crypto, stocks and prediction markets, in Add Chart and the picker.
struct SearchFieldRow: View {
    let placeholder: String
    @Binding var text: String
    let isSearching: Bool
    let onChange: (String) -> Void
    let onSubmit: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(focused ? Color.accentColor : .secondary)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .font(.body)
                .focused($focused)
                .onChange(of: text) { onChange(text) }
                .onSubmit { onSubmit() }
            if isSearching {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.8)
                    .frame(width: 16, height: 16)
            }
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .help("Clear search")
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: UI.searchFieldHeight)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 9, style: .continuous)
                .strokeBorder(focused ? Color.accentColor.opacity(0.7) : Color.primary.opacity(0.12))
        )
        .onAppear { focused = true }
    }
}

/// Confirmation strip showing what the user picked, before they commit to it.
///
/// Leads with the source's own artwork when the search payload carried one
/// (Polymarket markets do), and falls back to the source's SF Symbol.
struct SelectedResultBanner: View {
    /// "Selected" when adding, "New" when replacing an existing chart.
    let prefix: String
    let result: TickerSearchResult

    /// Event title for multi-choice PM; symbol for everything else.
    private var displayLabel: String {
        if let series = result.pmSeries, series.count > 1,
            let title = result.eventTitle, !title.isEmpty
        {
            return title
        }
        return result.symbol
    }

    var body: some View {
        HStack(spacing: 10) {
            if let url = result.imageURL {
                TickerIconView(symbol: result.symbol, url: url)
            } else {
                SourceLogoView(source: result.source)
            }

            VStack(alignment: .leading, spacing: 0) {
                Text(prefix).font(.caption2).foregroundStyle(.secondary)
                Text(displayLabel)
                    .font(.callout.weight(.semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
            }

            if let price = result.price {
                Text(PriceFormatter.headline(price, scale: result.source.priceScale))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Color.accentColor.opacity(0.35)))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(prefix): \(displayLabel)")
    }
}

/// Prediction-market search (Polymarket or Kalshi): field, grouped results, empty states.
///
/// Multi-choice groups show a group-level checkbox in the section header
/// plus individual toggles per row. Single-choice groups use tap-to-select.
struct PredictionMarketSearchPane: View {
    @ObservedObject var searchVM: PredictionMarketSearchViewModel
    @Binding var searchText: String
    /// A `List` has no intrinsic height, so in a sheet that sizes itself to its
    /// content it collapses to nothing without a floor.
    var resultsMinHeight: CGFloat = UI.addTickerResultsMinHeight
    var sizing: SearchResultListSizing
    var showsStatus = true
    /// When set, a provider dropdown (Polymarket / Kalshi) leads the search field.
    var provider: Binding<DataSourceType>? = nil
    /// Topic chips shown while the search box is empty; nil leaves the pane bare.
    var suggestions: [SuggestionChipGrid.Item]? = nil
    var onCommitResult: ((TickerSearchResult) -> Void)? = nil

    private var resultHeight: CGFloat {
        let rows = searchVM.groups.reduce(0) {
            $0 + (searchVM.isExpanded($1) ? $1.results.count : 0)
        }
        let calculated =
            CGFloat(rows) * UI.searchResultRowHeight
            + CGFloat(searchVM.groups.count) * UI.searchResultSectionHeight
            + UI.searchResultListInsets
        if case .contentFitting(let maxHeight) = sizing {
            return min(maxHeight, max(resultsMinHeight, calculated))
        }
        return calculated
    }

    private var isQueryEmpty: Bool { searchText.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(spacing: 16) {
            HStack(spacing: 8) {
                if let provider {
                    PredictionProviderMenu(provider: provider)
                }

                SearchFieldRow(
                    placeholder: "Search markets (e.g. Bitcoin, Fed, election)",
                    text: $searchText,
                    isSearching: searchVM.isSearching,
                    onChange: { searchVM.scheduleSearch(query: $0) },
                    onSubmit: {
                        if let first = searchVM.firstAvailableResult {
                            searchVM.selectedResult = first
                        }
                    }
                )
            }

            if let suggestions, isQueryEmpty {
                SuggestionChipGrid(caption: "Popular topics", items: suggestions) { item in
                    // Typing fills the box; the field's own change handler runs the search.
                    searchText = item.query
                }
                if searchVM.isShowingTrending && !searchVM.hasResults {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Loading trending markets…").font(.caption).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 4)
                }
            }

            if searchVM.hasResults {
                List {
                    if searchVM.isShowingTrending {
                        Label("Trending on \(searchVM.provider.displayName)", systemImage: "flame.fill")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .listRowSeparator(.hidden)
                            .listRowBackground(Color.clear)
                            .listRowInsets(.init(top: 8, leading: 10, bottom: 2, trailing: 10))
                    }
                    ForEach(Array(searchVM.groups.enumerated()), id: \.element.id) { index, group in
                        groupHeader(for: group)
                            .listRowSeparator(.hidden)
                            .listRowBackground(Color.clear)
                            .listRowInsets(.init(top: 6, leading: 10, bottom: 2, trailing: 10))

                        if searchVM.isExpanded(group) {
                            ForEach(group.results) { result in
                                marketRow(result, in: group)
                                    .listRowSeparator(.hidden)
                                    .listRowBackground(Color.clear)
                                    .listRowInsets(.init(top: 1, leading: 6, bottom: 1, trailing: 6))
                            }
                        }

                        if index < searchVM.groups.count - 1 {
                            Divider()
                                .listRowSeparator(.hidden)
                                .listRowBackground(Color.clear)
                                .listRowInsets(.init(top: 4, leading: 10, bottom: 2, trailing: 10))
                        }
                    }
                }
                .searchResultsChrome()
                .modifier(PredictionMarketListSizeModifier(sizing: sizing, height: resultHeight))
            }

            if showsStatus, let error = searchVM.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if showsStatus, !searchVM.isSearching,
                !searchText.trimmingCharacters(in: .whitespaces).isEmpty,
                !searchVM.hasResults
            {
                Text("No markets found")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .task {
            // First appearance with nothing typed: fetch the trending list.
            if suggestions != nil, isQueryEmpty, !searchVM.hasResults { searchVM.scheduleSearch(query: "") }
        }
    }

    @ViewBuilder
    private func marketRow(
        _ result: TickerSearchResult, in group: PredictionMarketResultGroup
    ) -> some View {
        if group.results.count > 1 {
            PredictionMarketResultRow(
                result: result,
                isChecked: searchVM.checkedChoices[result.fullSymbol] ?? false,
                onToggle: { searchVM.toggleChoice(result.fullSymbol) },
                onCommit: onCommitResult.map { commit in { commit(result) } }
            )
        } else {
            PredictionMarketResultRow(
                result: result,
                isSelected: searchVM.selectedResult == result,
                onSelect: { searchVM.selectedResult = result },
                onCommit: onCommitResult.map { commit in { commit(result) } }
            )
        }
    }

    /// Disclosure and selection are separate controls so either can be changed independently.
    private func groupHeader(for group: PredictionMarketResultGroup) -> some View {
        let allChecked = searchVM.isGroupChecked(group)
        let anyChecked = searchVM.isGroupAnyChecked(group)
        let iconName =
            allChecked
            ? "checkmark.square.fill"
            : anyChecked ? "minus.square.fill" : "square"

        return HStack(spacing: 10) {
            if group.results.count > 1 {
                Button {
                    searchVM.toggleGroup(group)
                } label: {
                    Image(systemName: iconName)
                        .foregroundStyle(anyChecked ? Color.accentColor : Color.secondary)
                        .font(.body)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Select all choices in \(group.eventTitle)")
            }

            Button {
                searchVM.toggleExpansion(group)
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: searchVM.isExpanded(group) ? "chevron.down" : "chevron.right")
                        .font(.caption.weight(.semibold))
                    if let result = group.results.first {
                        TickerIconView(
                            symbol: result.symbol,
                            url: result.imageURL,
                            size: UI.predictionMarketRowImageSize
                        )
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(group.eventTitle)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(2)
                        Text(
                            "\(group.results.count) choice\(group.results.count == 1 ? "" : "s") · \(searchVM.isExpanded(group) ? "Hide Choices" : "Show Choices")"
                        )
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(group.eventTitle), \(group.results.count) choices")
            .accessibilityValue(searchVM.isExpanded(group) ? "Expanded" : "Collapsed")
            .accessibilityHint(searchVM.isExpanded(group) ? "Hide Choices" : "Show Choices")
        }
        .padding(.vertical, 4)
    }
}

private struct PredictionMarketListSizeModifier: ViewModifier {
    let sizing: SearchResultListSizing
    let height: CGFloat

    func body(content: Content) -> some View {
        switch sizing {
        case .contentFitting:
            content.frame(height: height)
        case .fillAvailable:
            content.frame(maxHeight: .infinity)
        }
    }
}

/// The Polymarket / Kalshi switch: styled like the search box beside it, at the same height,
/// with the provider's logo and a single chevron (the system menu indicator is hidden).
private struct PredictionProviderMenu: View {
    @Binding var provider: DataSourceType

    var body: some View {
        Menu {
            ForEach(DataSourceType.predictionMarkets, id: \.self) { source in
                Button {
                    provider = source
                } label: {
                    if source == provider {
                        Label(source.displayName, systemImage: "checkmark")
                    } else {
                        Text(source.displayName)
                    }
                }
            }
        } label: {
            HStack(spacing: 8) {
                SourceLogoView(source: provider, size: 18)
                    .frame(width: 18, height: 18)
                Text(provider.displayName).font(.body.weight(.medium)).foregroundStyle(.primary)
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .frame(height: UI.searchFieldHeight)
            .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).strokeBorder(Color.primary.opacity(0.12)))
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Prediction market provider")
        .accessibilityLabel("Provider, \(provider.displayName)")
    }
}
