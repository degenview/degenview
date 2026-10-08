import Foundation

enum DataSourceType: String, CaseIterable, Codable {
    case binance = "Binance"
    case coinbase = "Coinbase"
    case coingecko = "CoinGecko"
    case dexscreener = "DEXScreener"
    case alpaca = "Alpaca (IEX)"
    case polymarket = "Polymarket"
    case kalshi = "Kalshi"
    case coinMarketCap = "CoinMarketCap"

    /// Crypto price sources — the set the multi-source ticker search fans out to, in
    /// the order its results are listed. Prediction markets are excluded: they get
    /// their own search pane.
    static var cryptoSources: [DataSourceType] {
        [.binance, .coinbase, .coingecko, .dexscreener]
    }

    var displayName: String { rawValue }

    /// Prediction-market providers — YES probabilities in 0…1, charted as lines.
    static let predictionMarkets: [DataSourceType] = [.polymarket, .kalshi]

    var isPredictionMarket: Bool {
        self == .polymarket || self == .kalshi
    }

    /// What this source's prices mean. Prediction markets quote probabilities in
    /// 0…1; everything else quotes USD.
    var priceScale: PriceScale {
        isPredictionMarket ? .probability : .currency
    }

    /// Whether this source reports per-candle turnover for the volume bars to draw.
    ///
    /// Binance sends quote volume with every kline, Coinbase's candles carry base volume
    /// that is priced into turnover, and DEX pairs get theirs from GeckoTerminal. CoinGecko's OHLC endpoint has no volume column — only
    /// `/market_chart`, which reports a rolling 24h figure rather than per-candle —
    /// and the prediction-market line charts report none at all.
    var providesVolume: Bool {
        switch self {
        case .binance, .coinbase, .dexscreener, .alpaca: return true
        case .coingecko, .polymarket, .kalshi, .coinMarketCap: return false
        }
    }

    /// Whether a larger `limit` buys *older* candles at the same interval.
    ///
    /// True everywhere except prediction markets, whose count only sets downsample fidelity
    /// inside a fixed window — asking for more shrinks the step rather than extending
    /// history, so indicator warm-up is fetched only where it actually works.
    ///
    /// CoinGecko qualifies because its window is a fixed span per interval, deep
    /// enough for the furthest zoom-out: a larger `limit` slices further back into a
    /// buffer already fetched, at the same candle size, without another request.
    var fetchesByCount: Bool {
        switch self {
        case .binance, .coinbase, .dexscreener, .coingecko, .alpaca: return true
        case .polymarket, .kalshi, .coinMarketCap: return false
        }
    }

    var icon: String {
        switch self {
        case .binance: return "building.columns.fill"
        case .coinbase: return "bitcoinsign.circle.fill"
        case .coingecko: return "chart.line.uptrend.xyaxis"
        case .dexscreener: return "arrow.triangle.swap"
        case .alpaca: return "chart.xyaxis.line"
        case .polymarket, .kalshi: return "chart.line.flattrend.xyaxis"
        case .coinMarketCap: return "gauge.with.dots.needle.50percent"
        }
    }

    /// Name of this provider's logo imageset in the app's asset catalog. A plain string
    /// so the alert agent, which bundles no assets, can still compile this file.
    var logoAsset: String {
        switch self {
        case .binance: return "SourceLogo-binance"
        case .coinbase: return "SourceLogo-coinbase"
        case .coingecko: return "SourceLogo-coingecko"
        case .dexscreener: return "SourceLogo-dexscreener"
        case .alpaca: return "SourceLogo-alpaca"
        case .polymarket: return "SourceLogo-polymarket"
        case .kalshi: return "SourceLogo-kalshi"
        case .coinMarketCap: return "SourceLogo-coinmarketcap"
        }
    }
}

enum ChartAssetType: String, CaseIterable, Identifiable {
    case crypto = "Crypto"
    case stock = "Stock"
    case predictionMarket = "Prediction Markets"

    var id: String { rawValue }
}

/// One tradable choice within a prediction-market event — a market id + human label
/// pair. The id is a CLOB token id on Polymarket and `"SERIES/MARKET"` on Kalshi; the
/// name is persisted JSON, so it stays.
struct PmSeriesConfig: Codable, Equatable, Hashable, Identifiable {
    let tokenID: String
    let label: String
    var enabled: Bool

    var id: String { tokenID }
}

enum PortfolioChartKind: String, Codable, CaseIterable, Identifiable {
    case valueChart = "Portfolio Value Chart"
    case value = "Portfolio Value"
    case allocation = "Allocation"

    var id: String { rawValue }
    var title: String {
        switch self {
        case .valueChart: return "Portfolio Value"
        case .value, .allocation: return rawValue
        }
    }
}

enum PortfolioChartRange: String, Codable, CaseIterable, Identifiable {
    case oneDay = "1D"
    case oneWeek = "1W"
    case oneMonth = "1M"
    case oneYear = "1Y"
    case all = "ALL"

    var id: String { rawValue }
    var duration: TimeInterval? {
        switch self {
        case .oneDay: return 86_400
        case .oneWeek: return 7 * 86_400
        case .oneMonth: return 31 * 86_400
        case .oneYear: return 365 * 86_400
        case .all: return nil
        }
    }
}

struct PortfolioChartConfig: Codable, Equatable, Hashable {
    var portfolioID: UUID?
    var kind: PortfolioChartKind
    var range: PortfolioChartRange = .oneMonth
}

enum CoinMarketCapChartType: String, Codable, CaseIterable, Identifiable {
    case altcoinSeasonHistorical = "coinmarketcap.altcoinSeasonHistorical"
    case altcoinSeasonLatest = "coinmarketcap.altcoinSeasonLatest"
    case fearAndGreedHistorical = "coinmarketcap.fearAndGreedHistorical"
    case fearAndGreedLatest = "coinmarketcap.fearAndGreedLatest"

    var id: String { rawValue }
    var title: String {
        switch self {
        case .altcoinSeasonHistorical: "Altcoin Season Index Historical"
        case .altcoinSeasonLatest: "Altcoin Season Index Latest"
        case .fearAndGreedHistorical: "CMC Crypto Fear and Greed Historical"
        case .fearAndGreedLatest: "CMC Crypto Fear and Greed Latest"
        }
    }
    var isHistorical: Bool { self == .altcoinSeasonHistorical || self == .fearAndGreedHistorical }
}

enum CMCAltcoinRange: String, Codable, CaseIterable, Identifiable {
    case sevenDays = "7d"
    case thirtyDays = "30d"
    case ninetyDays = "90d"
    var id: String { rawValue }
    var label: String { rawValue.uppercased() }
}

enum CMCFearGreedRange: String, Codable, CaseIterable, Identifiable {
    case sevenDays = "7D"
    case oneMonth = "1M"
    case threeMonths = "3M"
    case oneYear = "1Y"
    case all = "ALL"
    var id: String { rawValue }
    var dayCount: Int? {
        switch self {
        case .sevenDays: 7
        case .oneMonth: 31
        case .threeMonths: 90
        case .oneYear: 365
        case .all: nil
        }
    }
}

struct CoinMarketCapChartConfig: Codable, Equatable, Hashable {
    var type: CoinMarketCapChartType
    var altcoinRange: CMCAltcoinRange = .sevenDays
    var fearGreedRange: CMCFearGreedRange = .oneMonth
    var showAreaFill = true
    var showThresholdZones = true
    var showSupportingStatistics = true
}

/// Persisted config for a single ticker — symbol + which API to fetch from.
struct TickerConfig: Codable, Equatable, Hashable {
    /// Stable identity for script attachment.
    var chartID: UUID = UUID()
    let symbol: String
    let source: DataSourceType

    // Chart appearance settings (nil = use defaults)
    var bullishColorHex: String?
    var bearishColorHex: String?
    var yAxisDecimalPlaces: Int?  // nil = auto-detect

    /// Vertical price-scale zoom set by dragging the Y axis. nil = auto-fit (1.0).
    var yZoom: Double?

    /// Volume bars under the candles. nil = off.
    var showVolume: Bool?

    /// RSI line across the bottom of the plot. nil = off.
    var showRSI: Bool?

    /// EMA overlay on the price scale, and its period. nil = off / default period.
    var showEMA: Bool?
    var emaPeriod: Int?

    /// Bollinger bands on the price scale. nil = off.
    var showBollinger: Bool?

    /// Confirmed bullish/bearish Supertrend change markers. nil = off.
    var showTrendFlips: Bool?

    /// Raw values of the applied built-in indicators that are switched off without being removed.
    /// nil = none hidden.
    var hiddenIndicators: [String]?

    /// Human-readable label shown on the card. Only set for sources whose `symbol`
    /// is an opaque identifier — a Polymarket CLOB token id is 77 digits, so the
    /// market question has to ride along. Nil for crypto (the symbol reads fine).
    var displayName: String?

    /// All tradable choices for multi-outcome Polymarket events. Nil for single-choice
    /// markets and all non-Polymarket sources.
    var pmSeries: [PmSeriesConfig]?

    /// Present when this slot is a portfolio card rather than a market chart.
    var portfolioChart: PortfolioChartConfig? = nil
    var coinMarketCapChart: CoinMarketCapChartConfig? = nil
    var bitcoinPowerLaw: BitcoinPowerLawConfig? = nil

    /// One isolated script instance for this market chart. Draft and last-valid source
    /// are stored separately so a compiler error never blanks an already working plot.
    var pine: PineConfiguration? = nil

    /// Canonical local-script instances.
    var scripts: [ChartScriptInstance] = []

    init(
        symbol: String, source: DataSourceType, bullishColorHex: String? = nil,
        bearishColorHex: String? = nil, yAxisDecimalPlaces: Int? = nil, yZoom: Double? = nil,
        showVolume: Bool? = nil, showRSI: Bool? = nil, showEMA: Bool? = nil,
        emaPeriod: Int? = nil, showBollinger: Bool? = nil, showTrendFlips: Bool? = nil,
        hiddenIndicators: [String]? = nil, displayName: String? = nil,
        pmSeries: [PmSeriesConfig]? = nil, portfolioChart: PortfolioChartConfig? = nil,
        coinMarketCapChart: CoinMarketCapChartConfig? = nil,
        bitcoinPowerLaw: BitcoinPowerLawConfig? = nil,
        pine: PineConfiguration? = nil, chartID: UUID = UUID(),
        scripts: [ChartScriptInstance] = []
    ) {
        self.chartID = chartID
        self.symbol = symbol
        self.source = source
        self.bullishColorHex = bullishColorHex
        self.bearishColorHex = bearishColorHex
        self.yAxisDecimalPlaces = yAxisDecimalPlaces
        self.yZoom = yZoom
        self.showVolume = showVolume
        self.showRSI = showRSI
        self.showEMA = showEMA
        self.emaPeriod = emaPeriod
        self.showBollinger = showBollinger
        self.showTrendFlips = showTrendFlips
        self.hiddenIndicators = hiddenIndicators
        self.displayName = displayName
        self.pmSeries = pmSeries
        self.portfolioChart = portfolioChart
        self.coinMarketCapChart = coinMarketCapChart
        self.bitcoinPowerLaw = bitcoinPowerLaw
        self.pine = pine
        self.scripts = scripts
    }
}
