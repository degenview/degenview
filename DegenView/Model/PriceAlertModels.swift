import Foundation

enum AlertFrequency: String, Codable, CaseIterable, Identifiable, Sendable {
    case once = "Once"
    case everyTime = "Every Time"
    var id: String { rawValue }
}

enum AlertState: String, Codable, CaseIterable, Sendable {
    case active, paused, triggered, unsupported
}

enum ThresholdSide: String, Codable, Sendable { case below, atOrAbove }

enum AlertCondition: Equatable, Sendable {
    case crossesAbove(target: Decimal)
    case crossesBelow(target: Decimal)
    case risesBy(percent: Decimal, reference: Decimal, target: Decimal)
    case fallsBy(percent: Decimal, reference: Decimal, target: Decimal)
    case unsupported(tag: String)

    var target: Decimal? {
        switch self {
        case .crossesAbove(let value), .crossesBelow(let value): value
        case .risesBy(_, _, let value), .fallsBy(_, _, let value): value
        case .unsupported: nil
        }
    }

    var crossesAbove: Bool {
        switch self {
        case .crossesAbove, .risesBy: true
        default: false
        }
    }
}

extension AlertCondition: Codable {
    private enum Keys: String, CodingKey { case tag, target, percent, reference }
    init(from decoder: Decoder) throws {
        let box = try decoder.container(keyedBy: Keys.self)
        let tag = try box.decode(String.self, forKey: .tag)
        switch tag {
        case "crossesAbove": self = .crossesAbove(target: try box.decode(Decimal.self, forKey: .target))
        case "crossesBelow": self = .crossesBelow(target: try box.decode(Decimal.self, forKey: .target))
        case "risesBy":
            self = .risesBy(
                percent: try box.decode(Decimal.self, forKey: .percent),
                reference: try box.decode(Decimal.self, forKey: .reference),
                target: try box.decode(Decimal.self, forKey: .target))
        case "fallsBy":
            self = .fallsBy(
                percent: try box.decode(Decimal.self, forKey: .percent),
                reference: try box.decode(Decimal.self, forKey: .reference),
                target: try box.decode(Decimal.self, forKey: .target))
        default: self = .unsupported(tag: tag)
        }
    }
    func encode(to encoder: Encoder) throws {
        var box = encoder.container(keyedBy: Keys.self)
        switch self {
        case .crossesAbove(let target):
            try box.encode("crossesAbove", forKey: .tag)
            try box.encode(target, forKey: .target)
        case .crossesBelow(let target):
            try box.encode("crossesBelow", forKey: .tag)
            try box.encode(target, forKey: .target)
        case .risesBy(let percent, let reference, let target):
            try box.encode("risesBy", forKey: .tag)
            try box.encode(percent, forKey: .percent)
            try box.encode(reference, forKey: .reference)
            try box.encode(target, forKey: .target)
        case .fallsBy(let percent, let reference, let target):
            try box.encode("fallsBy", forKey: .tag)
            try box.encode(percent, forKey: .percent)
            try box.encode(reference, forKey: .reference)
            try box.encode(target, forKey: .target)
        case .unsupported(let tag): try box.encode(tag, forKey: .tag)
        }
    }
}

struct PriceAlert: Codable, Identifiable, Equatable, Sendable {
    let id: UUID
    var asset: PortfolioAsset
    var condition: AlertCondition
    var currency: PortfolioCurrency
    var frequency: AlertFrequency
    var state: AlertState
    var note: String
    var armed: Bool
    var previousValue: Decimal?
    var lastFingerprint: String?
    var createdAt: Date
    var updatedAt: Date
    /// Webhook endpoints (`WebhookEndpoint.id`) notified when this alert triggers. Empty by default,
    /// which leaves delivery exactly as it was before webhooks existed.
    var webhookEndpointIDs: [UUID] = []
    /// The webhook body, with `{{placeholders}}`. Nil sends `AlertMessageRenderer.defaultPriceAlertTemplate`.
    var webhookMessage: String?

    init(
        id: UUID = UUID(), asset: PortfolioAsset, condition: AlertCondition,
        currency: PortfolioCurrency = .USD, frequency: AlertFrequency = .once,
        state: AlertState = .active, note: String = "", armed: Bool = true,
        previousValue: Decimal? = nil, lastFingerprint: String? = nil,
        createdAt: Date = Date(), updatedAt: Date = Date(),
        webhookEndpointIDs: [UUID] = [], webhookMessage: String? = nil
    ) {
        self.id = id
        self.asset = asset
        self.condition = condition
        self.currency = currency
        self.frequency = frequency
        self.state = state
        self.note = note
        self.armed = armed
        self.previousValue = previousValue
        self.lastFingerprint = lastFingerprint
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.webhookEndpointIDs = webhookEndpointIDs
        self.webhookMessage = webhookMessage
    }

    private enum CodingKeys: String, CodingKey {
        case id, asset, condition, currency, frequency, state, note, armed, previousValue, lastFingerprint
        case createdAt, updatedAt, webhookEndpointIDs, webhookMessage
    }

    /// Alerts saved before webhooks existed have neither webhook key.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        asset = try c.decode(PortfolioAsset.self, forKey: .asset)
        condition = try c.decode(AlertCondition.self, forKey: .condition)
        currency = try c.decode(PortfolioCurrency.self, forKey: .currency)
        frequency = try c.decode(AlertFrequency.self, forKey: .frequency)
        state = try c.decode(AlertState.self, forKey: .state)
        note = try c.decode(String.self, forKey: .note)
        armed = try c.decode(Bool.self, forKey: .armed)
        previousValue = try c.decodeIfPresent(Decimal.self, forKey: .previousValue)
        lastFingerprint = try c.decodeIfPresent(String.self, forKey: .lastFingerprint)
        createdAt = try c.decode(Date.self, forKey: .createdAt)
        updatedAt = try c.decode(Date.self, forKey: .updatedAt)
        webhookEndpointIDs = try c.decodeIfPresent([UUID].self, forKey: .webhookEndpointIDs) ?? []
        webhookMessage = try c.decodeIfPresent(String.self, forKey: .webhookMessage)
    }
}

struct AlertTriggerEvent: Codable, Identifiable, Equatable, Sendable {
    let id: UUID
    let alertID: UUID
    let asset: PortfolioAsset
    let observedValue: Decimal
    let target: Decimal
    let currency: PortfolioCurrency
    let timestamp: Date
    let quoteFingerprint: String
    var origin: AlertEventOrigin = .live
    var delivery: AlertDeliveryRecord = .pending
    /// The candle the quote came from, for webhook placeholders. Nil for catch-up events.
    var candle: AlertCandleSnapshot?

    init(
        id: UUID = UUID(), alertID: UUID, asset: PortfolioAsset, observedValue: Decimal,
        target: Decimal, currency: PortfolioCurrency, timestamp: Date,
        quoteFingerprint: String, origin: AlertEventOrigin = .live,
        delivery: AlertDeliveryRecord = .pending, candle: AlertCandleSnapshot? = nil
    ) {
        self.id = id
        self.alertID = alertID
        self.asset = asset
        self.observedValue = observedValue
        self.target = target
        self.currency = currency
        self.timestamp = timestamp
        self.quoteFingerprint = quoteFingerprint
        self.origin = origin
        self.delivery = delivery
        self.candle = candle
    }

    private enum CodingKeys: String, CodingKey {
        case id, alertID, asset, observedValue, target, currency, timestamp, quoteFingerprint, origin, delivery, candle
    }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        alertID = try c.decode(UUID.self, forKey: .alertID)
        asset = try c.decode(PortfolioAsset.self, forKey: .asset)
        observedValue = try c.decode(Decimal.self, forKey: .observedValue)
        target = try c.decode(Decimal.self, forKey: .target)
        currency = try c.decode(PortfolioCurrency.self, forKey: .currency)
        timestamp = try c.decode(Date.self, forKey: .timestamp)
        quoteFingerprint = try c.decode(String.self, forKey: .quoteFingerprint)
        origin = try c.decodeIfPresent(AlertEventOrigin.self, forKey: .origin) ?? .live
        delivery = try c.decodeIfPresent(AlertDeliveryRecord.self, forKey: .delivery) ?? .pending
        candle = try c.decodeIfPresent(AlertCandleSnapshot.self, forKey: .candle)
    }
}

/// The candle a price quote was read from. Webhook placeholders (`{{open}}`, `{{volume}}`, ...)
/// come from here, in the asset's own quote currency (not converted to the alert's currency).
struct AlertCandleSnapshot: Codable, Equatable, Sendable {
    var openTime: Date
    var open: Double
    var high: Double
    var low: Double
    var close: Double
    var volume: Double
    /// The candle size the quote was polled at, in DegenView's names (`1m`, `1h`). Not the user's chart timeframe.
    var interval: String
}

enum AlertEventOrigin: String, Codable, Sendable { case live, catchUp }
enum AlertDeliveryState: String, Codable, Sendable { case pending, delivered, suppressed, failed }
struct AlertDeliveryRecord: Codable, Equatable, Sendable {
    var state: AlertDeliveryState
    var attemptCount: Int
    var lastAttemptAt: Date?
    var nextRetryAt: Date?
    var error: String?
    static let pending = AlertDeliveryRecord(state: .pending, attemptCount: 0)
}

struct AlertNotificationSettings: Codable, Equatable, Sendable {
    var deliveryEnabled = true
    var soundEnabled = true
    var inAppBannersEnabled = true
    var macOSNotificationsEnabled = true
}

struct AlertPersistenceSnapshot: Codable, Equatable, Sendable {
    var revision: UInt64 = 0
    var alerts: [PriceAlert] = []
    var history: [AlertTriggerEvent] = []
    var settings = AlertNotificationSettings()
    var processedCommandIDs: [UUID] = []
    var health = AlertRuntimeHealth()
}

enum AlertRuntimeOwner: String, Codable, Sendable { case none, app, agent }
enum AlertServiceState: String, Codable, Sendable {
    case unavailable, disabled, enabled, requiresApproval, notFound, failed
}
enum AlertNotificationPermission: String, Codable, Sendable {
    case unknown, notDetermined, denied, authorized, provisional, ephemeral
}
struct AlertProviderHealth: Codable, Equatable, Sendable, Identifiable {
    var id: String { source.rawValue }
    var source: DataSourceType
    var lastSuccessfulQuote: Date?
    var lastError: String?
    var retryAt: Date?
    var hasDataGap = false
}
struct AlertRuntimeHealth: Codable, Equatable, Sendable {
    var owner: AlertRuntimeOwner = .none
    var serviceState: AlertServiceState = .unavailable
    var heartbeat: Date?
    var notificationPermission: AlertNotificationPermission = .unknown
    var providers: [AlertProviderHealth] = []
    var unreconciledGaps: [String] = []
}

enum AlertRuntimeCommandPayload: Codable, Equatable, Sendable {
    case save(PriceAlert, baseline: Decimal?)
    case setState(UUID, AlertState, baseline: Decimal?)
    case delete(UUID)
    case clearHistory
    case updateSettings(AlertNotificationSettings)
    case requestNotificationAuthorization
}
struct AlertRuntimeCommand: Codable, Identifiable, Equatable, Sendable {
    let id: UUID
    let createdAt: Date
    let expectedRevision: UInt64?
    let payload: AlertRuntimeCommandPayload
    init(
        id: UUID = UUID(), createdAt: Date = Date(), expectedRevision: UInt64? = nil,
        payload: AlertRuntimeCommandPayload
    ) {
        self.id = id
        self.createdAt = createdAt
        self.expectedRevision = expectedRevision
        self.payload = payload
    }
}

struct MarketQuote: Codable, Equatable, Sendable {
    let asset: PortfolioAsset
    let price: Decimal
    let currency: PortfolioCurrency
    let sourceTimestamp: Date
    let receivedAt: Date
    let maximumAge: TimeInterval
    let fingerprint: String
    /// The candle behind `price`, when the provider gave one.
    var candle: AlertCandleSnapshot?

    var isFresh: Bool { receivedAt.timeIntervalSince(sourceTimestamp) <= maximumAge }
}

extension PortfolioCurrency {
    static var alertCurrencies: [PortfolioCurrency] { [.USD, .EUR, .GBP, .JPY, .CHF] }
}
