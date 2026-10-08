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
    var maxPolylinesCount: Int? = nil
    /// `behind_chart`: an overlay script's plots and drawings sit behind the candles (the default) or in
    /// front. Optional so records written before it existed still decode.
    var behindChart: Bool? = nil
    /// Present for `strategy()` scripts.
    var strategy: PineStrategySettings? = nil
}
