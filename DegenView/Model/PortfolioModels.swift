import Foundation

enum PortfolioCurrency: String, Codable, CaseIterable, Identifiable, Sendable {
    case USD, EUR, GBP, JPY, CHF, BTC
    var id: String { rawValue }

    /// The currency a pair's quote asset is priced in. Dollar stablecoins count as USD; a quote
    /// this app can't convert (`TRY`, `ETH`, …) falls back to USD.
    init(quoteSymbol: String?) {
        let quote = quoteSymbol?.uppercased()
        if quote == "USDT" || quote == "USDC" {
            self = .USD
        } else {
            self = quote.flatMap(PortfolioCurrency.init(rawValue:)) ?? .USD
        }
    }

    func format(_ value: Decimal) -> String {
        switch self {
        case .JPY: value.formatted(.currency(code: rawValue).precision(.fractionLength(0)))
        case .BTC: "BTC " + value.formatted(.number.precision(.fractionLength(0...8)))
        default: value.formatted(.currency(code: rawValue).precision(.fractionLength(2)))
        }
    }

    /// The currency's own sign, for badges: `$`, `€`, `£`, `¥`, `Fr`, `₿`.
    var glyph: String {
        switch self {
        case .USD: "$"
        case .EUR: "€"
        case .GBP: "£"
        case .JPY: "¥"
        case .CHF: "Fr"
        case .BTC: "₿"
        }
    }

    /// "US Dollar" in the user's language; Bitcoin isn't an ISO currency, so it is named here.
    var displayName: String {
        if self == .BTC { return "Bitcoin" }
        guard let name = Locale.current.localizedString(forCurrencyCode: rawValue), let first = name.first else {
            return rawValue
        }
        return first.uppercased() + name.dropFirst()
    }

    /// A per-unit price. Same as `format`, except under one unit it keeps up to eight decimals,
    /// so a micro-cap token reads `$0.00002` rather than `$0.00`.
    func formatPrice(_ value: Decimal) -> String {
        guard self != .BTC, value != 0, abs(value) < 1 else { return format(value) }
        return value.formatted(.currency(code: rawValue).precision(.fractionLength(2...8)))
    }

    /// Short form for chart axes: `135.84K`. The currency is stated beside the chart, and a
    /// suffix glued after a trailing currency sign (`135,84 US$K`) reads wrongly.
    func formatCompact(_ value: Decimal) -> String {
        guard self != .BTC else { return format(value) }
        let units: [(Decimal, String)] = [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")]
        let digits = Decimal.FormatStyle.number.precision(.fractionLength(0...2))
        for (divisor, suffix) in units where abs(value) >= divisor {
            return (value / divisor).formatted(digits) + suffix
        }
        return value.formatted(digits)
    }
}

/// Rules for the name typed into the create-portfolio sheet.
enum PortfolioNameCheck {
    static let maxLength = 40

    /// Trimmed, and cut to `maxLength`.
    static func normalized(_ name: String) -> String {
        String(name.trimmingCharacters(in: .whitespacesAndNewlines).prefix(maxLength))
    }

    static func isValid(_ name: String) -> Bool { !normalized(name).isEmpty }

    /// Whether another portfolio already has this name (ignoring case and surrounding spaces).
    static func isDuplicate(_ name: String, among existing: [String]) -> Bool {
        let wanted = normalized(name)
        guard !wanted.isEmpty else { return false }
        return existing.contains { normalized($0).caseInsensitiveCompare(wanted) == .orderedSame }
    }
}

enum PortfolioSource: String, Codable, CaseIterable, Sendable {
    case manual = "Manual"
    case wallet = "Wallet"
    case exchange = "Exchange"
    case csv = "CSV"
    case coinMarketCap = "CoinMarketCap"
}

enum PortfolioTransactionType: String, Codable, CaseIterable, Identifiable, Sendable {
    case buy = "Buy"
    case sell = "Sell"
    case transferIn = "Transfer In"
    case transferOut = "Transfer Out"
    case reward = "Reward"
    case stakingReward = "Staking Reward"
    case airdrop = "Airdrop"
    case mining = "Mining"
    case interest = "Interest"
    case fee = "Fee"
    case adjustment = "Adjustment"

    var id: String { rawValue }
    var addsQuantity: Bool {
        [.buy, .transferIn, .reward, .stakingReward, .airdrop, .mining, .interest].contains(self)
    }
    var removesQuantity: Bool { [.sell, .transferOut, .fee].contains(self) }
}

/// A source-qualified identifier. Symbols are display data and are never identity.
struct PortfolioAsset: Codable, Hashable, Identifiable, Sendable {
    var id: String { key }
    let key: String
    var symbol: String
    var name: String
    var source: DataSourceType
    var quoteCurrency: PortfolioCurrency
    var metadata: [String: String]

    init(
        key: String, symbol: String, name: String, source: DataSourceType,
        quoteCurrency: PortfolioCurrency = .USD, metadata: [String: String] = [:]
    ) {
        self.key = key
        self.symbol = symbol
        self.name = name
        self.source = source
        self.quoteCurrency = quoteCurrency
        self.metadata = metadata
    }

    init(searchResult: TickerSearchResult) {
        let quote = searchResult.symbol.split(separator: "/").last.map(String.init)
        let currency = PortfolioCurrency(quoteSymbol: quote)
        self.init(
            key: "\(searchResult.source.rawValue):\(searchResult.fullSymbol)",
            symbol: searchResult.symbol, name: searchResult.symbol,
            source: searchResult.source, quoteCurrency: currency,
            metadata: searchResult.metadata)
    }

    /// Separates an equity ticker from its company name in a search label ("AAPL — Apple Inc.").
    private static let nameSeparator = " — "

    /// The ticker alone, without trading pair or company name: `BTC/USDT` → `BTC`.
    var displayTicker: String {
        let label = symbol.components(separatedBy: Self.nameSeparator).first ?? symbol
        let base = label.split(separator: "/", maxSplits: 1).first.map(String.init) ?? label
        return base.trimmingCharacters(in: .whitespaces).uppercased()
    }

    /// A name the symbol itself carries (Alpaca labels equities "AAPL — Apple Inc."), if any.
    var embeddedName: String? {
        guard let range = symbol.range(of: Self.nameSeparator) else { return nil }
        let name = symbol[range.upperBound...].trimmingCharacters(in: .whitespaces)
        return name.isEmpty ? nil : name
    }
}

extension Decimal {
    /// An asset quantity: grouped, at most eight decimals (a satoshi), no trailing zeros.
    var portfolioQuantity: String {
        formatted(.number.grouping(.automatic).precision(.fractionLength(0...8)))
    }
}

struct Portfolio: Codable, Identifiable, Equatable, Sendable {
    let id: UUID
    var name: String
    var baseCurrency: PortfolioCurrency
    var createdAt: Date
    var updatedAt: Date
    var isArchived: Bool
    var sort: PortfolioHoldingsSort

    init(
        id: UUID = UUID(), name: String, baseCurrency: PortfolioCurrency = .USD,
        createdAt: Date = Date(), updatedAt: Date = Date(), isArchived: Bool = false,
        sort: PortfolioHoldingsSort = .currentValue
    ) {
        self.id = id
        self.name = name
        self.baseCurrency = baseCurrency
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.isArchived = isArchived
        self.sort = sort
    }
}

enum PortfolioHoldingsSort: String, Codable, CaseIterable, Identifiable, Sendable {
    case currentValue = "Value"
    case allocation = "Allocation"
    case dayChange = "24h %"
    case profitLoss = "P&L"
    case profitLossPercent = "P&L %"
    case asset = "Asset"
    var id: String { rawValue }
}

struct PortfolioTransaction: Codable, Identifiable, Equatable, Sendable {
    let id: UUID
    var portfolioID: UUID
    var asset: PortfolioAsset
    var type: PortfolioTransactionType
    var quantity: Decimal
    var price: Decimal?
    var priceCurrency: PortfolioCurrency
    var fee: Decimal
    var feeCurrency: PortfolioCurrency
    var timestamp: Date
    var notes: String
    var source: PortfolioSource
    var externalTransactionID: String?
    var createdAt: Date
    var updatedAt: Date

    init(
        id: UUID = UUID(), portfolioID: UUID, asset: PortfolioAsset,
        type: PortfolioTransactionType, quantity: Decimal, price: Decimal? = nil,
        priceCurrency: PortfolioCurrency = .USD, fee: Decimal = 0,
        feeCurrency: PortfolioCurrency = .USD, timestamp: Date = Date(), notes: String = "",
        source: PortfolioSource = .manual, externalTransactionID: String? = nil,
        createdAt: Date = Date(), updatedAt: Date = Date()
    ) {
        self.id = id
        self.portfolioID = portfolioID
        self.asset = asset
        self.type = type
        self.quantity = quantity
        self.price = price
        self.priceCurrency = priceCurrency
        self.fee = fee
        self.feeCurrency = feeCurrency
        self.timestamp = timestamp
        self.notes = notes
        self.source = source
        self.externalTransactionID = externalTransactionID
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }
}

struct PortfolioHolding: Identifiable, Equatable, Sendable {
    var id: String { asset.key }
    var asset: PortfolioAsset
    var quantity: Decimal
    var costBasis: Decimal
    var averageCost: Decimal
    var realizedPnL: Decimal
    var currentPrice: Decimal?
    var previousDayPrice: Decimal?
    var currentValue: Decimal?
    var unrealizedPnL: Decimal?
    var allocation: Decimal
    var isPriceStale: Bool

    var totalPnL: Decimal? { unrealizedPnL.map { $0 + realizedPnL } }
    var pnlPercent: Decimal? { costBasis == 0 ? nil : unrealizedPnL.map { $0 / costBasis } }
    var dayChangePercent: Decimal? {
        guard let currentPrice, let previousDayPrice, previousDayPrice != 0 else { return nil }
        return (currentPrice - previousDayPrice) / previousDayPrice
    }
}

struct PortfolioSnapshot: Codable, Identifiable, Equatable, Sendable {
    var id: String { "\(portfolioID.uuidString):\(timestamp.timeIntervalSince1970)" }
    let portfolioID: UUID
    let timestamp: Date
    let value: Decimal
    let netContributions: Decimal
    let realizedPnL: Decimal
    let unrealizedPnL: Decimal
    let isComplete: Bool
}

struct PortfolioLedgerSnapshot: Codable, Equatable, Sendable {
    var portfolios: [Portfolio] = []
    var transactions: [PortfolioTransaction] = []
    var historicalSnapshots: [PortfolioSnapshot] = []
    var selectedPortfolioID: UUID?
    var invalidatedAfter: [UUID: Date] = [:]
    static let empty = PortfolioLedgerSnapshot()
}

struct PortfolioQuote: Codable, Equatable, Sendable {
    var price: Decimal
    var previousDayPrice: Decimal?
    var timestamp: Date
}

enum PortfolioSelection: Hashable {
    case all
    case portfolio(UUID)
}

enum PortfolioError: LocalizedError, Equatable {
    case portfolioNotFound, transactionNotFound, invalidQuantity, missingPrice
    case insufficientHoldings(available: Decimal)
    case unsupportedCurrency(PortfolioCurrency, PortfolioCurrency)
    case duplicateExternalTransaction
    case invalidCSV(String)

    var errorDescription: String? {
        switch self {
        case .portfolioNotFound: "Portfolio not found."
        case .transactionNotFound: "Transaction not found."
        case .invalidQuantity: "Quantity must be greater than zero."
        case .missingPrice: "A price is required for this transaction type."
        case .insufficientHoldings(let available): "Insufficient holdings at that date (available: \(available))."
        case .unsupportedCurrency(let from, let to):
            "Historical conversion from \(from.rawValue) to \(to.rawValue) is unavailable."
        case .duplicateExternalTransaction: "This imported transaction already exists."
        case .invalidCSV(let message): message
        }
    }
}

enum PortfolioPrivacy {
    static func sensitive(_ visibleValue: String, enabled: Bool, mask: String = "••••••••") -> String {
        enabled ? mask : visibleValue
    }
    static func accessibility(_ visibleValue: String, enabled: Bool, hiddenDescription: String) -> String {
        enabled ? hiddenDescription : visibleValue
    }
}
