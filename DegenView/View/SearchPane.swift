import SwiftUI

enum SearchResultListSizing {
    case contentFitting(maxHeight: CGFloat)
    case fillAvailable
}

/// Shared source-grouped result list used by Add Ticker and Chart Settings.
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
                    Label(source.displayName, systemImage: source.icon)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .listRowSeparator(.hidden)
                        .listRowInsets(.init(top: 4, leading: 8, bottom: 2, trailing: 8))

                    ForEach(results) { result in
                        SearchResultRow(
                            result: result,
                            isSelected: searchVM.selectedResult == result,
                            onSelect: { searchVM.selectedResult = result },
                            onCommit: onCommitResult.map { commit in { commit(result) } }
                        )
                        .listRowSeparator(.hidden)
                        .listRowInsets(.init(top: 2, leading: 8, bottom: 2, trailing: 8))
                    }

                    if hasNonemptySource(after: index) {
                        Divider()
                            .listRowSeparator(.hidden)
                            .listRowInsets(.init(top: 2, leading: 8, bottom: 2, trailing: 8))
                    }
                }
            }
        }
        .listStyle(.inset)
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

/// Search text field with an inline progress spinner.
///
/// Shared by every search pane in both sheets — crypto and Polymarket, add and edit.
struct SearchFieldRow: View {
    let placeholder: String
    @Binding var text: String
    let isSearching: Bool
    let onChange: (String) -> Void
    let onSubmit: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            ZStack(alignment: .trailing) {
                TextField(placeholder, text: $text)
                    .textFieldStyle(.roundedBorder)
                    .font(.body)
                    .padding(.trailing, text.isEmpty ? 0 : 28)
                    .onChange(of: text) { onChange(text) }
                    .onSubmit { onSubmit() }

                if !text.isEmpty {
                    Button {
                        text = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                    .padding(.trailing, 6)
                    .help("Clear search")
                    .accessibilityLabel("Clear search")
                }
            }

            if isSearching {
                ProgressView()
                    .scaleEffect(0.7)
                    .frame(width: 20, height: 20)
            }
        }
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
        HStack(spacing: 8) {
            if let url = result.imageURL {
                TickerIconView(symbol: result.symbol, url: url)
            } else {
                Image(systemName: result.source.icon)
                    .foregroundStyle(.secondary)
            }

            Text("\(prefix): \(displayLabel)")
                .font(.callout.weight(.medium))
                .foregroundStyle(.primary)
                .lineLimit(2)

            if let price = result.price {
                Text(PriceFormatter.headline(price, scale: result.source.priceScale))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 0)
        }
        .padding(8)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
    }
}

/// Polymarket market search: field, grouped results, empty states.
///
/// Multi-choice groups show a group-level checkbox in the section header
/// plus individual toggles per row. Single-choice groups use tap-to-select.
struct PolymarketSearchPane: View {
    @ObservedObject var searchVM: PolymarketSearchViewModel
    @Binding var searchText: String
    /// A `List` has no intrinsic height, so in a sheet that sizes itself to its
    /// content it collapses to nothing without a floor.
    var resultsMinHeight: CGFloat = UI.addTickerResultsMinHeight
    var sizing: SearchResultListSizing
    var showsStatus = true
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

    var body: some View {
        VStack(spacing: 12) {
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

            if searchVM.hasResults {
                List {
                    ForEach(Array(searchVM.groups.enumerated()), id: \.element.id) { index, group in
                        groupHeader(for: group)
                            .listRowSeparator(.hidden)
                            .listRowInsets(.init(top: 4, leading: 8, bottom: 2, trailing: 8))

                        if searchVM.isExpanded(group) {
                            ForEach(group.results) { result in
                                polymarketRow(result, in: group)
                                    .listRowSeparator(.hidden)
                                    .listRowInsets(.init(top: 2, leading: 8, bottom: 2, trailing: 8))
                            }
                        }

                        if index < searchVM.groups.count - 1 {
                            Divider()
                                .listRowSeparator(.hidden)
                                .listRowInsets(.init(top: 2, leading: 8, bottom: 2, trailing: 8))
                        }
                    }
                }
                .listStyle(.inset)
                .modifier(PolymarketListSizeModifier(sizing: sizing, height: resultHeight))
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
    }

    @ViewBuilder
    private func polymarketRow(
        _ result: TickerSearchResult, in group: PolymarketResultGroup
    ) -> some View {
        if group.results.count > 1 {
            PolymarketResultRow(
                result: result,
                isChecked: searchVM.checkedChoices[result.fullSymbol] ?? false,
                onToggle: { searchVM.toggleChoice(result.fullSymbol) },
                onCommit: onCommitResult.map { commit in { commit(result) } }
            )
        } else {
            PolymarketResultRow(
                result: result,
                isSelected: searchVM.selectedResult == result,
                onSelect: { searchVM.selectedResult = result },
                onCommit: onCommitResult.map { commit in { commit(result) } }
            )
        }
    }

    /// Disclosure and selection are separate controls so either can be changed independently.
    private func groupHeader(for group: PolymarketResultGroup) -> some View {
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
                            size: UI.polymarketRowImageSize
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

private struct PolymarketListSizeModifier: ViewModifier {
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
