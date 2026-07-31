import SwiftUI

struct EmptyStateView: View {
    /// Offered as a shortcut so a fresh tab can be filled in one click instead
    /// of rebuilding a chart set by hand.
    let savedViews: [SavedView]
    let onAddTapped: () -> Void
    let onOpenView: (SavedView) -> Void

    var body: some View {
        VStack(spacing: 20) {
            Spacer()

            Image(systemName: "chart.line.uptrend.xyaxis")
                .font(.system(size: 56))
                .foregroundStyle(.secondary)

            Text("No Charts Yet")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Add a crypto ticker to start tracking prices.")
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 240)

            Button(action: onAddTapped) {
                Label("Add Your First Ticker", systemImage: "plus")
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

            if !savedViews.isEmpty {
                savedViewList
            }

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Saved views

    private var savedViewList: some View {
        VStack(spacing: 8) {
            Label("or open a saved view", systemImage: "folder")
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.top, 4)

            ScrollView {
                VStack(spacing: 2) {
                    ForEach(savedViews) { view in
                        SavedViewRow(view: view) { onOpenView(view) }
                    }
                }
            }
            .scrollIndicators(.automatic)
            .frame(maxHeight: UI.emptyStateViewListMaxHeight)
        }
        .frame(maxWidth: UI.emptyStateViewListWidth)
    }
}

/// One saved view offered on an empty tab, with enough detail to tell two
/// similar view names apart.
private struct SavedViewRow: View {
    let view: SavedView
    let onSelect: () -> Void

    @State private var isHovering = false

    private var subtitle: String {
        let count = view.resolvedConfigs.count
        let charts = count == 1 ? "1 chart" : "\(count) charts"
        return "\(charts) · \(view.timeRange.rawValue)"
    }

    var body: some View {
        Button(action: onSelect) {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(view.name)
                        .font(.body.weight(.medium))
                    Text(subtitle)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Image(systemName: "chevron.right")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .contentShape(Rectangle())
            .background(isHovering ? Color.accentColor.opacity(0.15) : Color.clear)
            .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
    }
}

#Preview {
    EmptyStateView(savedViews: [], onAddTapped: {}, onOpenView: { _ in })
}
