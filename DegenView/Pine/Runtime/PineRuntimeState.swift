import Foundation

/// Everything a script run accumulates. The session keeps a `committed` copy and a `working`
/// copy: a realtime tick runs on `working` and is thrown away unless the bar confirms.
struct PineRuntimeState {
    var variables: [String: PineRuntimeValue] = [:]
    var histories: [String: [PineRuntimeValue]] = [:]
    /// This bar's value of every `expression[n]` whose base is not a plain name, by history key.
    /// Folded into `histories` when the bar commits.
    var expressionValues: [String: PineRuntimeValue] = [:]
    var calls: [Int: [PineRuntimeValue]] = [:]
    var callInputs: [Int: [PineRuntimeValue]] = [:]
    var plots: [Int: PinePlotOutput] = [:]
    var hlines: [Int: PineHorizontalLine] = [:]
    var markers: [Int: PineMarkerOutput] = [:]
    var backgrounds: [Int: PineColorOutput] = [:]
    var barColors: [Int: PineColorOutput] = [:]
    var fills: [Int: PineFillOutput] = [:]
    // Reference-typed objects live in the state so realtime rollback restores them
    // together with the variables that point at them.
    var arrays: [Int: [PineRuntimeValue]] = [:]
    var maps: [Int: PineMap] = [:]
    /// Instances of script-defined types, by handle.
    var instances: [Int: PineObject] = [:]
    /// `request.security` call sites, by call-site key. Each holds a state of its own, so this
    /// type contains itself through the dictionary.
    var securities: [Int: PineSecuritySite] = [:]
    var lines: [Int: PineLineOutput] = [:]
    var labels: [Int: PineLabelOutput] = [:]
    var boxes: [Int: PineBoxOutput] = [:]
    var linefills: [Int: PineLinefillOutput] = [:]
    var tables: [Int: PineTableOutput] = [:]
    var candles: [Int: PineCandleOutput] = [:]
    var alerts: [PineAlertEvent] = []
    var broker = PineBrokerEmulator()
    var nextReference = 0
    var barIndex = -1
    var instructions = 0
}
