import Foundation

// MARK: - Identity

/// A Kalshi market as the app stores it: `"SERIES/MARKET"`.
///
/// The candlestick endpoint is addressed by both the series and the market ticker, and
/// nothing on a market itself names its series, so the pair travels together as the
/// chart's opaque symbol — the same slot a Polymarket CLOB token id fills.
struct KalshiMarketID: Equatable, Hashable {
    let series: String
    let market: String

    var raw: String { "\(series)/\(market)" }

    init(series: String, market: String) {
        self.series = series
        self.market = market
    }

    init?(_ raw: String) {
        let parts = raw.split(separator: "/", maxSplits: 1).map(String.init)
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else { return nil }
        self.init(series: parts[0], market: parts[1])
    }
}

// MARK: - Decoding

enum KalshiJSON {
    /// Kalshi fields are snake_case, and prices are fixed-point *strings* ("0.5600").
    static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }

    /// A dollar string as a probability, 0…1.
    static func probability(_ raw: String?) -> Double? {
        guard let raw, let value = Double(raw), value.isFinite, (0...1).contains(value) else {
            return nil
        }
        return value
    }
}

// MARK: - Series list

/// `GET /series` — every series Kalshi lists. Only the fields search needs are kept,
/// which is also what gets cached on disk: the raw payload is ~18 MB.
struct KalshiSeriesList: Decodable {
    let series: [KalshiSeries]?
}

struct KalshiSeries: Codable, Equatable {
    let ticker: String
    let title: String
    let category: String?
    let tags: [String]?

    /// Lenient on `title`: one malformed row must not sink the whole 14k-row list.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        ticker = try container.decode(String.self, forKey: .ticker)
        title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        category = try container.decodeIfPresent(String.self, forKey: .category)
        tags = try container.decodeIfPresent([String].self, forKey: .tags)
    }

    init(ticker: String, title: String, category: String? = nil, tags: [String]? = nil) {
        self.ticker = ticker
        self.title = title
        self.category = category
        self.tags = tags
    }
}

// MARK: - Events and markets

/// `GET /events?series_ticker=…&with_nested_markets=true`
struct KalshiEventsResponse: Decodable {
    let events: [KalshiEvent]?
}

/// `GET /markets/{ticker}`
struct KalshiMarketResponse: Decodable {
    let market: KalshiMarket?
}

/// A group of related markets, e.g. "Fed decision in Oct 2026?".
struct KalshiEvent: Decodable {
    let eventTicker: String?
    let seriesTicker: String?
    let title: String?
    let subTitle: String?
    let markets: [KalshiMarket]?
}

/// One tradable YES/NO contract.
struct KalshiMarket: Decodable {
    let ticker: String
    let title: String?
    let yesSubTitle: String?
    let status: String?
    let yesAskDollars: String?
    let yesBidDollars: String?
    let lastPriceDollars: String?

    /// Price currently offered to a YES buyer — what Kalshi's market page shows. A
    /// book with no offers reports an ask of `0.0000`, so that falls back to the last
    /// trade instead of charting 0%.
    var displayedYesPrice: Double? {
        if let ask = KalshiJSON.probability(yesAskDollars), ask > 0 { return ask }
        return KalshiJSON.probability(lastPriceDollars)
    }

    /// Short label for this market within its event, falling back to its full title.
    var shortTitle: String? {
        if let label = yesSubTitle, !label.isEmpty { return label }
        return title
    }

    var isTradable: Bool {
        status == "active"
    }
}

// MARK: - Candlesticks

/// `GET /series/{series}/markets/{ticker}/candlesticks`
struct KalshiCandlesResponse: Decodable {
    let candlesticks: [KalshiCandle]?
}

struct KalshiCandle: Decodable {
    /// Unix timestamp, seconds, of the end of the period.
    let endPeriodTs: Double
    /// Trade prices. Absent or null for a period with no trades.
    let price: KalshiCandlePrice?
}

struct KalshiCandlePrice: Decodable {
    let closeDollars: String?
    /// Close of the last period that had a trade.
    let previousDollars: String?
}
