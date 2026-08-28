import SwiftUI

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
    var resultsMaxHeight: CGFloat
    var usesAdaptiveResultHeight = false
    var showsStatus = true
    var onCommitResult: ((TickerSearchResult) -> Void)? = nil

    private var resultHeight: CGFloat {
        let rows = searchVM.groups.reduce(0) { $0 + $1.results.count }
        let calculated = CGFloat(rows) * UI.searchResultRowHeight
            + CGFloat(searchVM.groups.count) * UI.searchResultSectionHeight
            + UI.searchResultListInsets
        return min(resultsMaxHeight, max(resultsMinHeight, calculated))
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
                    ForEach(searchVM.groups) { group in
                        Section {
                            ForEach(group.results) { result in
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
                        } header: {
                            if group.results.count > 1 {
                                groupHeader(for: group)
                            } else {
                                Text(group.eventTitle)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                        }
                    }
                }
                .listStyle(.inset)
                .frame(height: usesAdaptiveResultHeight ? resultHeight : nil)
                .frame(minHeight: usesAdaptiveResultHeight ? nil : resultsMinHeight,
                       maxHeight: usesAdaptiveResultHeight ? nil : resultsMaxHeight)
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
    }

    /// Section header with a group-level toggle checkbox for multi-choice events.
    private func groupHeader(for group: PolymarketResultGroup) -> some View {
        let allChecked = searchVM.isGroupChecked(group)
        let anyChecked = searchVM.isGroupAnyChecked(group)
        let iconName =
            allChecked
            ? "checkmark.square.fill"
            : anyChecked ? "minus.square.fill" : "square"

        return Button {
            searchVM.toggleGroup(group)
        } label: {
            HStack(spacing: 4) {
                Image(systemName: iconName)
                    .foregroundStyle(anyChecked ? Color.accentColor : Color.secondary)
                    .font(.caption)
                Text(group.eventTitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .buttonStyle(.plain)
    }
}
