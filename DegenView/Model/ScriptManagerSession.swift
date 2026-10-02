import Foundation

/// Whether the Script Manager was open at quit, and where, so a relaunch can put it back.
///
/// The manager is a fixed window scene rather than a `ChartTab`, so `TabsStore` never sees it.
/// This is UI state, kept beside the other `scriptManager.*` defaults.
struct ScriptManagerSession: Codable, Equatable {
    var isOpen: Bool
    /// A chart tab that shared its tab group, so the manager rejoins that group.
    var anchorTabID: UUID?
    /// It was the selected tab of its group.
    var wasSelected: Bool

    static let defaultsKey = "scriptManager.session"

    static func load(from defaults: UserDefaults = .standard) -> ScriptManagerSession? {
        defaults.data(forKey: defaultsKey).flatMap { try? JSONDecoder().decode(Self.self, from: $0) }
    }

    func save(to defaults: UserDefaults = .standard) {
        defaults.set(try? JSONEncoder().encode(self), forKey: Self.defaultsKey)
    }
}
