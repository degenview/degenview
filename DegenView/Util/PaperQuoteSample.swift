import Foundation

/// What a chart tells the paper-trading engine about its market at one moment: the last price, and
/// the best bid and ask when the exchange stream supplied them recently.
///
/// Equatable on purpose — the quote feed re-reads it on every chart change and acts only when it
/// differs, so `refreshedAt` is part of it: a refresh with an unchanged price still counts as news.
struct PaperQuoteSample: Equatable {
    let last: Double?
    let bid: Double?
    let ask: Double?
    let refreshedAt: Date?
    let isConnected: Bool

    /// A bid and ask older than this are not shown or traded against. The stream pushes on every
    /// book change, so a quiet gap this long means it stopped, and a stale spread beside a fresh
    /// last price would be worse than none.
    static let bookMaximumAge: TimeInterval = 15

    static func make(
        last: Double?, bid: Double?, ask: Double?, bookUpdatedAt: Date?, refreshedAt: Date?,
        isConnected: Bool, now: Date = Date()
    ) -> PaperQuoteSample {
        let isFresh = bookUpdatedAt.map { now.timeIntervalSince($0) <= bookMaximumAge } ?? false
        let isSane = (bid ?? 0) > 0 && (ask ?? 0) >= (bid ?? 0)
        let usesBook = isFresh && isSane
        return PaperQuoteSample(
            last: last, bid: usesBook ? bid : nil, ask: usesBook ? ask : nil,
            refreshedAt: refreshedAt, isConnected: isConnected)
    }

    /// A price as the engine stores it. Goes through the shortest decimal string a `Double` prints,
    /// so `84080.22` stays `84080.22` rather than picking up binary noise such as `84080.2200000000012`.
    static func decimal(_ value: Double) -> Decimal {
        Decimal(string: "\(value)", locale: Locale(identifier: "en_US_POSIX")) ?? Decimal(value)
    }
}
