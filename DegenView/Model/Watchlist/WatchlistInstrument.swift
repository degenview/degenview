import Foundation

/// One market in a watchlist. Carries only what is needed to find the market again and
/// to label it before a quote arrives; prices are transient and never stored here.
struct WatchlistInstrument: Identifiable, Codable, Equatable, Sendable {
    /// Identity of this row, so drag, selection and removal never depend on array offsets.
    let id: UUID
    var instrument: InstrumentID
    /// Title shown first: "Bitcoin", "BTC/USDT", a prediction-market question.
    var name: String
    /// Short secondary label: "BTC", the symbol.
    var label: String
    /// Set for prediction markets, whose symbol is an opaque token id.
    var displayName: String?
    var pmSeries: [PmSeriesConfig]?

    init(
        id: UUID = UUID(), instrument: InstrumentID, name: String, label: String,
        displayName: String? = nil, pmSeries: [PmSeriesConfig]? = nil
    ) {
        self.id = id
        self.instrument = instrument
        self.name = name
        self.label = label
        self.displayName = displayName
        self.pmSeries = pmSeries
    }

    init(searchResult result: TickerSearchResult) {
        let labels = Self.labels(for: result)
        self.init(
            instrument: InstrumentID(searchResult: result),
            name: labels.name,
            label: labels.ticker,
            displayName: result.source.isPredictionMarket
                ? (result.eventTitle ?? result.question ?? result.symbol) : nil,
            pmSeries: result.pmSeries)
    }

    /// `InstrumentID` ignores the DEX chain in its own equality, but a row that gained one
    /// has changed and must be saved.
    static func == (lhs: WatchlistInstrument, rhs: WatchlistInstrument) -> Bool {
        lhs.id == rhs.id && lhs.instrument == rhs.instrument && lhs.instrument.chain == rhs.instrument.chain
            && lhs.name == rhs.name && lhs.label == rhs.label && lhs.displayName == rhs.displayName
            && lhs.pmSeries == rhs.pmSeries
    }

    /// What a chart needs to open this market.
    var tickerConfig: TickerConfig {
        TickerConfig(
            symbol: instrument.symbol, source: instrument.source, displayName: displayName,
            pmSeries: pmSeries)
    }

    /// A copy under a new identity, for duplicated lists.
    func copy() -> WatchlistInstrument {
        WatchlistInstrument(
            instrument: instrument, name: name, label: label, displayName: displayName,
            pmSeries: pmSeries)
    }

    private static func labels(for result: TickerSearchResult) -> (name: String, ticker: String) {
        if result.source.isPredictionMarket {
            return (result.eventTitle ?? result.question ?? result.symbol, result.symbol)
        }

        if result.source == .alpaca {
            let parts = result.symbol.components(separatedBy: " — ")
            return (
                parts.count > 1 ? parts.dropFirst().joined(separator: " — ") : result.symbol,
                result.fullSymbol.uppercased()
            )
        }

        if result.source == .coingecko {
            let name = result.fullSymbol
                .split(separator: "-")
                .map { $0.capitalized }
                .joined(separator: " ")
            return (name, result.symbol.uppercased())
        }

        return (result.symbol, result.symbol.uppercased())
    }
}
