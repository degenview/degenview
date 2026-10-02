import SwiftUI

/// The markets the user picked before, shown under the suggestions while the search box is empty.
struct RecentMarketsCard: View {
    let markets: [RecentMarket]
    let selected: TickerSearchResult?
    let onSelect: (TickerSearchResult) -> Void
    let onCommit: (TickerSearchResult) -> Void
    let onRemove: (RecentMarket) -> Void
    let onClear: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Recent", systemImage: "clock.arrow.circlepath")
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Clear", action: onClear)
                    .buttonStyle(.link)
                    .font(.caption)
            }
            LazyVGrid(
                columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 2
            ) {
                ForEach(markets) { market in
                    let result = market.result
                    SearchResultRow(
                        result: result, isSelected: selected == result, onSelect: { onSelect(result) },
                        onCommit: { onCommit(result) }, showsSource: true
                    )
                    .contextMenu {
                        Button("Remove from Recent") { onRemove(market) }
                    }
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.5)))
    }
}
