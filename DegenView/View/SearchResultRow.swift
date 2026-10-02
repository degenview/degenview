import SwiftUI

/// Shared search result row used by AddTickerSheet and ChartSettingsSheet.
struct SearchResultRow: View {
    let result: TickerSearchResult
    let isSelected: Bool
    let onSelect: () -> Void
    var onCommit: (() -> Void)? = nil
    /// Marks which source the row is from, for lists that mix sources (Recent).
    var showsSource = false
    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 10) {
            SearchResultIcon(result: result)

            VStack(alignment: .leading, spacing: 1) {
                Text(result.symbol)
                    .font(.body.weight(.semibold))
                    .lineLimit(1)

                if let chain = result.chain, let dex = result.dex {
                    Text("\(chain.capitalized) · \(dex.capitalized)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if showsSource {
                SourceLogoView(source: result.source, size: 14)
                    .help(result.source.displayName)
            }

            if let price = result.price {
                Text(price, format: .currency(code: "USD").precision(.fractionLength(2...6)))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }

            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(Color.accentColor)
                .opacity(isSelected ? 1 : 0)
                .accessibilityLabel("Selected")
                .accessibilityHidden(!isSelected)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .modifier(ResultRowTapModifier(onSelect: onSelect, onCommit: onCommit))
        .background(rowFill, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(isSelected ? Color.accentColor.opacity(0.5) : .clear)
        )
        .onHover { isHovered = $0 }
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var rowFill: Color {
        if isSelected { return Color.accentColor.opacity(0.12) }
        return isHovered ? Color.primary.opacity(0.06) : .clear
    }
}

/// The coin, company or market artwork for a result; a monogram until (or unless) one resolves.
///
/// Artwork the search payload carried is used as-is. Otherwise the shared icon cache is asked,
/// except for DEX pairs, whose lookup is one request each and which fall back to the monogram.
struct SearchResultIcon: View {
    let result: TickerSearchResult
    var size: CGFloat = 28
    @State private var url: URL?

    var body: some View {
        TickerIconView(symbol: base, url: url ?? result.imageURL, size: size)
            .task(id: "\(result.source.rawValue):\(result.fullSymbol)") {
                guard result.imageURL == nil else { return }
                switch result.source {
                case .binance, .coinbase, .coingecko, .alpaca:
                    url = await IconResolver.shared.iconURL(
                        ticker: result.fullSymbol, source: result.source, baseSymbol: base)
                case .dexscreener, .polymarket, .kalshi, .coinMarketCap:
                    break
                }
            }
    }

    /// The ticker without a trading pair or company name: "BTC/USDT" → "BTC".
    private var base: String {
        let label = result.symbol.components(separatedBy: " — ").first ?? result.symbol
        return (label.split(separator: "/").first.map(String.init) ?? label).uppercased()
    }
}

/// Delays the single-click action just long enough to distinguish it from a double-click.
/// Without an exclusive gesture, a double-click can select and commit independently.
private struct ResultRowTapModifier: ViewModifier {
    let onSelect: () -> Void
    let onCommit: (() -> Void)?

    func body(content: Content) -> some View {
        if let onCommit {
            content.gesture(
                TapGesture(count: 2)
                    .exclusively(before: TapGesture(count: 1))
                    .onEnded { value in
                        switch value {
                        case .first:
                            onCommit()
                        case .second:
                            onSelect()
                        }
                    }
            )
        } else {
            content.onTapGesture(perform: onSelect)
        }
    }
}
