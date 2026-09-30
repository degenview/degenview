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

    init(
        id: UUID = UUID(), scriptID: UUID, loadedRevisionID: UUID,
        inputs: [String: PineInputValue] = [:], isVisible: Bool = true,
        styleOverrides: [String: String] = [:], updateStatus: UpdateStatus = .current
    ) {
        self.id = id
        self.scriptID = scriptID
        self.loadedRevisionID = loadedRevisionID
        self.inputs = inputs
        self.isVisible = isVisible
        self.styleOverrides = styleOverrides
        self.updateStatus = updateStatus
    }
}
