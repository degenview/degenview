import Foundation

/// One applied script's display data, derived fresh from `scriptInstances`/`pineResults` — never
/// stored, so it cannot drift from the model.
struct PineLegendRowInfo: Identifiable {
    let id: UUID  // == ChartScriptInstance.id
    let scriptID: UUID  // the library script this instance runs
    let title: String  // the declaration title; inputs live in the settings popover
    let isVisible: Bool
    let hasError: Bool
}

extension ChartViewModel {
    var pineLegendRows: [PineLegendRowInfo] {
        scriptInstances.map { instance in
            let result = pineResults[instance.id]
            let title = result?.declaration?.title ?? "Script"
            let hasError = result?.diagnostics.contains { $0.severity == .error } ?? false
            return PineLegendRowInfo(
                id: instance.id, scriptID: instance.scriptID, title: title, isVisible: instance.isVisible,
                hasError: hasError)
        }
    }
}
