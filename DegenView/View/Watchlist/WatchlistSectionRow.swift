import SwiftUI

/// A section heading: a small-caps title, a hairline to the right, then the count and a fold chevron.
///
/// Sections are flat. The title starts on the same left edge as every symbol and the chevron ends on the
/// same right edge as every value, so nothing under a section is indented. Clicking anywhere on the heading
/// folds or unfolds it.
struct WatchlistSectionRow: View {
    let section: WatchlistSection
    let count: Int
    /// The heading opens the list, so it needs no space above it.
    var isFirst = false
    /// A symbol or section is being dragged over this heading.
    var isDropTarget = false
    let onToggle: () -> Void

    @State private var isHovering = false
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        HStack(spacing: 8) {
            Text(section.title.uppercased())
                .font(.system(size: 10.5, weight: .semibold))
                .tracking(0.6)
                .lineLimit(1)
                .layoutPriority(1)
                .foregroundStyle(titleColor)

            Rectangle()
                .fill(ruleColor)
                .frame(height: isDropTarget ? 2 : 1 / displayScale)
                .frame(minWidth: 12, maxWidth: .infinity)

            Text("\(count)")
                .font(.caption2)
                .monospacedDigit()
                .foregroundStyle(.tertiary)

            Image(systemName: "chevron.down")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(isHovering ? Color.primary : Color.secondary)
                .rotationEffect(.degrees(section.isCollapsed ? -90 : 0))
                .animation(.easeInOut(duration: 0.15), value: section.isCollapsed)
                .frame(width: 14, alignment: .trailing)
        }
        .padding(.top, isFirst ? 6 : 16)
        .padding(.bottom, 4)
        .contentShape(Rectangle())
        .onHover { isHovering = $0 }
        .onTapGesture(perform: onToggle)
        .help(section.isCollapsed ? "Expand section" : "Collapse section")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(section.title), \(count) symbols")
        .accessibilityValue(section.isCollapsed ? "collapsed" : "expanded")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction(named: section.isCollapsed ? "Expand" : "Collapse", onToggle)
    }

    private var titleColor: Color {
        if isDropTarget { return .accentColor }
        return isHovering ? .primary : .secondary
    }

    private var ruleColor: Color {
        isDropTarget ? .accentColor : Color(nsColor: .separatorColor)
    }
}

#Preview("Section headings") {
    VStack(spacing: 0) {
        WatchlistSectionRow(section: WatchlistSection(title: "Majors"), count: 3, isFirst: true) {}
        WatchlistSectionRow(section: WatchlistSection(title: "Watching"), count: 12) {}
        WatchlistSectionRow(section: WatchlistSection(title: "Folded away", isCollapsed: true), count: 5) {}
        WatchlistSectionRow(section: WatchlistSection(title: "Dragging over"), count: 2, isDropTarget: true) {}
        WatchlistSectionRow(
            section: WatchlistSection(title: "A section title long enough that it has to be truncated"), count: 40
        ) {}
    }
    .padding(.horizontal, WatchlistMetrics.edgeInset)
    .frame(width: 270)
    .background(.bar)
}

#Preview("Aligned with rows") {
    let book = WatchlistQuoteBook()
    let display = WatchlistDisplaySettings()
    @MainActor func row(_ symbol: String, price: Double, change: Double) -> some View {
        let id = InstrumentID(source: .binance, symbol: symbol)
        book.apply([
            id: WatchlistQuote(
                last: price, change: price * change / 100, changePercent: change, changeBasis: .rolling24h,
                freshness: .live)
        ])
        return WatchlistInstrumentRow(
            item: WatchlistInstrument(instrument: id, name: symbol, label: symbol),
            cell: book.cell(for: id), flag: nil, display: display, menu: { EmptyView() })
    }
    return VStack(spacing: 0) {
        row("BTCUSDT", price: 82_514, change: -1.4)
        row("ETHUSDT", price: 2_537, change: 1.6)
        WatchlistSectionRow(section: WatchlistSection(title: "Majors"), count: 2) {}
        row("SOLUSDT", price: 113.2, change: -3.4)
        row("DOGEUSDT", price: 0.087, change: 1.9)
        WatchlistSectionRow(section: WatchlistSection(title: "Watching"), count: 0) {}
        WatchlistEmptySectionRow()
    }
    .padding(.horizontal, WatchlistMetrics.edgeInset)
    .frame(width: 270)
    .background(.bar)
}
