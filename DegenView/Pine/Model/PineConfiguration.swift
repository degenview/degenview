import Foundation

struct PineConfiguration: Codable, Equatable, Hashable, Sendable {
    var draftSource: String
    var appliedSource: String?
    var inputs: [String: PineInputValue]
    init(
        draftSource: String = "", appliedSource: String? = nil, inputs: [String: PineInputValue] = [:]
    ) {
        self.draftSource = draftSource
        self.appliedSource = appliedSource
        self.inputs = inputs
    }
}
