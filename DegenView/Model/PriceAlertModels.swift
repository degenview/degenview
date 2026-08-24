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
        switch self { case .crossesAbove, .risesBy: true; default: false }
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
        case "risesBy": self = .risesBy(percent: try box.decode(Decimal.self, forKey: .percent), reference: try box.decode(Decimal.self, forKey: .reference), target: try box.decode(Decimal.self, forKey: .target))
        case "fallsBy": self = .fallsBy(percent: try box.decode(Decimal.self, forKey: .percent), reference: try box.decode(Decimal.self, forKey: .reference), target: try box.decode(Decimal.self, forKey: .target))
        default: self = .unsupported(tag: tag)
        }
    }
    func encode(to encoder: Encoder) throws {
        var box = encoder.container(keyedBy: Keys.self)
        switch self {
        case .crossesAbove(let target): try box.encode("crossesAbove", forKey: .tag); try box.encode(target, forKey: .target)
        case .crossesBelow(let target): try box.encode("crossesBelow", forKey: .tag); try box.encode(target, forKey: .target)
        case .risesBy(let percent, let reference, let target): try box.encode("risesBy", forKey: .tag); try box.encode(percent, forKey: .percent); try box.encode(reference, forKey: .reference); try box.encode(target, forKey: .target)
        case .fallsBy(let percent, let reference, let target): try box.encode("fallsBy", forKey: .tag); try box.encode(percent, forKey: .percent); try box.encode(reference, forKey: .reference); try box.encode(target, forKey: .target)
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

    init(id: UUID = UUID(), asset: PortfolioAsset, condition: AlertCondition,
         currency: PortfolioCurrency = .USD, frequency: AlertFrequency = .once,
         state: AlertState = .active, note: String = "", armed: Bool = true,
         previousValue: Decimal? = nil, lastFingerprint: String? = nil,
         createdAt: Date = Date(), updatedAt: Date = Date()) {
        self.id = id; self.asset = asset; self.condition = condition; self.currency = currency
        self.frequency = frequency; self.state = state; self.note = note; self.armed = armed
        self.previousValue = previousValue; self.lastFingerprint = lastFingerprint
        self.createdAt = createdAt; self.updatedAt = updatedAt
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
}

struct AlertNotificationSettings: Codable, Equatable, Sendable {
    var deliveryEnabled = true
    var soundEnabled = true
    var inAppBannersEnabled = true
    var macOSNotificationsEnabled = true
}

struct AlertPersistenceSnapshot: Codable, Equatable, Sendable {
    static let currentSchemaVersion = 1
    var schemaVersion = currentSchemaVersion
    var alerts: [PriceAlert] = []
    var history: [AlertTriggerEvent] = []
    var settings = AlertNotificationSettings()
}

struct MarketQuote: Codable, Equatable, Sendable {
    let asset: PortfolioAsset
    let price: Decimal
    let currency: PortfolioCurrency
    let sourceTimestamp: Date
    let receivedAt: Date
    let maximumAge: TimeInterval
    let fingerprint: String

    var isFresh: Bool { receivedAt.timeIntervalSince(sourceTimestamp) <= maximumAge }
}

extension PortfolioCurrency {
    static var alertCurrencies: [PortfolioCurrency] { [.USD, .EUR, .GBP, .JPY, .CHF] }
}
