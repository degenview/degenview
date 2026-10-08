import Foundation

/// Text for the watchlist columns. Missing values are a dash, never zero.
enum WatchlistQuoteFormat {
    static let missing = "–"

    static func last(_ quote: WatchlistQuote?, source: DataSourceType) -> String {
        guard let last = quote?.last else { return missing }
        return PriceFormatter.format(last, scale: source.priceScale)
    }

    static func change(_ quote: WatchlistQuote?, source: DataSourceType) -> String {
        guard let change = quote?.change else { return missing }
        return PriceFormatter.changeAmount(change, scale: source.priceScale)
    }

    static func percent(_ quote: WatchlistQuote?) -> String {
        guard let percent = quote?.changePercent else { return missing }
        return String(format: "%+.2f%%", percent)
    }

    static func volume(_ quote: WatchlistQuote?) -> String {
        guard let quote, let volume = quote.volume else { return missing }
        let text = compactNumber(volume)
        return quote.volumeKind == .quoteCurrency ? "$" + text : text
    }

    /// 1,234 → "1.23K", 5,600,000 → "5.6M".
    static func compactNumber(_ value: Double) -> String {
        let units: [(Double, String)] = [(1_000_000_000_000, "T"), (1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")]
        for (divisor, suffix) in units where abs(value) >= divisor {
            return (value / divisor).formatted(.number.precision(.fractionLength(0...2))) + suffix
        }
        return value.formatted(.number.precision(.fractionLength(0...2)))
    }

    /// Direction as a glyph, so it never rests on red and green alone.
    static func direction(_ quote: WatchlistQuote?) -> Direction {
        guard let value = quote?.changePercent ?? quote?.change, value != 0 else { return .flat }
        return value > 0 ? .up : .down
    }

    enum Direction {
        case up
        case down
        case flat

        var glyph: String {
            switch self {
            case .up: return "▲"
            case .down: return "▼"
            case .flat: return ""
            }
        }

        var spoken: String {
            switch self {
            case .up: return "up"
            case .down: return "down"
            case .flat: return "unchanged"
            }
        }
    }

    /// One sentence a screen reader can use for a row.
    static func spoken(name: String, quote: WatchlistQuote?, source: DataSourceType) -> String {
        guard let quote, quote.last != nil else { return "\(name), no quote" }
        var parts = [name, last(quote, source: source)]
        if quote.changePercent != nil {
            parts.append("\(direction(quote).spoken) \(percent(quote).replacingOccurrences(of: "%", with: " percent"))")
        }
        if !quote.freshness.isCurrent { parts.append(quote.freshness.label) }
        return parts.joined(separator: ", ")
    }
}
