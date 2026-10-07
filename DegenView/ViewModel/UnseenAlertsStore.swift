import Combine
import Foundation

/// How many alerts have fired since the user last looked at the Alerts window; drives the red
/// bubble on the sidebar bell.
///
/// "Seen" is a timestamp in the shared `setting` table, not a flag on the events: the alert
/// snapshot rewrites its event rows, and the agent can fire alerts while the app is closed.
/// Those arrive through `AlertStore.reload()` after launch and count when they postdate it.
@MainActor
final class UnseenAlertsStore: ObservableObject {
    static let shared = UnseenAlertsStore()
    static let lastSeenKey = "alerts.lastSeenAt"

    @Published private(set) var count = 0

    private let database: AppDatabase
    private let now: () -> Date
    private var lastSeenAt: Date
    private var unseenIDs: Set<UUID> = []
    private var isAlertsWindowActive = false
    private var subscription: AnyCancellable?

    /// A first run has nothing to compare against, so history from before this build is not "new".
    init(bus: AlertEventBus = .shared, database: AppDatabase = .shared, now: @escaping () -> Date = Date.init) {
        self.database = database
        self.now = now
        if let saved = database.setting(Date.self, key: Self.lastSeenKey) {
            lastSeenAt = saved
        } else {
            lastSeenAt = now()
            database.setSetting(lastSeenAt, key: Self.lastSeenKey)
        }
        subscription = bus.events.sink { [weak self] in self?.receive($0) }
    }

    /// The Alerts window became the key window: everything so far is seen, and so is what arrives while it stays.
    func windowDidBecomeActive() {
        isAlertsWindowActive = true
        markAllSeen()
    }

    func windowDidResign() {
        isAlertsWindowActive = false
    }

    private func receive(_ event: AlertDomainEvent) {
        guard event.occurredAt > lastSeenAt else { return }
        if isAlertsWindowActive {
            markAllSeen(upTo: event.occurredAt)
        } else if unseenIDs.insert(event.eventID).inserted {
            count = unseenIDs.count
        }
    }

    private func markAllSeen(upTo date: Date? = nil) {
        unseenIDs.removeAll()
        count = 0
        lastSeenAt = max(date ?? now(), lastSeenAt)
        database.setSetting(lastSeenAt, key: Self.lastSeenKey)
    }
}
