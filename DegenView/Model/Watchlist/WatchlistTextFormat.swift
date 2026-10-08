import Foundation

/// Plain-text watchlist exchange.
///
/// One entry per line, `SOURCE:SYMBOL`, with `###Title` lines for sections:
///
///     ###Majors
///     BINANCE:BTCUSDT
///     COINBASE:ETH-USD
///     DEXSCREENER:solana/<pair address>
///     POLYMARKET:<token id>|Will it happen?
///
/// A single line of comma-separated entries, as TradingView exports, is read too.
/// TradingView compatibility is best effort: exchange prefixes DegenView has no provider
/// for are reported as skipped, never guessed at.
enum WatchlistTextFormat {
    enum Row: Equatable {
        case section(String)
        case instrument(WatchlistInstrument)
    }

    struct Parsed: Equatable {
        var rows: [Row] = []
        var skipped: [WatchlistImportReport.Skipped] = []
    }

    /// US listings on these venues map to Alpaca, which serves US equities.
    private static let usExchanges: Set<String> = ["NASDAQ", "NYSE", "AMEX", "ARCA", "NYSEARCA", "BATS", "NYSEAMERICAN"]

    // MARK: Export

    static func export(_ list: Watchlist) -> String {
        list.entries.map { entry in
            switch entry {
            case .section(let section): return "###\(section.title)"
            case .instrument(let item): return line(for: item)
            }
        }
        .joined(separator: "\n")
    }

    private static func line(for item: WatchlistInstrument) -> String {
        let id = item.instrument
        switch id.source {
        case .dexscreener:
            let symbol = id.chain.map { "\($0)/\(id.symbol)" } ?? id.symbol
            return "\(id.source.watchlistSlug):\(symbol)"
        case .polymarket, .kalshi:
            let title = (item.displayName ?? item.name).replacingOccurrences(of: "\n", with: " ")
            return "\(id.qualifiedSymbol)|\(title)"
        default:
            return id.qualifiedSymbol
        }
    }

    // MARK: Import

    static func parse(_ text: String) -> Parsed {
        let lines = text.components(separatedBy: .newlines)
        let tokens: [String]
        if lines.filter({ !$0.trimmingCharacters(in: .whitespaces).isEmpty }).count == 1, text.contains(",") {
            tokens = text.components(separatedBy: ",")
        } else {
            tokens = lines
        }

        var parsed = Parsed()
        for (offset, raw) in tokens.enumerated() {
            let token = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !token.isEmpty else { continue }
            if token.hasPrefix("###") {
                let title = token.dropFirst(3).trimmingCharacters(in: .whitespaces)
                parsed.rows.append(.section(title.isEmpty ? "Section" : title))
                continue
            }
            switch instrument(from: token) {
            case .success(let item): parsed.rows.append(.instrument(item))
            case .failure(let failure):
                parsed.skipped.append(.init(line: offset + 1, text: token, reason: failure.reason))
            }
        }
        return parsed
    }

    private struct Failure: Error { let reason: String }

    private static func instrument(from token: String) -> Result<WatchlistInstrument, Failure> {
        guard let colon = token.firstIndex(of: ":") else {
            return .failure(Failure(reason: "Missing exchange prefix, expected SOURCE:SYMBOL"))
        }
        let prefix = token[..<colon].trimmingCharacters(in: .whitespaces).uppercased()
        var rest = String(token[token.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
        var title: String?
        if let bar = rest.firstIndex(of: "|") {
            let candidate = String(rest[rest.index(after: bar)...]).trimmingCharacters(in: .whitespaces)
            title = candidate.isEmpty ? nil : candidate
            rest = String(rest[..<bar]).trimmingCharacters(in: .whitespaces)
        }
        guard !rest.isEmpty else { return .failure(Failure(reason: "Missing symbol")) }

        if usExchanges.contains(prefix) {
            return stock(rest)
        }
        guard let source = DataSourceType(watchlistSlug: prefix) else {
            return .failure(Failure(reason: "Unsupported exchange \(prefix)"))
        }

        switch source {
        case .binance:
            let symbol = rest.uppercased()
            guard symbol.range(of: "^[A-Z0-9]{3,20}$", options: .regularExpression) != nil else {
                return .failure(Failure(reason: "Not a Binance spot symbol (perpetuals and futures aren't supported)"))
            }
            return .success(item(source, symbol, name: symbol, label: symbol))
        case .coinbase:
            let symbol = rest.uppercased()
            guard symbol.range(of: "^[A-Z0-9]+-[A-Z0-9]+$", options: .regularExpression) != nil else {
                return .failure(Failure(reason: "Coinbase products are written BASE-QUOTE, like BTC-USD"))
            }
            return .success(item(source, symbol, name: symbol.replacingOccurrences(of: "-", with: "/"), label: symbol))
        case .alpaca:
            return stock(rest)
        case .coingecko:
            let id = rest.lowercased()
            let name = id.split(separator: "-").map { $0.capitalized }.joined(separator: " ")
            return .success(item(source, id, name: name, label: id.uppercased()))
        case .dexscreener:
            let pieces = rest.split(separator: "/", maxSplits: 1).map(String.init)
            if pieces.count == 2, pieces[0].range(of: "^[a-z0-9_-]+$", options: .regularExpression) != nil {
                return .success(item(source, pieces[1], chain: pieces[0], name: pieces[1], label: pieces[1]))
            }
            return .success(item(source, rest, name: rest, label: rest))
        case .polymarket, .kalshi:
            guard let title else {
                return .failure(Failure(reason: "Prediction markets need a title after |; add them from search instead"))
            }
            let id = InstrumentID(source: source, symbol: rest)
            return .success(
                WatchlistInstrument(instrument: id, name: title, label: rest, displayName: title))
        case .coinMarketCap:
            return .failure(Failure(reason: "CoinMarketCap index charts aren't markets"))
        }
    }

    private static func stock(_ symbol: String) -> Result<WatchlistInstrument, Failure> {
        let upper = symbol.uppercased()
        guard upper.range(of: "^[A-Z]{1,5}([.-][A-Z])?$", options: .regularExpression) != nil else {
            return .failure(Failure(reason: "Not a US stock ticker"))
        }
        return .success(item(.alpaca, upper, name: upper, label: upper))
    }

    private static func item(
        _ source: DataSourceType, _ symbol: String, chain: String? = nil, name: String, label: String
    ) -> WatchlistInstrument {
        WatchlistInstrument(
            instrument: InstrumentID(source: source, symbol: symbol, chain: chain), name: name, label: label)
    }
}
