import SwiftUI

/// One market. Observes only its own quote cell, so a tick redraws this row and nothing else.
struct WatchlistInstrumentRow<MenuContent: View>: View {
    let item: WatchlistInstrument
    @ObservedObject var cell: WatchlistQuoteCell
    let flag: WatchlistFlag?
    let display: WatchlistDisplaySettings
    let isIndented: Bool
    @ViewBuilder let menu: () -> MenuContent

    @State private var iconURL: URL?
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 6) {
            TickerIconView(symbol: item.iconBaseSymbol, url: iconURL, size: 20)
                .task(id: item.instrument.key) {
                    iconURL = await IconResolver.shared.iconURL(
                        ticker: item.instrument.symbol, source: item.instrument.source,
                        baseSymbol: item.iconBaseSymbol)
                }

            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    if let flag {
                        Circle().fill(flag.color).frame(width: 7, height: 7)
                            .accessibilityLabel("\(flag.title) flag")
                    }
                    Text(item.name)
                        .font(.callout.weight(.medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                if display.showsDescription {
                    Text(item.label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .help("\(item.name)\n\(item.instrument.qualifiedSymbol)")

            Spacer(minLength: 4)

            ForEach(display.columns) { column in
                valueText(for: column)
                    .frame(width: column.width, alignment: .trailing)
            }
        }
        .padding(.leading, isIndented ? 10 : 0)
        .padding(.vertical, 2)
        .opacity(isDimmed ? 0.6 : 1)
        .contentShape(Rectangle())
        .overlay(alignment: .trailing) {
            if isHovering {
                Menu {
                    menu()
                } label: {
                    Image(systemName: "ellipsis.circle.fill")
                        .foregroundStyle(.secondary)
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .background(.bar, in: Circle())
                .accessibilityLabel("Actions for \(item.name)")
            }
        }
        .onHover { isHovering = $0 }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            WatchlistQuoteFormat.spoken(name: item.name, quote: cell.quote, source: item.instrument.source))
        .accessibilityAddTraits(.isButton)
    }

    /// A price that is no longer current stays visible but quieter.
    private var isDimmed: Bool {
        guard let quote = cell.quote, quote.last != nil else { return false }
        return !quote.freshness.isCurrent
    }

    private var directionColor: Color {
        switch WatchlistQuoteFormat.direction(cell.quote) {
        case .up: return .green
        case .down: return .red
        case .flat: return .secondary
        }
    }

    @ViewBuilder
    private func valueText(for column: WatchlistColumn) -> some View {
        let source = item.instrument.source
        switch column {
        case .last:
            Text(WatchlistQuoteFormat.last(cell.quote, source: source)).monospacedDigit()
        case .change:
            Text(WatchlistQuoteFormat.change(cell.quote, source: source))
                .monospacedDigit().foregroundStyle(directionColor)
        case .changePercent:
            let glyph = WatchlistQuoteFormat.direction(cell.quote).glyph
            Text((glyph.isEmpty ? "" : glyph + " ") + WatchlistQuoteFormat.percent(cell.quote))
                .monospacedDigit().foregroundStyle(directionColor)
                .help(cell.quote?.changeBasis?.label ?? "")
        case .volume:
            Text(WatchlistQuoteFormat.volume(cell.quote)).monospacedDigit().foregroundStyle(.secondary)
        case .exchange:
            Text(source.displayName).foregroundStyle(.secondary)
        case .status:
            Text(cell.quote?.freshness.label ?? WatchlistQuote.Freshness.unavailable.label)
                .foregroundStyle(.secondary)
        }
    }
}
