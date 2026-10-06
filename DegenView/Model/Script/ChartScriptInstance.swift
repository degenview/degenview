import Foundation

struct ChartScriptInstance: Codable, Equatable, Hashable, Identifiable, Sendable {
    enum UpdateStatus: String, Codable, Sendable { case current, available, missing }
    var id: UUID
    var scriptID: UUID
    var loadedRevisionID: UUID
    var inputs: [String: PineInputValue]
    var isVisible: Bool
    var styleOverrides: [String: String]
    var updateStatus: UpdateStatus
    /// Only set when migrated from the pre-instance `TickerConfig.pine` shape, whose source
    /// text was never registered with `ScriptStore`. New instances always resolve source
    /// through `ScriptStore` by `scriptID`/`loadedRevisionID`; this is strictly a
    /// legacy-decode fallback, not a general "inline script" feature.
    var legacySource: String? = nil

    init(
        id: UUID = UUID(), scriptID: UUID, loadedRevisionID: UUID,
        inputs: [String: PineInputValue] = [:], isVisible: Bool = true,
        styleOverrides: [String: String] = [:], updateStatus: UpdateStatus = .current,
        legacySource: String? = nil
    ) {
        self.id = id
        self.scriptID = scriptID
        self.loadedRevisionID = loadedRevisionID
        self.inputs = inputs
        self.isVisible = isVisible
        self.styleOverrides = styleOverrides
        self.updateStatus = updateStatus
        self.legacySource = legacySource
    }
}
