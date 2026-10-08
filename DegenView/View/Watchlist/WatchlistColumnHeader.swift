import SwiftUI

/// Column titles. Clicking one sorts by it; the stored order is untouched and "Manual" returns to it.
struct WatchlistColumnHeader: View {
    let display: WatchlistDisplaySettings
    let onSort: (WatchlistSortKey) -> Void

    var body: some View {
        HStack(spacing: 6) {
            sortButton("Symbol", key: .symbol)
            Spacer(minLength: 4)
            ForEach(display.columns) { column in
                if let key = column.sortKey {
                    sortButton(column.title, key: key, alignment: .trailing)
                        .frame(width: column.width, alignment: .trailing)
                        .accessibilityLabel(column.accessibilityTitle)
                } else {
                    Text(column.title)
                        .frame(width: column.width, alignment: .trailing)
                        .accessibilityLabel(column.accessibilityTitle)
                }
            }
        }
        .font(.caption2.weight(.medium))
        .foregroundStyle(.secondary)
        .watchlistHorizontalInsets()
        .padding(.vertical, 4)
    }

    private func sortButton(_ title: String, key: WatchlistSortKey, alignment: Alignment = .leading) -> some View {
        let active = display.sort.key == key
        return Button {
            onSort(key)
        } label: {
            HStack(spacing: 2) {
                Text(title)
                if active {
                    Image(systemName: display.sort.ascending ? "chevron.up" : "chevron.down")
                        .font(.system(size: 7, weight: .bold))
                }
            }
            .frame(maxWidth: alignment == .trailing ? .infinity : nil, alignment: alignment)
            .foregroundStyle(active ? Color.primary : Color.secondary)
        }
        .buttonStyle(.plain)
        .accessibilityValue(active ? (display.sort.ascending ? "sorted ascending" : "sorted descending") : "")
    }
}
