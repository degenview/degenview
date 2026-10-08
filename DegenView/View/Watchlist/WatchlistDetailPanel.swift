import SwiftUI

/// Facts about the highlighted market, from what the quote provider reports. Nothing here is
/// invented: a field the provider does not supply is a dash.
struct WatchlistDetailPanel: View {
    let item: WatchlistInstrument
    @ObservedObject var cell: WatchlistQuoteCell
    let flag: WatchlistFlag?
    let listNames: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                SourceLogoView(source: item.instrument.source, size: 18)
                VStack(alignment: .leading, spacing: 0) {
                    Text(item.name).font(.callout.weight(.semibold)).lineLimit(1)
                    Text(item.instrument.source.displayName).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if let flag {
                    HStack(spacing: 5) {
                        WatchlistFlagMark(flag: flag)
                        Text(flag.title).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }

            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 3) {
                row("Symbol", item.instrument.qualifiedSymbol, selectable: true)
                row("Last", WatchlistQuoteFormat.last(cell.quote, source: item.instrument.source))
                row(
                    "Change",
                    "\(WatchlistQuoteFormat.change(cell.quote, source: item.instrument.source)) (\(WatchlistQuoteFormat.percent(cell.quote, source: item.instrument.source)))"
                )
                if let basis = cell.quote?.changeBasis {
                    row("Measured", basis.label)
                }
                row("Volume", WatchlistQuoteFormat.volume(cell.quote))
                row("Status", cell.quote?.freshness.label ?? WatchlistQuote.Freshness.unavailable.label)
                if let note = cell.quote?.note { row("Note", note) }
                if let timestamp = cell.quote?.timestamp {
                    row("Updated", timestamp.formatted(date: .omitted, time: .standard))
                }
                row("In lists", listNames.isEmpty ? WatchlistQuoteFormat.missing : listNames.joined(separator: ", "))
            }
            .font(.caption)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.regularMaterial)
    }

    private func row(_ title: String, _ value: String, selectable: Bool = false) -> some View {
        GridRow(alignment: .firstTextBaseline) {
            Text(title).foregroundStyle(.secondary)
            if selectable {
                Text(value).textSelection(.enabled).lineLimit(2)
            } else {
                Text(value).lineLimit(2)
            }
        }
    }
}
