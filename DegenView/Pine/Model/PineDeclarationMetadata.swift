import Foundation

struct PineDeclarationMetadata: Codable, Equatable, Sendable {
    var type: ScriptType = .indicator
    var pineVersion: Int? = nil
    var title: String
    var shortTitle: String?
    var overlay: Bool
    var format: String?
    var precision: Int?
    var maxBarsBack: Int?
    var maxLinesCount: Int? = nil
    var maxLabelsCount: Int? = nil
    var maxBoxesCount: Int? = nil
    /// Present for `strategy()` scripts.
    var strategy: PineStrategySettings? = nil
}
