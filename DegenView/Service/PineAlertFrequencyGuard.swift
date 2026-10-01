import Foundation

/// Decides whether an alert may reach the user, per subscription, call site and bar.
///
/// The Pine session already suppresses repeats inside one host. This guard covers what a session
/// cannot see: a rebuilt host (its ledger starts empty), several subscriptions to one chart, and
/// an app restart, which `seed(_:)` covers from the persisted history.
struct PineAlertFrequencyGuard: Sendable {
    /// Oldest keys drop first, so memory stays bounded however long the app runs.
    static let capacity = 2_000

    private var seen: Set<String> = []
    private var order: [String] = []

    init(seeded keys: [String] = []) {
        seed(keys)
    }

    /// `subscription|site|barOpenMs|frequency`: one slot per call site per bar per mode.
    static func key(subscription: UUID, event: PineAlertEvent) -> String {
        let barOpenMs = Int64((event.time.timeIntervalSince1970 * 1000).rounded())
        return "\(subscription.uuidString)|\(event.site)|\(barOpenMs)|\(event.frequency.rawValue)"
    }

    /// The key that makes a persisted delivery unique, or nil for `freq_all`, which may
    /// legitimately fire many times on one bar.
    static func dedupeKey(subscription: UUID, event: PineAlertEvent) -> String? {
        event.frequency == .all ? nil : key(subscription: subscription, event: event)
    }

    /// Whether `event` may notify. Records the slot when it does.
    mutating func admit(_ event: PineAlertEvent, subscription: UUID) -> Bool {
        switch event.frequency {
        case .all:
            return true
        case .oncePerBar:
            return remember(Self.key(subscription: subscription, event: event))
        case .oncePerBarClose:
            guard event.isConfirmed, event.isRealtime else { return false }
            return remember(Self.key(subscription: subscription, event: event))
        }
    }

    mutating func seed(_ keys: [String]) {
        for key in keys { _ = remember(key) }
    }

    /// Forgets everything for one subscription, for a re-arm.
    mutating func reset(subscription: UUID) {
        let prefix = subscription.uuidString + "|"
        order.removeAll { $0.hasPrefix(prefix) }
        seen = Set(order)
    }

    private mutating func remember(_ key: String) -> Bool {
        guard seen.insert(key).inserted else { return false }
        order.append(key)
        if order.count > Self.capacity {
            let overflow = order.count - Self.capacity
            for dropped in order.prefix(overflow) { seen.remove(dropped) }
            order.removeFirst(overflow)
        }
        return true
    }
}
