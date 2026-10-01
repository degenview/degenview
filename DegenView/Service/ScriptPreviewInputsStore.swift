import Foundation

/// The input values a user tried in the Script Manager preview, per script.
///
/// A small cache beside the scripts, not part of them: nothing here is applied to a chart.
@MainActor
final class ScriptPreviewInputsStore {
    static let shared = ScriptPreviewInputsStore()

    private let store: JSONStore<[String: [String: PineInputValue]]>
    private var entries: [String: [String: PineInputValue]]

    init(store: JSONStore<[String: [String: PineInputValue]]> = JSONStore(filename: "script_preview_inputs.json")) {
        self.store = store
        entries = store.load() ?? [:]
    }

    func inputs(for scriptID: UUID) -> [String: PineInputValue] {
        entries[scriptID.uuidString] ?? [:]
    }

    /// An empty map forgets the script, so defaults are what the next session starts from.
    func setInputs(_ inputs: [String: PineInputValue], for scriptID: UUID) {
        let key = scriptID.uuidString
        guard entries[key] != (inputs.isEmpty ? nil : inputs) else { return }
        entries[key] = inputs.isEmpty ? nil : inputs
        store.save(entries)
    }

    /// Drops the values of scripts that no longer exist.
    func prune(keeping scriptIDs: Set<UUID>) {
        let keep = Set(scriptIDs.map(\.uuidString))
        let pruned = entries.filter { keep.contains($0.key) }
        guard pruned.count != entries.count else { return }
        entries = pruned
        store.save(entries)
    }
}
