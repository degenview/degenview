import Foundation

typealias PaperOrderID = UUID

enum PaperCurrency: String, Codable, CaseIterable, Identifiable {
    case USD, EUR, GBP, JPY, USDT, USDC
    var id: String { rawValue }
}

enum PaperAssetClass: String, Codable, CaseIterable, Identifiable {
    case stock, crypto, forex, futures, prediction
    var id: String { rawValue }
}

enum PaperOrderSide: String, Codable, CaseIterable { case buy, sell }
enum PaperOrderType: String, Codable, CaseIterable, Identifiable {
    case market, limit, stop, stopLimit
    var id: String { rawValue }
}
enum PaperTimeInForce: String, Codable, CaseIterable, Identifiable {
    case day = "DAY"
    case goodTilCanceled = "GTC"
    var id: String { rawValue }
}
enum PaperOrderStatus: String, Codable {
    case pendingSubmission, working, partiallyFilled, filled, pendingCancel, canceled, rejected, expired

    var isTerminal: Bool { [.filled, .canceled, .rejected, .expired].contains(self) }
    var isWorking: Bool { self == .working || self == .partiallyFilled }
}
enum PaperOrderRole: String, Codable { case entry, takeProfit, stopLoss }
enum PaperPositionSide: String, Codable { case long, short }
enum PaperFillPriceSource: String, Codable { case bidAsk, lastPriceFallback, barFallback }

struct PaperInstrument: Codable, Hashable, Identifiable {
    var id: String { key }
    var key: String
    var symbol: String
    var displayName: String
    var source: DataSourceType
    var assetClass: PaperAssetClass
    var quoteCurrency: PaperCurrency
    var tickSize: Decimal
    var minimumQuantity: Decimal
    var quantityIncrement: Decimal
    var contractMultiplier: Decimal
    var pointValue: Decimal
    var expiration: Date?

    static func chart(symbol: String, displayName: String, source: DataSourceType) -> Self {
        let assetClass: PaperAssetClass = source == .alpaca ? .stock : (source == .polymarket ? .prediction : .crypto)
        return .init(
            key: "\(source.rawValue):\(symbol)", symbol: symbol, displayName: displayName,
            source: source, assetClass: assetClass, quoteCurrency: .USD,
            tickSize: source == .polymarket ? 0.001 : 0.00000001,
            minimumQuantity: source == .alpaca ? 1 : 0.00000001,
            quantityIncrement: source == .alpaca ? 1 : 0.00000001,
            contractMultiplier: 1, pointValue: 1
        )
    }
}

enum PaperCommissionConfiguration: Codable, Equatable {
    case none
    case fixedPerOrder(Decimal)
    case percentage(Decimal)
    case perContract(Decimal)

    var label: String {
        switch self {
        case .none: return "None"
        case .fixedPerOrder(let value): return "\(value) per order"
        case .percentage(let value): return "\(value)% of value"
        case .perContract(let value): return "\(value) per contract"
        }
    }
}

struct PaperLeverageConfiguration: Codable, Equatable {
    var stocks: Decimal = 1
    var crypto: Decimal = 1
    var forex: Decimal = 1
    var futures: Decimal = 1
    var prediction: Decimal = 1

    func leverage(for assetClass: PaperAssetClass) -> Decimal {
        switch assetClass {
        case .stock: stocks
        case .crypto: crypto
        case .forex: forex
        case .futures: futures
        case .prediction: prediction
        }
    }
}

struct PaperAccountSettings: Codable, Equatable {
    var commission: PaperCommissionConfiguration = .none
    var leverage = PaperLeverageConfiguration()
    var slippageTicks: Decimal = 0
    var showPositionsOnChart = true
    var showOrdersOnChart = true
    var showExecutionsOnChart = true
    var instantOrderPlacement = false
}

struct PaperAccount: Codable, Identifiable, Equatable {
    let id: UUID
    var name: String
    var baseCurrency: PaperCurrency
    var initialBalance: Decimal
    var cashBalance: Decimal
    var realizedPnL: Decimal
    var settings: PaperAccountSettings
    var createdAt: Date

    init(
        id: UUID = UUID(), name: String = "Paper Trading", baseCurrency: PaperCurrency = .USD,
        initialBalance: Decimal = 100_000, settings: PaperAccountSettings = .init(), createdAt: Date = Date()
    ) {
        self.id = id
        self.name = name
        self.baseCurrency = baseCurrency
        self.initialBalance = initialBalance
        self.cashBalance = initialBalance
        self.realizedPnL = 0
        self.settings = settings
        self.createdAt = createdAt
    }
}

struct PaperOrderRequest: Codable, Equatable {
    var accountID: UUID
    var instrument: PaperInstrument
    var side: PaperOrderSide
    var type: PaperOrderType
    var quantity: Decimal
    var limitPrice: Decimal?
    var stopPrice: Decimal?
    var timeInForce: PaperTimeInForce = .goodTilCanceled
    var takeProfit: Decimal?
    var stopLoss: Decimal?
}

struct PaperOrderChanges: Codable, Equatable {
    var quantity: Decimal?
    var limitPrice: Decimal?
    var stopPrice: Decimal?
}

struct PaperOrder: Codable, Identifiable, Equatable {
    var id: PaperOrderID
    var accountID: UUID
    var instrument: PaperInstrument
    var side: PaperOrderSide
    var type: PaperOrderType
    var originalQuantity: Decimal
    var filledQuantity: Decimal
    var averageFillPrice: Decimal?
    var limitPrice: Decimal?
    var stopPrice: Decimal?
    var timeInForce: PaperTimeInForce
    var status: PaperOrderStatus
    var role: PaperOrderRole
    var parentOrderID: UUID?
    var ocoGroupID: UUID?
    var stopTriggered: Bool
    var reservedMargin: Decimal
    var rejectionReason: String?
    var createdAt: Date
    var updatedAt: Date

    var remainingQuantity: Decimal { max(0, originalQuantity - filledQuantity) }
}

struct PaperFill: Codable, Identifiable, Equatable {
    let id: UUID
    let orderID: PaperOrderID
    let accountID: UUID
    let instrument: PaperInstrument
    let side: PaperOrderSide
    let quantity: Decimal
    let price: Decimal
    let commission: Decimal
    let priceSource: PaperFillPriceSource
    let timestamp: Date
}

struct PaperPosition: Codable, Identifiable, Equatable {
    var id: String { instrument.key }
    var accountID: UUID
    var instrument: PaperInstrument
    var signedQuantity: Decimal
    var averageEntryPrice: Decimal
    var realizedGrossPnL: Decimal
    var commissions: Decimal
    var openedAt: Date
    var updatedAt: Date

    var side: PaperPositionSide { signedQuantity >= 0 ? .long : .short }
    var quantity: Decimal { abs(signedQuantity) }
}

struct PaperQuote: Codable, Equatable {
    var instrumentKey: String
    var bid: Decimal?
    var ask: Decimal?
    var last: Decimal?
    var timestamp: Date
    var isMarketOpen: Bool = true

    func executablePrice(for side: PaperOrderSide) -> (Decimal, PaperFillPriceSource)? {
        switch side {
        case .buy:
            if let ask { return (ask, .bidAsk) }
        case .sell:
            if let bid { return (bid, .bidAsk) }
        }
        return last.map { ($0, .lastPriceFallback) }
    }
}

enum PaperOrderEventKind: String, Codable {
    case placed, accepted, partiallyFilled, filled, modified, canceled, rejected, expired
}
struct PaperOrderEvent: Codable, Identifiable, Equatable {
    let id: UUID
    let orderID: UUID
    let accountID: UUID
    let kind: PaperOrderEventKind
    let timestamp: Date
    let message: String
    let order: PaperOrder
}

struct PaperClosedTrade: Codable, Identifiable, Equatable {
    let id: UUID
    let accountID: UUID
    let instrument: PaperInstrument
    let side: PaperPositionSide
    let entryTimestamp: Date
    let exitTimestamp: Date
    let entryPrice: Decimal
    let exitPrice: Decimal
    let quantity: Decimal
    let grossPnL: Decimal
    let commission: Decimal
    var netPnL: Decimal { grossPnL - commission }
}

struct PaperJournalEntry: Codable, Identifiable, Equatable {
    let id: UUID
    let accountID: UUID
    let timestamp: Date
    let message: String
}

struct PaperAccountMetrics: Codable, Equatable {
    var balance: Decimal
    var equity: Decimal
    var realizedPnL: Decimal
    var unrealizedPnL: Decimal
    var positionMargin: Decimal
    var ordersMargin: Decimal
    var availableFunds: Decimal
    var marginBuffer: Decimal
}

struct PaperTradingSnapshot: Codable, Equatable {
    var accounts: [PaperAccount] = []
    var selectedAccountID: UUID?
    var orders: [PaperOrder] = []
    var fills: [PaperFill] = []
    var positions: [PaperPosition] = []
    var orderEvents: [PaperOrderEvent] = []
    var closedTrades: [PaperClosedTrade] = []
    var journal: [PaperJournalEntry] = []
    var quotes: [String: PaperQuote] = [:]

    static let empty = PaperTradingSnapshot()
}

enum PaperTradingError: LocalizedError, Equatable {
    case accountNotFound, orderNotFound, invalidTransition
    case invalidQuantity(String)
    case invalidPrice(String)
    case staleMarketData, marketClosed, noMarketData
    case unsupportedCurrencyConversion(PaperCurrency, PaperCurrency)
    case insufficientFunds(required: Decimal, available: Decimal)
    case unsupportedSymbol

    var errorDescription: String? {
        switch self {
        case .accountNotFound: "Paper account not found."
        case .orderNotFound: "Paper order not found."
        case .invalidTransition: "The order can no longer be changed."
        case .invalidQuantity(let value): value
        case .invalidPrice(let value): value
        case .staleMarketData: "Order cannot be simulated reliably because market data is stale."
        case .marketClosed: "The market is closed; the paper order remains pending."
        case .noMarketData: "No executable market price is available."
        case .unsupportedCurrencyConversion(let from, let to):
            "FX conversion from \(from.rawValue) to \(to.rawValue) is unavailable."
        case .insufficientFunds(let required, let available):
            "Order rejected: insufficient available funds. Required margin: \(required); available funds: \(available)."
        case .unsupportedSymbol: "This symbol is not supported by Paper Trading."
        }
    }
}

extension Decimal {
    var doubleValue: Double { NSDecimalNumber(decimal: self).doubleValue }
    static func rounded(_ value: Decimal, scale: Int = 8) -> Decimal {
        var source = value
        var result = Decimal()
        NSDecimalRound(&result, &source, scale, .bankers)
        return result
    }
}
