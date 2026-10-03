import SwiftUI

/// "Open layout…": every saved layout, for the long tail the toolbar's recent list doesn't reach.
/// Click or arrow to select, double-click or Return to open, Delete to remove.
struct SavedLayoutPickerSheet: View {
    @ObservedObject var layout: SavedLayoutController
    @ObservedObject var store: SavedViewStore
    @Environment(\.dismiss) private var dismiss

    @State private var selection: UUID?
    @State private var query = ""
    @State private var pendingDelete: SavedView?

    /// Below this many layouts the list fits on screen and a search field is only noise.
    private static let searchThreshold = 6

    /// Most recently opened first; layouts that were never opened follow by name.
    private var sorted: [SavedView] {
        store.views.sorted { lhs, rhs in
            switch (lhs.lastOpenedAt, rhs.lastOpenedAt) {
            case (let l?, let r?): return l > r
            case (nil, _?): return false
            case (_?, nil): return true
            case (nil, nil): return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
            }
        }
    }

    private var visible: [SavedView] {
        let trimmed = query.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return sorted }
        return sorted.filter { $0.name.localizedCaseInsensitiveContains(trimmed) }
    }

    private var selectedView: SavedView? { visible.first { $0.id == selection } }
    private var showsSearch: Bool { store.views.count >= Self.searchThreshold }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeader(
                systemImage: "rectangle.3.group.fill", title: "Open Layout", subtitle: subtitle
            )
            .padding(.horizontal, 24)
            .padding(.top, 24)
            .padding(.bottom, 16)

            if showsSearch {
                searchField
                    .padding(.horizontal, 24)
                    .padding(.bottom, 12)
            }

            content

            Divider()
            footer
        }
        .frame(width: 540, height: 500)
        .onAppear { selection = initialSelection }
        .onChange(of: query) { _, _ in
            if selectedView == nil { selection = visible.first?.id }
        }
        .confirmationDialog(
            "Delete “\(pendingDelete?.name ?? "layout")”?",
            isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }),
            presenting: pendingDelete
        ) { view in
            Button("Delete Layout", role: .destructive) {
                layout.delete(view.id)
                pendingDelete = nil
            }
            Button("Cancel", role: .cancel) { pendingDelete = nil }
        } message: { _ in
            Text("Tabs showing it keep their charts but become Unnamed. This can’t be undone.")
        }
    }

    /// The layout this tab is on, so Return reopens it; otherwise the most recent.
    private var initialSelection: UUID? {
        if let active = layout.activeViewID, visible.contains(where: { $0.id == active }) { return active }
        return visible.first?.id
    }

    private var subtitle: String {
        let count = store.views.count
        guard count > 0 else { return "Nothing saved yet." }
        let total = count == 1 ? "1 layout" : "\(count) layouts"
        return "\(total) · Double-click or press Return to open."
    }

    // MARK: - Pieces

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            TextField("Search layouts", text: $query)
                .textFieldStyle(.plain)
                .onKeyPress(.downArrow) { move(1) }
                .onKeyPress(.upArrow) { move(-1) }
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    @ViewBuilder
    private var content: some View {
        if store.views.isEmpty {
            ContentUnavailableView(
                "No Saved Layouts", systemImage: "rectangle.3.group",
                description: Text("Choose Save layout in the layout menu to keep your charts and timeframe.")
            )
            .frame(maxHeight: .infinity)
        } else if visible.isEmpty {
            ContentUnavailableView.search(text: query)
                .frame(maxHeight: .infinity)
        } else {
            list
        }
    }

    private var list: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 6) {
                    ForEach(visible) { view in
                        SavedLayoutRow(
                            view: view,
                            isCurrent: view.id == layout.activeViewID,
                            isSelected: view.id == selection,
                            onSelect: { selection = view.id },
                            onOpen: { open(view) },
                            onDelete: { pendingDelete = view }
                        )
                        .id(view.id)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 4)
            }
            .onChange(of: selection) { _, id in
                guard let id else { return }
                withAnimation(.easeOut(duration: 0.12)) { proxy.scrollTo(id) }
            }
        }
        .focusable()
        .focusEffectDisabled()
        .onKeyPress(.downArrow) { move(1) }
        .onKeyPress(.upArrow) { move(-1) }
        .onKeyPress(.delete) {
            guard let view = selectedView else { return .ignored }
            pendingDelete = view
            return .handled
        }
    }

    private var footer: some View {
        HStack {
            Spacer()
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Button("Open") { if let view = selectedView { open(view) } }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(selectedView == nil)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    // MARK: - Actions

    private func move(_ delta: Int) -> KeyPress.Result {
        guard !visible.isEmpty else { return .ignored }
        let current = visible.firstIndex { $0.id == selection } ?? (delta > 0 ? -1 : visible.count)
        selection = visible[min(max(current + delta, 0), visible.count - 1)].id
        return .handled
    }

    private func open(_ view: SavedView) {
        dismiss()
        layout.requestOpen(view)
    }
}

/// One saved layout as a card: name, what is in it, when it was last opened, and its first markets.
private struct SavedLayoutRow: View {
    let view: SavedView
    let isCurrent: Bool
    let isSelected: Bool
    let onSelect: () -> Void
    let onOpen: () -> Void
    let onDelete: () -> Void

    @State private var isHovered = false

    private var previews: [TickerConfig] { EmptyStateSuggestions.previewConfigs(of: view) }

    private var detail: String {
        let count = view.tickerConfigs.count
        var parts = [count == 1 ? "1 chart" : "\(count) charts", view.timeRange.rawValue]
        if let opened = view.lastOpenedAt {
            parts.append("Opened \(opened.formatted(.relative(presentation: .named)))")
        }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Text(view.name)
                        .font(.body.weight(.semibold))
                        .lineLimit(1)
                        .truncationMode(.tail)
                    if isCurrent { badge("Current") }
                }
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            artwork
            Button(action: onDelete) {
                Image(systemName: "trash")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(isHovered ? Color.red : Color.secondary)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .opacity(isHovered || isSelected ? 1 : 0)
            .help("Delete…")
            .accessibilityLabel("Delete \(view.name)")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(fill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(isSelected ? Color.accentColor.opacity(0.6) : Color.primary.opacity(0.1))
        )
        .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .onHover { isHovered = $0 }
        .onTapGesture(count: 2, perform: onOpen)
        .onTapGesture(perform: onSelect)
        .contextMenu {
            Button("Open", action: onOpen)
            Divider()
            Button("Delete…", role: .destructive, action: onDelete)
        }
        .animation(.easeOut(duration: 0.12), value: isHovered)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(isCurrent ? "\(view.name), current layout, \(detail)" : "\(view.name), \(detail)")
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(named: "Open", onOpen)
    }

    /// The first few markets, overlapping; a plain glyph for layouts with no market charts.
    @ViewBuilder
    private var artwork: some View {
        if previews.isEmpty {
            Image(systemName: "rectangle.3.group")
                .foregroundStyle(.tertiary)
                .frame(height: 24)
        } else {
            HStack(spacing: -7) {
                ForEach(previews, id: \.chartID) { config in
                    MarketIcon(
                        ticker: config.symbol, source: config.source, displayName: config.displayName, size: 24
                    )
                    .overlay(Circle().strokeBorder(Color(nsColor: .windowBackgroundColor), lineWidth: 1.5))
                }
            }
            .accessibilityHidden(true)
        }
    }

    private var fill: Color {
        if isSelected { return Color.accentColor.opacity(0.1) }
        return isHovered ? Color.primary.opacity(0.06) : Color.primary.opacity(0.03)
    }

    private func badge(_ text: String) -> some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(Color.accentColor)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(Color.accentColor.opacity(0.14), in: Capsule())
    }
}

#Preview("Layouts") {
    let store = SavedViewStore(database: try! .makeInMemory())
    let samples: [(String, [(String, DataSourceType)])] = [
        ("Majors", [("BTCUSDT", .binance), ("ETHUSDT", .binance), ("SOLUSDT", .binance)]),
        ("Day Trading", [("BTC-USD", .coinbase)]),
        ("Macro", []),
    ]
    for (name, markets) in samples {
        let configs = markets.map { TickerConfig(symbol: $0.0, source: $0.1) }
        try? store.upsert(
            SavedView(
                name: name, tickers: configs.map(\.symbol), timeRange: .oneDay, createdAt: Date(),
                tickerConfigs: configs, candleCount: TimeRange.oneDay.dataPointLimit))
    }
    let layout = SavedLayoutController(store: store, activeViewID: store.views.first?.id)
    return SavedLayoutPickerSheet(layout: layout, store: store)
}
